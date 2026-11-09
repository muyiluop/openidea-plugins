//! Reading the plugin's settings into a [`Defaults`] value.
//!
//! Every setting is declared in `plugin.toml` / `settings.json` and read through
//! `host::setting(key)`, which returns the *effective* value — the project's
//! override, else the user's value, else the schema default — as JSON text. This
//! module owns the keys and the coercion, so the plugin wrapper only has to
//! forward the raw values and the tests can fake them.
//!
//! All keys are flat scalars on purpose: the settings schema dialect (whether
//! nested objects with descriptions are supported) is not confirmed, and flat
//! keys with `jvmParams.` prefixes render fine in any of the plausible dialects.

use crate::merge::Mode;
use crate::shell;

/// Something that can read a plugin setting as JSON text.
pub trait SettingsSource {
    fn setting(&self, key: &str) -> Option<String>;
}

pub const KEY_ENABLED: &str = "jvmParams.enabled";
pub const KEY_VM_OPTIONS: &str = "jvmParams.vmOptions";
pub const KEY_PROGRAM_ARGS: &str = "jvmParams.programArgs";
pub const KEY_MODE: &str = "jvmParams.mode";
pub const KEY_SPRING_PROFILES: &str = "jvmParams.springProfiles";
pub const KEY_JAVA_EXECUTABLE: &str = "jvmParams.javaExecutable";
pub const KEY_MAVEN_PROGRAM: &str = "jvmParams.mavenProgram";
pub const KEY_GRADLE_PROGRAM: &str = "jvmParams.gradleProgram";
pub const KEY_PLATFORM: &str = "jvmParams.platform";
pub const KEY_JAR_DIRS: &str = "jvmParams.jarSearchDirs";
pub const KEY_PROVIDER_ENABLED: &str = "jvmParams.provider.enabled";
pub const KEY_PROVIDER_SPRING_BOOT_MAVEN: &str = "jvmParams.provider.springBootMaven";
pub const KEY_PROVIDER_SPRING_BOOT_GRADLE: &str = "jvmParams.provider.springBootGradle";
pub const KEY_PROVIDER_BOOT_JAR: &str = "jvmParams.provider.bootJar";
pub const KEY_PROVIDER_EMIT_ALL: &str = "jvmParams.provider.emitAllStrategies";
pub const KEY_PROVIDER_MAX: &str = "jvmParams.provider.maxConfigurations";

/// Which generated launch configurations the provider offers in the Run dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub spring_boot_maven: bool,
    pub spring_boot_gradle: bool,
    pub boot_jar: bool,
    /// Emit every applicable strategy per module instead of the first one.
    pub emit_all_strategies: bool,
    /// Upper bound on the configurations contributed to one project.
    pub max_configurations: usize,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            spring_boot_maven: true,
            spring_boot_gradle: true,
            boot_jar: true,
            emit_all_strategies: false,
            max_configurations: 24,
        }
    }
}

/// The effective defaults for one project, with any parse warnings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Defaults {
    /// Master switch: when false nothing is injected anywhere.
    pub enabled: bool,
    /// JVM options, already split into arguments.
    pub vm_options: Vec<String>,
    /// Application (program) arguments, already split into arguments.
    pub program_args: Vec<String>,
    /// How to combine the defaults with parameters that already exist.
    pub mode: Mode,
    /// Spring profiles, applied as `-Dspring.profiles.active=…`.
    pub spring_profiles: Vec<String>,
    /// Program used to launch a jar; `java` by default.
    pub java_executable: String,
    /// Override for the Maven program (empty = use the project's wrapper).
    pub maven_program: String,
    /// Override for the Gradle program (empty = use the project's wrapper).
    pub gradle_program: String,
    /// `auto`, `windows` or `unix`; overrides the project-root heuristic.
    pub platform: String,
    /// Extra directories searched for a runnable jar, relative to the module.
    pub jar_dirs: Vec<String>,
    pub provider: ProviderSettings,
    /// Problems found while reading the settings, shown once as a warning.
    pub warnings: Vec<String>,
}

impl Defaults {
    /// The value every setting has when the user has not touched it.
    pub fn schema_defaults() -> Self {
        Self {
            enabled: true,
            vm_options: Vec::new(),
            program_args: Vec::new(),
            mode: Mode::Append,
            spring_profiles: Vec::new(),
            java_executable: "java".to_string(),
            maven_program: String::new(),
            gradle_program: String::new(),
            platform: "auto".to_string(),
            jar_dirs: Vec::new(),
            provider: ProviderSettings::default(),
            warnings: Vec::new(),
        }
    }

