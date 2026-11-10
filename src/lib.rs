//! OpenIDEA guest plugin "JVM Default Parameters" — plugin API **1.3** adapter.
//!
//! What it does, in one sentence: it appends your JVM options (`-Xmx…`, `-D…`)
//! and program arguments to **every launch configuration the IDE shows for the
//! project**, without editing any of them by hand.
//!
//! How: the Spring Boot configurations in the run dialog are *generated* by the
//! IDE on every listing (`crates/ide-run/src/detect.rs`) and carry no VM options
//! at all. A configuration saved in `.openidea/run.toml` with the same id
//! replaces the generated one (`crates/ide-host/src/ops/run.rs`, "saved wins"),
//! so `jvm-params.apply-run-files` writes one table per detected application
//! into `.openidea/run.toml`, using the same `java … -cp ${classpath} <main>`
//! command line the IDE's own JetBrains importer writes.
//!
//! The same command also patches the launch configuration *files* the IDE reads
//! (`.idea/workspace.xml`, `.run/*.run.xml`, `.idea/runConfigurations/*.xml`,
//! `.vscode/launch.json`), so configurations that came from IntelliJ or from
//! VS Code get the parameters too. `.idea/` is normally git-ignored, hence the
//! workspace file is addressed by its literal path.
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
use jvm_params_core::run_toml;
use jvm_params_core::settings::{self, Defaults, SettingsSource};
use jvm_params_core::springboot;
use openidea_plugin::{
    host, register_plugin, Context, Error, Invocation, NotificationLevel, Plugin, Project,
};

#[cfg(feature = "api14")]
mod provider;

const PREVIEW_COMMAND: &str = "jvm-params.preview";
const APPLY_COMMAND: &str = "jvm-params.apply-run-files";

const TITLE: &str = "JVM Default Parameters";

/// How many source files the application scan may read.
const SOURCE_LIMIT: u32 = 4000;

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
            APPLY_COMMAND => Ok(Some(apply(ctx)?)),
            other => Err(format!("unknown command {other}").into()),
        }
    }
}

/// What would be written, without writing anything.
fn preview(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    let apps = applications(&project, &root, &defaults);
    let platform = plan::resolve_platform(&root, &defaults);
    let scan = scan_project(&project, &root, &defaults);
    let mut text = plan::preview(&scan, &defaults, platform);
    text.push_str(&format!(
        "\n.openidea/run.toml: {} Spring Boot application(s) would be saved\n",
        apps.len()
    ));
    for app in apps.iter().take(40) {
        text.push_str(&format!("  {} — {}\n", app.id, app.name));
    }
    if apps.len() > 40 {
        text.push_str(&format!("  … and {} more\n", apps.len() - 40));
    }
    host::notify(
        NotificationLevel::Info,
        TITLE,
        &format!("{} application(s) would be saved in .openidea/run.toml", apps.len()),
    );
    Ok(text)
}

/// Saves the configurations and patches the configuration files.
fn apply(ctx: &Invocation) -> Result<String, Error> {
    let project = ctx.project().ok_or("open a project first")?;
    let root = project.root()?;
    let defaults = load_defaults();
    require_parameters(&defaults)?;

    let mut report = String::new();
    let mut hints: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    // 1. The configurations the run dialog lists.
    let apps = applications(&project, &root, &defaults);
    let saved = match save_run_toml(&project, &apps, &defaults) {
        Ok(saved) => saved,
        Err(err) => {
            failures.push(format!("{}: {err}", run_toml::RUN_TOML));
            0
        }
    };
    report.push_str(&format!(
        "{}: {} application(s) found, {saved} configuration(s) written\n",
        run_toml::RUN_TOML,
        apps.len()
    ));

    // 2. The configuration files the IDE reads, for configurations that came
    //    from IntelliJ or VS Code.
    let mut updated = 0usize;
    // The IDE's own detected configurations are stored in `.idea/workspace.xml`,
    // which `find_files` skips because `.idea/` is usually ignored — so it is
    // patched by its literal path.
    let mut candidates: Vec<String> = Vec::new();
    for literal in patch::literal_run_files() {
        if project.exists(literal).unwrap_or(false) {
            candidates.push(literal.to_string());
        }
    }
    for glob in patch::run_file_globs() {
        match project.find_files(glob, 64) {
            Ok(paths) => candidates.extend(paths),
            Err(err) => failures.push(format!("{glob}: {err}")),
        }
    }
    candidates.sort();
    candidates.dedup();

    for path in candidates {
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
                    "allow the optional `project.write` capability for this plugin in Settings › Plugins to write files"
                        .to_string(),
                );
            }
        }
    }

    if saved > 0 {
        report.push_str(
            "\nnext: open the run dialog — the configurations are listed right away; restart the IDE if an old one is still cached\n",
        );
    }
    for failure in &failures {
        report.push_str(&format!("failed: {failure}\n"));
    }
    for hint in &hints {
        report.push_str(&format!("hint: {hint}\n"));
    }

    if saved == 0 && updated == 0 && !failures.is_empty() {
        host::notify(
            NotificationLevel::Warning,
            TITLE,
            &format!("could not write any configuration ({} failure(s))", failures.len()),
        );
        return Ok(report);
    }
    host::notify(
        NotificationLevel::Info,
        TITLE,
        &format!("{saved} configuration(s) saved to {}", run_toml::RUN_TOML),
    );
    Ok(report)
}

/// Writes the detected applications into `.openidea/run.toml`, keeping the
/// tables the user wrote there. Returns how many tables changed.
fn save_run_toml(
    project: &Project,
    apps: &[springboot::App],
    defaults: &Defaults,
) -> Result<usize, Error> {
    let entries = run_toml::entries(apps, defaults);
    let existing = if project.exists(run_toml::RUN_TOML).unwrap_or(false) {
        project
            .read_file(run_toml::RUN_TOML)
            .map_err(|err| Error::Failed(format!("{}: {err}", run_toml::RUN_TOML)))?
    } else {
        String::new()
    };
    let (text, changed) = run_toml::upsert(&existing, &entries, defaults);
    if changed == 0 {
        return Ok(0);
    }
    project
        .write_file(run_toml::RUN_TOML, &text)
        .map_err(|err| Error::Failed(format!("{}: {err}", run_toml::RUN_TOML)))?;
    Ok(changed)
}

/// The Spring Boot applications the IDE would list for this project.
fn applications(project: &Project, root: &str, defaults: &Defaults) -> Vec<springboot::App> {
    let scan = scan_project(project, root, defaults);
    let sources = scan_sources(project);
    let reader = ProjectReader { project };
    springboot::apps(&scan, &sources, &reader)
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

/// The Java and Kotlin sources the application scan looks at.
fn scan_sources(project: &Project) -> Vec<String> {
    let mut sources = Vec::new();
    for glob in springboot::source_globs() {
        sources.extend(find(project, glob, SOURCE_LIMIT));
    }
    sources.sort();
    sources.dedup();
    sources
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
