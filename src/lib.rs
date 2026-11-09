//! OpenIDEA guest plugin "JVM Default Parameters".
//!
//! Two features, both fed by the same settings and the same planner:
//!
//! 1. A `run-configurations` provider. For every trusted project that contains a
//!    `pom.xml`, `build.gradle` or `build.gradle.kts`, it lists launch
//!    configurations built from the user's VM options and program arguments:
//!    `java -jar <boot jar>` when the jar is visible, otherwise the Maven
//!    `spring-boot:run` or the Gradle `bootRun` wrapper command.
//! 2. `jvm-params.apply-run-files`, which patches the VM options of the run
//!    configuration files the IDE already reads (`*.run.xml`, `.vscode/launch.json`)
//!    in place — the equivalent of IntelliJ's "JVM Default Options" for the
//!    configurations the IDE owns itself.
//!
//! Everything that can be unit tested lives in `jvm_params_core`; this crate is
//! only the adapter to the SDK: settings in, `PluginRunConfiguration`s or patched
//! files out.

use jvm_params_core::detect::{self, DetectOptions, Files, Reader};
use jvm_params_core::patch;
use jvm_params_core::plan::{self, GeneratedConfig};
use jvm_params_core::settings::{self, Defaults, SettingsSource};
use openidea_plugin::{
    host, register_plugin, Context, Error, Invocation, NotificationLevel, Plugin, PluginRunConfiguration,
    Project, RunConfigurations,
};

/// Output channel the preview and the patch report are written to.
const OUTPUT: &str = "JVM Default Parameters";
const PREVIEW_COMMAND: &str = "jvm-params.preview";
const APPLY_COMMAND: &str = "jvm-params.apply-run-files";

/// The plugin instance. It keeps no state: settings, detection and planning are
/// cheap enough to redo for every refresh, and re-reading them is what makes a
/// settings change take effect on the next Run-dialog listing.
#[derive(Default)]
pub struct JvmParams;

impl Plugin for JvmParams {
    fn activate(&mut self, _ctx: &Context) -> Result<(), Error> {
        let defaults = load_defaults();
        let mut report = format!(
            "openidea-plugin-jvm-params (model {})\n{}\n",
            jvm_params_core::MODEL_VERSION,
            defaults.summary()
        );
        for warning in &defaults.warnings {
            report.push_str(&format!("warning: {warning}\n"));
        }
        let _ = host::output_append(OUTPUT, &report);
        Ok(())
    }

    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            PREVIEW_COMMAND => Ok(Some(preview(ctx)?)),
            APPLY_COMMAND => Ok(Some(apply_run_files(ctx)?)),
            other => Err(format!("unknown command {other}").into()),
        }
    }
}

impl RunConfigurations for JvmParams {
    fn configurations(&mut self, project_root: &str) -> Vec<PluginRunConfiguration> {
        let defaults = load_defaults();
        if !defaults.enabled || !defaults.provider.enabled || !defaults.has_parameters() {
            return Vec::new();
        }
        let project = match open_project(project_root) {
            Some(project) => project,
            None => return Vec::new(),
        };
        let scan = scan_project(&project, project_root, &defaults);
        let platform = plan::resolve_platform(project_root, &defaults);
        plan::generate(&scan, &defaults, platform)
            .into_iter()
            .map(to_configuration)
            .collect()
    }
}

/// The preview: everything the planner would contribute, one command line each.
fn preview(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    let scan = scan_project(&project, &root, &defaults);
    let platform = plan::resolve_platform(&root, &defaults);
    let text = plan::preview(&scan, &defaults, platform);
    let _ = host::output_append(OUTPUT, &text);
    let _ = host::output_append(OUTPUT, "\n");
    let _ = host::output_reveal(OUTPUT);
    let count = plan::generate(&scan, &defaults, platform).len();
    host::notify(
        NotificationLevel::Info,
        "JVM Default Parameters",
        &format!("{count} configuration(s) planned for this project; the details are in the output tab."),
    );
    Ok(text)
}