    /// JVM options including the Spring profile property, as they are injected.
    pub fn effective_vm_args(&self) -> Vec<String> {
        let mut args = self.vm_options.clone();
        if !self.spring_profiles.is_empty() {
            let value = self.spring_profiles.join(",");
            args.push(format!("-Dspring.profiles.active={value}"));
        }
        args
    }

    /// Whether there is anything to inject at all.
    pub fn has_parameters(&self) -> bool {
        !self.vm_options.is_empty() || !self.program_args.is_empty() || !self.spring_profiles.is_empty()
    }

    /// A one-line summary for the status bar / notifications.
    pub fn summary(&self) -> String {
        format!(
            "VM: {} · Program: {} · mode: {}",
            if self.vm_options.is_empty() {
                "—".to_string()
            } else {
                shell::join_args(&self.effective_vm_args())
            },
            if self.program_args.is_empty() {
                "—".to_string()
            } else {
                shell::join_args(&self.program_args)
            },
            self.mode.as_str()
        )
    }
}

/// Reads all settings from `source`. Never fails: bad values become warnings and
/// fall back to the schema default.
pub fn load<S: SettingsSource + ?Sized>(source: &S) -> Defaults {
    let mut warnings = Vec::new();
    let base = Defaults::schema_defaults();

    Defaults {
        enabled: read_bool(source, KEY_ENABLED, base.enabled, &mut warnings),
        vm_options: read_args(source, KEY_VM_OPTIONS, &mut warnings),
        program_args: read_args(source, KEY_PROGRAM_ARGS, &mut warnings),
        mode: read_string(source, KEY_MODE)
            .map(|s| Mode::parse(&s))
            .unwrap_or(base.mode),
        spring_profiles: read_string(source, KEY_SPRING_PROFILES)
            .map(|s| split_profiles(&s))
            .unwrap_or_default(),
        java_executable: read_string(source, KEY_JAVA_EXECUTABLE)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(base.java_executable),
        maven_program: read_string(source, KEY_MAVEN_PROGRAM)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        gradle_program: read_string(source, KEY_GRADLE_PROGRAM)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        platform: read_string(source, KEY_PLATFORM)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or(base.platform),
        jar_dirs: read_string(source, KEY_JAR_DIRS)
            .map(|s| split_list(&s))
            .unwrap_or_default(),
        provider: ProviderSettings {
            enabled: read_bool(source, KEY_PROVIDER_ENABLED, base.provider.enabled, &mut warnings),
            spring_boot_maven: read_bool(
                source,
                KEY_PROVIDER_SPRING_BOOT_MAVEN,
                base.provider.spring_boot_maven,
                &mut warnings,
            ),
            spring_boot_gradle: read_bool(
                source,
                KEY_PROVIDER_SPRING_BOOT_GRADLE,
                base.provider.spring_boot_gradle,
                &mut warnings,
            ),
            boot_jar: read_bool(
                source,
                KEY_PROVIDER_BOOT_JAR,
                base.provider.boot_jar,
                &mut warnings,
            ),
            emit_all_strategies: read_bool(
                source,
                KEY_PROVIDER_EMIT_ALL,
                base.provider.emit_all_strategies,
                &mut warnings,
            ),
            max_configurations: read_number(
                source,
                KEY_PROVIDER_MAX,
                base.provider.max_configurations,
                &mut warnings,
            ),
        },
        warnings,
    }
}

/// Splits a comma/whitespace separated profile list, dropping duplicates.
pub fn split_profiles(value: &str) -> Vec<String> {
    split_list(value)
}

/// Splits a comma-or-whitespace separated list, dropping empty entries and
/// duplicates while keeping the order.
pub fn split_list(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in value.split(|c: char| c == ',' || c.is_whitespace()) {
        let part = part.trim();
        if !part.is_empty() && !out.iter().any(|existing| existing == part) {
            out.push(part.to_string());
        }
    }
    out
}

fn raw_value<S: SettingsSource + ?Sized>(source: &S, key: &str) -> Option<String> {
    source.setting(key).map(|s| s.trim().to_string())
}

