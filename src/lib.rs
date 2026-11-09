//! OpenIDEA guest plugin "JVM Default Parameters" — plugin API **1.3** adapter.
//!
//! Three commands, all fed by the same settings and the same planner (which live
//! in `jvm_params_core`, and are unit tested without the SDK):
//!
//! * `jvm-params.preview` — prints every launch the planner would produce for the
//!   open project, one command line each.
//! * `jvm-params.apply-run-files` — patches the VM options (and, when program
//!   arguments are configured, the program arguments) of the run configuration
//!   files the IDE already reads: `.run/*.run.xml`,
//!   `.idea/runConfigurations/*.xml` and `.vscode/launch.json`. This is the
//!   equivalent of IntelliJ's "JVM Default Options" for the configurations the
//!   IDE owns itself.
//! * `jvm-params.run` — starts a planned launch in an IDE terminal, so the
//!   parameters are used without touching any file.
//!
//! Why no `runConfigurations` provider: that contribution point needs plugin API
//! 1.4, and the released IDE (v0.1.39) still reports plugin API **1.3.0** — a
//! manifest with `engines.openidea = "^1.4"` is rejected at install time. The
//! provider code is kept in [`provider`] behind the `api14` cargo feature so it
//! can be switched on once a 1.4 host ships: see `Cargo.toml` for the two changes
//! that takes.

use jvm_params_core::detect::{self, DetectOptions, Files, Reader};
use jvm_params_core::patch;
use jvm_params_core::plan;
use jvm_params_core::settings::{self, Defaults, SettingsSource};
use openidea_plugin::{
    host, register_plugin, Context, Error, Invocation, NotificationLevel, PickItem, Plugin, Project,
};

#[cfg(feature = "api14")]
mod provider;

const PREVIEW_COMMAND: &str = "jvm-params.preview";
const APPLY_COMMAND: &str = "jvm-params.apply-run-files";
const RUN_COMMAND: &str = "jvm-params.run";

const TITLE: &str = "JVM Default Parameters";

/// The plugin instance. It keeps no state: settings, detection and planning are
/// cheap enough to redo per command, and re-reading the settings is what makes a
/// change take effect immediately.
#[derive(Default)]
pub struct JvmParams;

impl Plugin for JvmParams {
    fn activate(&mut self, _ctx: &Context) -> Result<(), Error> {
        let defaults = load_defaults();
        host::log(
            openidea_plugin::LogLevel::Info,
            &format!("jvm-default-parameters ready: {}", defaults.summary()),
        );
        for warning in &defaults.warnings {
            host::log(openidea_plugin::LogLevel::Warn, &format!("jvm-default-parameters: {warning}"));
        }
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
            RUN_COMMAND => Ok(Some(run_planned(ctx)?)),
            other => Err(format!("unknown command {other}").into()),
        }
    }
}

/// Everything the planner would produce for the open project.
fn preview(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    let scan = scan_project(&project, &root, &defaults);
    let platform = plan::resolve_platform(&root, &defaults);
    let text = plan::preview(&scan, &defaults, platform);
    let count = plan::generate(&scan, &defaults, platform).len();
    host::notify(
        NotificationLevel::Info,
        TITLE,
        &format!("{count} configuration(s) planned for this project"),
    );
    Ok(text)
}

/// Patches the VM options of every run configuration file the IDE reads.
fn apply_run_files(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    require_parameters(&defaults)?;

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
                    report.push_str(&format!(
                        "patched {path} ({} configuration(s))\n",
                        patched.updated
                    ));
                }
                Err(err) => {
                    failures.push(format!("{path}: {err}"));
                    hints.push(
                        "allow the optional `project.write` capability for this plugin in Settings › Plugins to patch files"
                            .to_string(),
                    );
                }
            }
        }
    }

    if updated == 0 && failures.is_empty() {
        report.push_str(
            "nothing to patch: no .run/*.run.xml, .idea/runConfigurations/*.xml or .vscode/launch.json found\n",
        );
    }
    for failure in &failures {
        report.push_str(&format!("failed: {failure}\n"));
    }
    for hint in &hints {
        report.push_str(&format!("hint: {hint}\n"));
    }

    if updated == 0 && !failures.is_empty() {
        host::notify(
            NotificationLevel::Warning,
            TITLE,
            &format!("could not patch any run configuration file ({} failure(s))", failures.len()),
        );
        return Ok(report);
    }
    host::notify(
        NotificationLevel::Info,
        TITLE,
        &format!("{updated} configuration(s) patched in {root}"),
    );
    Ok(report)
}

/// Starts a planned launch in an IDE terminal (no files touched).
fn run_planned(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    require_parameters(&defaults)?;

    let scan = scan_project(&project, &root, &defaults);
    let platform = plan::resolve_platform(&root, &defaults);
    let configs = plan::generate(&scan, &defaults, platform);
    if configs.is_empty() {
        return Err(
            "nothing to run: enable the strategies you need in Settings › Plugins › JVM Default Parameters, or check that this project has a pom.xml / build.gradle with Spring Boot"
                .into(),
        );
    }

    let chosen = if configs.len() == 1 {
        configs.into_iter().next().expect("length checked")
    } else {
        let items: Vec<PickItem> = configs
            .iter()
            .map(|config| PickItem::new(&config.id, &config.name).description(&config.argv_line()))
            .collect();
        let key = match host::pick("Run with JVM Default Parameters", &items)? {
            Some(key) => key,
            None => return Err(Error::Cancelled),
        };
        configs
            .into_iter()
            .find(|config| config.id == key)
            .ok_or("the chosen configuration disappeared")?
    };

    // `open_terminal` always runs in the project root, so the module directory of
    // a `bootJar` launch is only used in the preview (the jar path is absolute).
    if let Err(err) = host::open_terminal(&chosen.name, &chosen.program, &chosen.args) {
        return Err(Error::Failed(format!(
            "could not start `{}` ({err}). Allow the optional `process:{}` capability for this project in Settings › Plugins, then try again.",
            chosen.program, chosen.program
        )));
    }
    Ok(format!("started `{}` in a terminal:\n{}\n", chosen.name, chosen.argv_line()))
}

fn require_parameters(defaults: &Defaults) -> Result<(), Error> {
    if !defaults.enabled {
        return Err("JVM Default Parameters is disabled in Settings › Plugins".into());
    }
    if !defaults.has_parameters() {
        return Err(
            "no parameters configured yet: set the VM options or the program arguments in Settings › Plugins › JVM Default Parameters"
                .into(),
        );
    }
    Ok(())
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

// Plugin API 1.3: no optional exports. With `--features api14` (and an SDK built
// with `run.wit`) the provider is registered instead — see `provider.rs`.
#[cfg(feature = "api14")]
register_plugin!(JvmParams, run_configurations);
#[cfg(not(feature = "api14"))]
register_plugin!(JvmParams);