/// Patches the VM options of every run configuration file the IDE reads.
fn apply_run_files(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    if !defaults.enabled {
        return Err("JVM Default Parameters is disabled in Settings › Plugins".into());
    }
    if !defaults.has_parameters() {
        return Err(
            "no parameters configured yet: set the VM options or the program arguments in Settings › Plugins › JVM Default Parameters"
                .into(),
        );
    }

    let mut report = String::new();
    let mut updated = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut hints: Vec<String> = Vec::new();

    for glob in patch::run_file_globs() {
        let paths = match project.find_files(glob, 64) {
            Ok(paths) => paths,
            Err(err) => {
                failures.push(format!("{glob}: {err}"));
                continue;
            }
        };
        for path in paths {
            let text = match project.read_file(&path) {
                Ok(text) => text,
                Err(err) => {
                    failures.push(format!("{path}: {err}"));
                    continue;
                }
            };
            let patched = match patch::patch_file(&path, &text, &defaults) {
                Ok(patched) => patched,
                Err(err) => {
                    failures.push(format!("{path}: {err}"));
                    continue;
                }
            };
            if patched.updated == 0 {
                continue;
            }
            match project.write_file(&path, &patched.text) {
                Ok(()) => {
                    updated += patched.updated;
                    report.push_str(&format!("patched {path} ({} configuration(s))\n", patched.updated));
                }
                Err(err) => {
                    failures.push(format!("{path}: {err}"));
                    hints.push(
                        "allow project.write (and fs.write for the path) for this plugin in Settings › Plugins to patch files"
                            .to_string(),
                    );
                }
            }
        }
    }

    if updated == 0 && failures.is_empty() {
        report.push_str("nothing to patch: no .run/*.run.xml, .idea/runConfigurations/*.xml or .vscode/launch.json found\n");
    }
    for failure in &failures {
        report.push_str(&format!("failed: {failure}\n"));
    }
    for hint in &hints {
        report.push_str(&format!("hint: {hint}\n"));
    }
    let _ = host::output_append(OUTPUT, &report);
    let _ = host::output_reveal(OUTPUT);

    if updated == 0 && !failures.is_empty() {
        return Err(format!(
            "could not patch any run configuration file ({} failure(s)); see the output tab",
            failures.len()
        )
        .into());
    }
    host::notify(
        NotificationLevel::Info,
        "JVM Default Parameters",
        &format!("{updated} configuration(s) patched in {root}"),
    );
    Ok(report)
}

fn load_defaults() -> Defaults {
    settings::load(&HostSettings)
}

/// `host::setting` as JSON text, exactly what [`settings::load`] expects.
struct HostSettings;

impl SettingsSource for HostSettings {
    fn setting(&self, key: &str) -> Option<String> {
        host::setting(key)
    }
}

/// The open project whose root is `project_root`, compared leniently because the
/// host may pass `D:/x` where the project reports `D:\x`.
fn open_project(project_root: &str) -> Option<Project> {
    let wanted = normalize(project_root);
    let projects = host::projects().ok()?;
    projects
        .into_iter()
        .find(|project| project.root().map(|root| normalize(&root) == wanted).unwrap_or(false))
}

fn normalize(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    trimmed.to_ascii_lowercase()
}

/// Scans the project: build files, wrappers and jars through the project API,
/// build file contents through [`Reader`].
fn scan_project(project: &Project, root: &str, defaults: &Defaults) -> detect::Project {
    let files = collect_files(project, defaults);
    let reader = ProjectReader { project };
    let options = DetectOptions::with_extra_dirs(&defaults.jar_dirs);
    detect::detect_with_options(root, &files, &reader, &options)
}

fn collect_files(project: &Project, defaults: &Defaults) -> Files {
    let mut files = Files::default();
    files.poms = find(project, "**/pom.xml", 64);
    files.gradle_builds = find(project, "**/build.gradle", 64);
    files.gradle_builds.extend(find(project, "**/build.gradle.kts", 64));
    for pattern in ["**/mvnw", "**/mvnw.cmd", "**/gradlew", "**/gradlew.bat"] {
        files.wrappers.extend(find(project, pattern, 8));
    }
    if defaults.provider.boot_jar {
        files.jars = find(project, "**/*.jar", 128);
    }
    files
}

fn find(project: &Project, glob: &str, limit: u32) -> Vec<String> {
    project.find_files(glob, limit).unwrap_or_default()
}

/// Reads build file contents from the project (unsaved editor contents win).
struct ProjectReader<'a> {
    project: &'a Project,
}

impl Reader for ProjectReader<'_> {
    fn read(&self, relative_path: &str) -> Option<String> {
        self.project.read_file(relative_path).ok()
    }
}

fn to_configuration(config: GeneratedConfig) -> PluginRunConfiguration {
    PluginRunConfiguration {
        id: config.id,
        name: config.name,
        program: config.program,
        args: config.args,
        cwd: config.cwd,
        test: config.test,
        framework: config.framework,
    }
}

register_plugin!(JvmParams, run_configurations);

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn normalizes_separators_case_and_trailing_slash() {
        assert_eq!(normalize("D:\\work\\app\\"), "d:/work/app");
        assert_eq!(normalize("d:/work/app"), "d:/work/app");
        assert_eq!(normalize("/home/me/app/"), "/home/me/app");
    }
}
