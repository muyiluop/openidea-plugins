//! `runConfigurations` provider — **only for plugin API 1.4 and later**.
//!
//! Compiled only with `--features api14`, against an SDK that ships `wit/run.wit`
//! (upstream `main` from `4bc08a1…` onwards; the released `v0.1.39` SDK does
//! not). With the feature on, the planner's launches show up in the IDE's Run
//! dialog as contributed configurations instead of only in the terminal; every
//! `program` is one of the bare tokens declared in `[capabilities]`, which is
//! what the host compares against `process:<program>` grants.

use jvm_params_core::detect::{DetectOptions, Files, Reader};
use jvm_params_core::plan;
use jvm_params_core::settings::{self, Defaults, SettingsSource};
use openidea_plugin::{host, PluginRunConfiguration, Project, RunConfigurations};

use crate::JvmParams;

impl RunConfigurations for JvmParams {
    fn configurations(&mut self, project_root: &str) -> Vec<PluginRunConfiguration> {
        let defaults = settings::load(&HostSettings);
        if !defaults.enabled || !defaults.has_parameters() {
            return Vec::new();
        }
        let Some(project) = open_project(project_root) else {
            return Vec::new();
        };
        let files = collect_files(&project, &defaults);
        let reader = ProjectReader { project: &project };
        let options = DetectOptions::with_extra_dirs(&defaults.jar_dirs);
        let scanned = jvm_params_core::detect::detect_with_options(project_root, &files, &reader, &options);
        let platform = plan::resolve_platform(project_root, &defaults);
        plan::generate(&scanned, &defaults, platform)
            .into_iter()
            .map(to_configuration)
            .collect()
    }
}

fn to_configuration(config: plan::GeneratedConfig) -> PluginRunConfiguration {
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

/// The host hands the configurations provider a root path; `host::projects()`
/// is what turns it into a `Project` handle.
fn open_project(root: &str) -> Option<Project> {
    let wanted = normalize(root);
    let projects = host::projects().ok()?;
    projects
        .into_iter()
        .find(|project| project.root().map(|candidate| normalize(&candidate) == wanted).unwrap_or(false))
}

fn normalize(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let lowered = trimmed.replace('/', "\\").to_lowercase();
    if lowered.is_empty() {
        "\\".to_string()
    } else {
        lowered
    }
}

/// `host::setting` as JSON text, exactly what [`settings::load`] expects.
struct HostSettings;

impl SettingsSource for HostSettings {
    fn setting(&self, key: &str) -> Option<String> {
        host::setting(key)
    }
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

struct ProjectReader<'a> {
    project: &'a Project,
}

impl Reader for ProjectReader<'_> {
    fn read(&self, relative_path: &str) -> Option<String> {
        self.project.read_file(relative_path).ok()
    }
}