fn read_bool<S: SettingsSource + ?Sized>(
    source: &S,
    key: &str,
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(raw) = raw_value(source, key) else {
        return default;
    };
    if raw.is_empty() || raw == "null" {
        return default;
    }
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(serde_json::Value::Bool(b)) => b,
        Ok(serde_json::Value::String(s)) => {
            parse_bool_text(&s, key, default, warnings)
        }
        Ok(serde_json::Value::Number(n)) => n.as_i64().map(|v| v != 0).unwrap_or(default),
        _ => parse_bool_text(&raw, key, default, warnings),
    }
}

fn parse_bool_text(text: &str, key: &str, default: bool, warnings: &mut Vec<String>) -> bool {
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => true,
        "false" | "0" | "no" | "off" => false,
        other => {
            warnings.push(format!("{key}: '{other}' is not a boolean, using {default}"));
            default
        }
    }
}

fn read_number<S: SettingsSource + ?Sized>(
    source: &S,
    key: &str,
    default: usize,
    warnings: &mut Vec<String>,
) -> usize {
    let Some(raw) = raw_value(source, key) else {
        return default;
    };
    if raw.is_empty() || raw == "null" {
        return default;
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|value| match value {
            serde_json::Value::Number(n) => n.as_u64().map(|v| v as usize),
            serde_json::Value::String(s) => s.trim().parse::<usize>().ok(),
            _ => None,
        })
        .or_else(|| raw.trim().parse::<usize>().ok());
    match parsed {
        Some(value) if value > 0 && value <= 256 => value,
        Some(value) => {
            warnings.push(format!("{key}: {value} is out of range, using {default}"));
            default
        }
        None => {
            warnings.push(format!("{key}: '{raw}' is not a number, using {default}"));
            default
        }
    }
}

