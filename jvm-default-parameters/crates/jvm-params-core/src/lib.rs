//! Host-independent logic behind the OpenIDEA "JVM Default Parameters" plugin.
//!
//! The plugin itself is a thin wrapper around this crate: everything that needs
//! unit tests (parsing settings, splitting and merging command lines, reading
//! and patching JetBrains and VS Code run configurations, detecting Maven and
//! Gradle projects, building the launch plan) lives here, where it compiles and
//! tests with only `serde_json` and no network or OpenIDEA SDK.
//!
//! The chain of a single run is:
//!
//! ```text
//! settings ─┐
//!           ├─► Defaults ─► merge_args(existing, defaults) ─► plan::generate ─► run configuration
//! project ──┘                                                └─► patch::apply ─► *.run.xml / launch.json
//! ```

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod detect;
pub mod jetbrains;
pub mod jsonc;
pub mod merge;
pub mod patch;
pub mod plan;
pub mod project_settings;
pub mod run_toml;
pub mod settings;
pub mod shell;
pub mod springboot;
pub mod vscode;

pub use merge::Mode;
pub use settings::{Defaults, SettingsSource};

/// Version of the parameter model, shown in previews and in the plugin's README.
pub const MODEL_VERSION: &str = "1.0";

/// The `process:` capabilities the contributed run configurations need.
///
/// A launch configuration is matched against an approved program by the exact
/// `program` string in the record, and only strings declared in the plugin
/// manifest can ever be approved, so the planned programs must stay inside this
/// list. `plugin.toml` declares the same set under `[capabilities].required`.
pub const PROGRAM_CAPABILITIES: &[&str] = &[
    "java",
    "mvnw",
    "mvnw.cmd",
    "gradlew",
    "gradlew.bat",
    "mvn",
    "gradle",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{DetectOptions, Files, MapReader};
    use crate::plan::{generate, resolve_platform};

    #[test]
    fn planned_programs_are_declared_capabilities() {
        let files = Files {
            poms: vec!["pom.xml".into(), "app/pom.xml".into()],
            gradle_builds: vec!["gradle/build.gradle".into()],
            wrappers: vec![
                "mvnw".into(),
                "mvnw.cmd".into(),
                "gradlew".into(),
                "gradlew.bat".into(),
            ],
            jars: vec!["app/target/app-1.0.jar".into(), "gradle/build/libs/g-1.0.jar".into()],
        };
        let reader = MapReader::new()
            .with("pom.xml", "<artifactId>a</artifactId>spring-boot-maven-plugin")
            .with("app/pom.xml", "<artifactId>a</artifactId>spring-boot-maven-plugin")
            .with("gradle/build.gradle", "id 'org.springframework.boot' version '3.3.0'");
        let project = crate::detect::detect_with_options(
            "/p",
            &files,
            &reader,
            &DetectOptions::default(),
        );
        let mut defaults = crate::settings::Defaults::schema_defaults();
        defaults.vm_options = vec!["-Xmx1g".to_string()];
        defaults.provider.emit_all_strategies = true;
        for platform in [plan::Platform::Unix, plan::Platform::Windows] {
            for config in generate(&project, &defaults, platform) {
                assert!(
                    PROGRAM_CAPABILITIES.contains(&config.program.as_str()),
                    "{} is not a declared process capability",
                    config.program
                );
            }
        }
        assert_eq!(resolve_platform("/p", &defaults), plan::Platform::Unix);
    }
}