fn read_string<S: SettingsSource + ?Sized>(source: &S, key: &str) -> Option<String> {
    let raw = raw_value(source, key)?;
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(serde_json::Value::String(s)) => Some(s),
        Ok(serde_json::Value::Null) => None,
        Ok(serde_json::Value::Array(items)) => Some(
            items
                .iter()
                .map(|item| match item {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Ok(other) => Some(other.to_string()),
        Err(_) => {
            if raw.is_empty() {
                None
            } else {
                Some(raw)
            }
        }
    }
}

fn read_args<S: SettingsSource + ?Sized>(
    source: &S,
    key: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let Some(text) = read_string(source, key) else {
        return Vec::new();
    };
    if text.trim().is_empty() {
        return Vec::new();
    }
    match shell::try_split_args(&text) {
        Ok(args) => args,
        Err(err) => {
            warnings.push(format!("{key}: {err}"));
            shell::split_args(&text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Debug, Default)]
    struct Fake(BTreeMap<String, String>);

    impl Fake {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
        }
    }

    impl SettingsSource for Fake {
        fn setting(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn empty_source_yields_schema_defaults() {
        let defaults = load(&Fake::default());
        assert!(defaults.enabled);
        assert!(defaults.vm_options.is_empty());
        assert_eq!(defaults.mode, Mode::Append);
        assert_eq!(defaults.java_executable, "java");
        assert_eq!(defaults.platform, "auto");
        assert!(defaults.provider.enabled);
        assert!(defaults.provider.spring_boot_maven);
        assert!(defaults.provider.spring_boot_gradle);
        assert!(defaults.provider.boot_jar);
        assert!(!defaults.provider.emit_all_strategies);
        assert_eq!(defaults.provider.max_configurations, 24);
        assert!(defaults.warnings.is_empty());
        assert!(!defaults.has_parameters());
    }

    #[test]
    fn reads_json_string_parameters() {
        let source = Fake::new(&[
            (KEY_VM_OPTIONS, r#""-Xmx1g -Dfoo=\"a b\"""#),
            (KEY_PROGRAM_ARGS, r#""--server.port=8080 --debug""#),
        ]);
        let defaults = load(&source);
        assert_eq!(defaults.vm_options, vec!["-Xmx1g", "-Dfoo=a b"]);
        assert_eq!(defaults.program_args, vec!["--server.port=8080", "--debug"]);
        assert!(defaults.has_parameters());
    }

    #[test]
    fn reads_booleans_in_every_shape() {
        let source = Fake::new(&[
            (KEY_ENABLED, "false"),
            (KEY_PROVIDER_ENABLED, r#"true"#),
            (KEY_PROVIDER_BOOT_JAR, "\"off\""),
            (KEY_PROVIDER_SPRING_BOOT_GRADLE, "1"),
            (KEY_PROVIDER_EMIT_ALL, "\"yes\""),
        ]);
        let defaults = load(&source);
        assert!(!defaults.enabled);
        assert!(defaults.provider.enabled);
        assert!(!defaults.provider.boot_jar);
        assert!(defaults.provider.spring_boot_gradle);
        assert!(defaults.provider.emit_all_strategies);
    }

    #[test]
    fn bad_boolean_falls_back_with_warning() {
        let source = Fake::new(&[(KEY_PROVIDER_BOOT_JAR, "\"maybe\"")]);
        let defaults = load(&source);
        assert!(defaults.provider.boot_jar);
        assert_eq!(defaults.warnings.len(), 1);
        assert!(defaults.warnings[0].contains("is not a boolean"));
    }

    #[test]
    fn reads_mode_and_paths() {
        let source = Fake::new(&[
            (KEY_MODE, "\"Override\""),
            (KEY_JAVA_EXECUTABLE, "\"/opt/jdk21/bin/java\""),
            (KEY_MAVEN_PROGRAM, "\"mvn\""),
            (KEY_PLATFORM, "\"Windows\""),
            (KEY_JAR_DIRS, "\"build/jars, deploy\""),
        ]);
        let defaults = load(&source);
        assert_eq!(defaults.mode, Mode::Override);
        assert_eq!(defaults.java_executable, "/opt/jdk21/bin/java");
        assert_eq!(defaults.maven_program, "mvn");
        assert_eq!(defaults.platform, "windows");
        assert_eq!(defaults.jar_dirs, vec!["build/jars", "deploy"]);
    }

    #[test]
    fn reads_max_configurations_with_bounds() {
        let source = Fake::new(&[(KEY_PROVIDER_MAX, "12")]);
        assert_eq!(load(&source).provider.max_configurations, 12);
        let source = Fake::new(&[(KEY_PROVIDER_MAX, r#""7""#)]);
        assert_eq!(load(&source).provider.max_configurations, 7);
        let source = Fake::new(&[(KEY_PROVIDER_MAX, "0")]);
        let defaults = load(&source);
        assert_eq!(defaults.provider.max_configurations, 24);
        assert_eq!(defaults.warnings.len(), 1);
        let source = Fake::new(&[(KEY_PROVIDER_MAX, "\"lots\"")]);
        assert_eq!(load(&source).provider.max_configurations, 24);
    }

    #[test]
    fn profiles_split_and_dedupe() {
        let source = Fake::new(&[(KEY_SPRING_PROFILES, "\"dev, test dev\"")]);
        let defaults = load(&source);
        assert_eq!(defaults.spring_profiles, vec!["dev", "test"]);
        assert_eq!(
            defaults.effective_vm_args(),
            vec!["-Dspring.profiles.active=dev,test"]
        );
    }

    #[test]
    fn unterminated_quote_warns_and_keeps_the_value() {
        let source = Fake::new(&[(KEY_VM_OPTIONS, r#""-Xmx1g -Dfoo=\"bar""#)]);
        let defaults = load(&source);
        assert_eq!(defaults.vm_options, vec!["-Xmx1g", "-Dfoo=bar"]);
        assert_eq!(defaults.warnings.len(), 1);
        assert!(defaults.warnings[0].contains("unterminated"));
    }

    #[test]
    fn raw_text_and_arrays_are_tolerated() {
        let source = Fake::new(&[(KEY_VM_OPTIONS, "-Xmx1g")]);
        assert_eq!(load(&source).vm_options, vec!["-Xmx1g"]);
        let source = Fake::new(&[(KEY_VM_OPTIONS, r#"["-Xmx1g","-ea"]"#)]);
        assert_eq!(load(&source).vm_options, vec!["-Xmx1g", "-ea"]);
    }

    #[test]
    fn summary_mentions_both_sides() {
        let source = Fake::new(&[
            (KEY_VM_OPTIONS, r#""-Xmx1g""#),
            (KEY_PROGRAM_ARGS, r#""--debug""#),
        ]);
        let summary = load(&source).summary();
        assert!(summary.contains("-Xmx1g"));
        assert!(summary.contains("--debug"));
        assert!(summary.contains("append"));
    }
}
