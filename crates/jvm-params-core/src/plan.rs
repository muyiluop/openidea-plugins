//! Turning the settings and a detected project into launch configurations.
//!
//! The provider can only express "run this program with these arguments", so the
//! plan is a small set of strategies per module, ordered by how likely they are
//! to be exactly what the user wants:
//!
//! 1. `bootJar` — `java <vm options> -jar <jar> <program args>`; needs a built
//!    jar, but it is the only shape that carries JVM options faithfully without
//!    a build tool in between.
//! 2. `springBootMaven` — `mvnw spring-boot:run` with
//!    `-Dspring-boot.run.jvmArguments=…` and `-Dspring-boot.run.arguments=…`.
//! 3. `springBootGradle` — `gradlew bootRun --args=…`; Gradle reads `jvmArgs`
//!    only from the build script, so VM options are NOT injected here. The
//!    generated name says so, and the jar strategy is the faithful one.
//!
//! A configuration is a plain command line: no shell, no classpath resolution.
//! Everything is computed from the project root, so the module is testable with
//! a fake file map and no host at all.

use crate::detect::{join_path, Build, Module, Project, Wrappers};
use crate::settings::Defaults;

/// The command-line conventions of the platform the IDE runs on. A WASM guest
/// cannot query the host OS, so the project root string decides, and
/// `jvmParams.platform` can override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

impl Platform {
    /// `D:\work\app` or `C:/work/app` are Windows roots; `C:` in the second
    /// byte cannot appear in a Unix path.
    pub fn detect(root: &str) -> Self {
        if root.contains('\\') || root.as_bytes().get(1) == Some(&b':') {
            Platform::Windows
        } else {
            Platform::Unix
        }
    }

    /// Parses the `jvmParams.platform` setting; `auto`/unknown means "detect".
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "windows" | "win" | "win32" => Some(Platform::Windows),
            "unix" | "linux" | "mac" | "macos" | "darwin" => Some(Platform::Unix),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Unix => "unix",
        }
    }
}

/// The platform to plan for: the setting when it names one, else the root.
pub fn resolve_platform(root: &str, defaults: &Defaults) -> Platform {
    Platform::parse(&defaults.platform).unwrap_or_else(|| Platform::detect(root))
}

/// One launch configuration contributed to the IDE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedConfig {
    /// Stable id suffix; the plugin prefixes it with its own namespace.
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    /// Working directory (absolute when the project root is absolute).
    pub cwd: Option<String>,
    pub test: bool,
    pub framework: Option<String>,
    /// The strategy this configuration was generated from.
    pub strategy: Strategy,
}

impl GeneratedConfig {
    /// The full command line, program first, for previews and logs.
    pub fn argv(&self) -> Vec<String> {
        let mut argv = Vec::with_capacity(self.args.len() + 1);
        argv.push(self.program.clone());
        argv.extend(self.args.iter().cloned());
        argv
    }

    /// The command line as one line, quoting arguments that need it.
    pub fn argv_line(&self) -> String {
        crate::shell::join_args(&self.argv())
    }
}

/// Which launch strategy produced a configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    BootJar,
    SpringBootMaven,
    SpringBootGradle,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Strategy::BootJar => "bootjar",
            Strategy::SpringBootMaven => "spring-boot-maven",
            Strategy::SpringBootGradle => "spring-boot-gradle",
        }
    }

    /// Human-readable label used in configuration names.
    pub fn label(self) -> &'static str {
        match self {
            Strategy::BootJar => "jar",
            Strategy::SpringBootMaven => "Spring Boot · Maven",
            Strategy::SpringBootGradle => "Spring Boot · Gradle",
        }
    }
}

/// Builds the configurations for a project. Empty when the plugin is disabled,
/// when there is nothing to inject, or when no strategy applies.
pub fn generate(project: &Project, defaults: &Defaults, platform: Platform) -> Vec<GeneratedConfig> {
    if !defaults.enabled || !defaults.provider.enabled || !defaults.has_parameters() {
        return Vec::new();
    }
    let vm_args = defaults.effective_vm_args();
    let mut out: Vec<GeneratedConfig> = Vec::new();

    for module in &project.modules {
        let mut strategies: Vec<GeneratedConfig> = Vec::new();

        if defaults.provider.boot_jar {
            if let Some(config) = boot_jar_config(project, module, defaults, &vm_args, platform) {
                strategies.push(config);
            }
        }
        if module.spring_boot {
            match module.build {
                Build::Maven => {
                    if defaults.provider.spring_boot_maven {
                        if let Some(config) =
                            spring_boot_maven_config(project, module, defaults, &vm_args, platform)
                        {
                            strategies.push(config);
                        }
                    }
                }
                Build::Gradle => {
                    if defaults.provider.spring_boot_gradle {
                        if let Some(config) =
                            spring_boot_gradle_config(project, module, defaults, platform)
                        {
                            strategies.push(config);
                        }
                    }
                }
            }
        }

        if !defaults.provider.emit_all_strategies {
            strategies.truncate(1);
        }
        out.extend(strategies);
    }

    // Deterministic ids, then the hard cap.
    let mut used: Vec<String> = Vec::new();
    for config in out.iter_mut() {
        let mut id = config.id.clone();
        let mut suffix = 2;
        while used.contains(&id) {
            id = format!("{}-{suffix}", config.id);
            suffix += 1;
        }
        used.push(id.clone());
        config.id = id;
    }
    out.truncate(defaults.provider.max_configurations);
    out
}

fn boot_jar_config(
    project: &Project,
    module: &Module,
    defaults: &Defaults,
    vm_args: &[String],
    platform: Platform,
) -> Option<GeneratedConfig> {
    let jar = module.boot_jar.as_ref()?;
    let jar = join_path(&project.root, jar);
    let program = java_program(defaults);
    let mut args = vm_args.to_vec();
    args.push("-jar".to_string());
    args.push(jar);
    args.extend(defaults.program_args.iter().cloned());
    Some(GeneratedConfig {
        id: config_id(Strategy::BootJar, module),
        name: config_name(module, Strategy::BootJar, defaults, platform),
        program,
        args,
        cwd: Some(join_path(&project.root, &module.dir)),
        test: false,
        framework: None,
        strategy: Strategy::BootJar,
    })
}

fn spring_boot_maven_config(
    project: &Project,
    module: &Module,
    defaults: &Defaults,
    vm_args: &[String],
    platform: Platform,
) -> Option<GeneratedConfig> {
    let program = build_program(&module.wrappers, Build::Maven, defaults, platform)?;
    // One argv element per -D: the value itself is unquoted, so a JVM options
    // string with spaces arrives as a single argument.
    let mut args: Vec<String> = vec!["-B".to_string()];
    if !module.dir.is_empty() {
        args.push("-pl".to_string());
        args.push(module.dir.clone());
        args.push("-am".to_string());
    }
    if !vm_args.is_empty() {
        args.push(format!(
            "-Dspring-boot.run.jvmArguments={}",
            crate::shell::join_args(vm_args)
        ));
        // Without forking, the plugin's VM options would be applied to Maven.
        args.push("-Dspring-boot.run.fork=true".to_string());
    }
    if !defaults.program_args.is_empty() {
        args.push(format!(
            "-Dspring-boot.run.arguments={}",
            crate::shell::join_args(&defaults.program_args)
        ));
    }
    args.push("spring-boot:run".to_string());
    Some(GeneratedConfig {
        id: config_id(Strategy::SpringBootMaven, module),
        name: config_name(module, Strategy::SpringBootMaven, defaults, platform),
        program,
        args,
        // The wrapper is used from the aggregator root, so -pl finds the module.
        cwd: Some(project.root.clone()),
        test: false,
        framework: None,
        strategy: Strategy::SpringBootMaven,
    })
}

fn spring_boot_gradle_config(
    project: &Project,
    module: &Module,
    defaults: &Defaults,
    platform: Platform,
) -> Option<GeneratedConfig> {
    let program = build_program(&module.wrappers, Build::Gradle, defaults, platform)?;
    let mut args: Vec<String> = vec!["bootRun".to_string()];
    if !defaults.program_args.is_empty() {
        args.push(format!(
            "--args={}",
            crate::shell::join_args(&defaults.program_args)
        ));
    }
    Some(GeneratedConfig {
        id: config_id(Strategy::SpringBootGradle, module),
        name: config_name(module, Strategy::SpringBootGradle, defaults, platform),
        program,
        args,
        cwd: Some(join_path(&project.root, &module.dir)),
        test: false,
        framework: None,
        strategy: Strategy::SpringBootGradle,
    })
}

/// The `java` program: the setting when it names one, else a bare `java` so the
/// IDE resolves it on `PATH` (and can approve `process:java` once).
pub fn java_program(defaults: &Defaults) -> String {
    let program = defaults.java_executable.trim();
    if program.is_empty() {
        "java".to_string()
    } else {
        program.to_string()
    }
}

/// The build-tool program token: the explicit setting, else the wrapper script
/// that matches the platform, else the other flavor, else nothing.
///
/// The value is a *token*, never a path: the IDE matches a plugin's launch
/// configuration against an approved program by the exact string in the record,
/// and only tokens declared in the plugin manifest can be approved at all. A
/// project-relative or absolute path returned here would make the configuration
/// permanently invisible in the Run dialog. `mvnw.cmd` and friends are declared
/// in `[capabilities]`, and the IDE pins the actual file per project.
pub fn build_program(
    wrappers: &Wrappers,
    build: Build,
    defaults: &Defaults,
    platform: Platform,
) -> Option<String> {
    let (configured, cmd_flavor, unix_flavor) = match build {
        Build::Maven => (&defaults.maven_program, &wrappers.mvnw_cmd, &wrappers.mvnw_unix),
        Build::Gradle => (
            &defaults.gradle_program,
            &wrappers.gradlew_bat,
            &wrappers.gradlew_unix,
        ),
    };
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    let (first, second) = match platform {
        Platform::Windows => (cmd_flavor, unix_flavor),
        Platform::Unix => (unix_flavor, cmd_flavor),
    };
    let relative = first.as_ref().or(second.as_ref())?;
    // The detected wrapper is stored as a project-relative path; only its file
    // name is a program token.
    Some(
        relative
            .rsplit(|c| c == '/' || c == '\\')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(relative.as_str())
            .to_string(),
    )
}

/// `jvm-defaults:<strategy>:<module slug>`, stable across rebuilds: no version,
/// no absolute path.
pub fn config_id(strategy: Strategy, module: &Module) -> String {
    format!("jvm-defaults:{}:{}", strategy.as_str(), module_slug(module))
}

/// `attach-gateway` for a submodule, the folder name for the root module.
fn module_slug(module: &Module) -> String {
    let raw = if module.dir.is_empty() {
        module.name.clone()
    } else {
        module.dir.clone()
    };
    let mut slug = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

fn config_name(
    module: &Module,
    strategy: Strategy,
    defaults: &Defaults,
    platform: Platform,
) -> String {
    let mut name = format!("{} · {} [JVM defaults]", module.name, strategy.label());
    if matches!(strategy, Strategy::SpringBootGradle) && !defaults.effective_vm_args().is_empty() {
        // Say it out loud: Gradle cannot receive JVM options from the command
        // line, so this configuration only carries the program arguments.
        name.push_str(" (program args only)");
    }
    let _ = platform;
    name
}

/// A human-readable preview of what the plugin would do, used by a command.
pub fn preview(project: &Project, defaults: &Defaults, platform: Platform) -> String {
    let mut out = String::new();
    out.push_str("JVM Default Parameters\n");
    out.push_str(&format!("  project: {}\n", project.root));
    out.push_str(&format!("  platform: {}\n", platform.as_str()));
    out.push_str(&format!(
        "  VM options: {}\n",
        empty_marker(&defaults.effective_vm_args())
    ));
    out.push_str(&format!(
        "  Program args: {}\n",
        empty_marker(&defaults.program_args)
    ));
    out.push_str(&format!("  mode: {}\n", defaults.mode.as_str()));
    out.push_str(&format!(
        "  provider: {} (jar {}, maven {}, gradle {})\n",
        on_off(defaults.provider.enabled),
        on_off(defaults.provider.boot_jar),
        on_off(defaults.provider.spring_boot_maven),
        on_off(defaults.provider.spring_boot_gradle)
    ));

    if project.modules.is_empty() {
        out.push_str("\nNo Maven or Gradle module was detected in this project.\n");
    } else {
        out.push_str("\nDetected modules:\n");
        for module in &project.modules {
            out.push_str(&format!(
                "  {} · {} · {} · jar: {} · wrapper: {}\n",
                if module.dir.is_empty() {
                    "<root>"
                } else {
                    module.dir.as_str()
                },
                module.build.as_str(),
                if module.spring_boot {
                    "Spring Boot"
                } else {
                    "plain"
                },
                module.boot_jar.as_deref().unwrap_or("—"),
                wrapper_marker(&module.wrappers)
            ));
        }
    }

    let configs = generate(project, defaults, platform);
    if configs.is_empty() {
        out.push_str("\nNo launch configuration would be contributed.\n");
    } else {
        out.push_str("\nContributed launch configurations:\n");
        for (index, config) in configs.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", index + 1, config.name));
            out.push_str(&format!("     id: {}\n", config.id));
            out.push_str(&format!("     {}\n", config.argv_line()));
        }
    }

    if !defaults.warnings.is_empty() {
        out.push_str("\nSettings warnings:\n");
        for warning in &defaults.warnings {
            out.push_str(&format!("  ! {warning}\n"));
        }
    }
    out
}

fn empty_marker(args: &[String]) -> String {
    if args.is_empty() {
        "—".to_string()
    } else {
        crate::shell::join_args(args)
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn wrapper_marker(wrappers: &Wrappers) -> String {
    let mut found = Vec::new();
    if wrappers.mvnw_cmd.is_some() {
        found.push("mvnw.cmd");
    }
    if wrappers.mvnw_unix.is_some() {
        found.push("mvnw");
    }
    if wrappers.gradlew_bat.is_some() {
        found.push("gradlew.bat");
    }
    if wrappers.gradlew_unix.is_some() {
        found.push("gradlew");
    }
    if found.is_empty() {
        "—".to_string()
    } else {
        found.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::MapReader;
    use crate::detect::{detect_with_options, DetectOptions, Files};
    use crate::merge::Mode;

    fn defaults(vm: &str, program: &str) -> Defaults {
        let mut defaults = Defaults::schema_defaults();
        defaults.vm_options = crate::shell::split_args(vm);
        defaults.program_args = crate::shell::split_args(program);
        defaults.mode = Mode::Append;
        defaults
    }

    fn files(pairs: &[(&str, &str)]) -> Files {
        let mut files = Files::default();
        for (path, _) in pairs {
            push(&mut files, path);
        }
        files
    }

    fn push(files: &mut Files, path: &str) {
        let name = path.rsplit('/').next().unwrap_or(path);
        match name {
            "pom.xml" => files.poms.push(path.to_string()),
            "build.gradle" | "build.gradle.kts" => files.gradle_builds.push(path.to_string()),
            "mvnw" | "mvnw.cmd" | "gradlew" | "gradlew.bat" => files.wrappers.push(path.to_string()),
            _ if name.ends_with(".jar") => files.jars.push(path.to_string()),
            _ => {}
        }
    }

    fn reader(pairs: &[(&str, &str)]) -> MapReader {
        let mut reader = MapReader::new();
        for (path, contents) in pairs {
            reader = reader.with(path, contents);
        }
        reader
    }

    const POM: &str = r#"<project><artifactId>app</artifactId>
<build><plugins><plugin><groupId>org.springframework.boot</groupId>
<artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>"#;

    #[test]
    fn detects_the_platform_from_the_root_string() {
        assert_eq!(Platform::detect(r"D:\work\app"), Platform::Windows);
        assert_eq!(Platform::detect("C:/work/app"), Platform::Windows);
        assert_eq!(Platform::detect("/home/me/app"), Platform::Unix);
        assert_eq!(Platform::parse("auto"), None);
        assert_eq!(Platform::parse("Windows"), Some(Platform::Windows));
        assert_eq!(Platform::parse("linux"), Some(Platform::Unix));
        let mut defaults = Defaults::schema_defaults();
        assert_eq!(resolve_platform(r"D:\work\app", &defaults), Platform::Windows);
        defaults.platform = "unix".to_string();
        assert_eq!(resolve_platform(r"D:\work\app", &defaults), Platform::Unix);
    }

    #[test]
    fn maven_strategy_builds_the_canonical_argv() {
        let root = r"D:\work\app";
        let files = files(&[
            ("pom.xml", ""),
            ("attach-gateway/pom.xml", ""),
            ("mvnw.cmd", ""),
            ("mvnw", ""),
        ]);
        let project = detect_with_options(
            root,
            &files,
            &reader(&[("pom.xml", POM), ("attach-gateway/pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let defaults = defaults("-Xmx1g -Dspring.profiles.active=dev", "--server.port=8081");
        let configs = generate(&project, &defaults, Platform::Windows);
        assert_eq!(configs.len(), 2);
        // Root module first, wrapper from the project root.
        assert_eq!(configs[0].strategy, Strategy::SpringBootMaven);
        assert_eq!(configs[0].program, "mvnw.cmd");
        assert_eq!(
            configs[0].args,
            vec![
                "-B",
                "-Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev",
                "-Dspring-boot.run.fork=true",
                "-Dspring-boot.run.arguments=--server.port=8081",
                "spring-boot:run",
            ]
        );
        assert_eq!(configs[1].args[1], "-pl");
        assert_eq!(configs[1].args[2], "attach-gateway");
        assert_eq!(configs[1].args[3], "-am");
        assert_eq!(configs[1].cwd.as_deref(), Some(root));
        assert_eq!(configs[1].id, "jvm-defaults:spring-boot-maven:attach-gateway");
    }

    #[test]
    fn jar_strategy_is_preferred_over_the_wrapper() {
        let root = r"D:\work\app";
        let files = files(&[
            ("pom.xml", ""),
            ("mvnw.cmd", ""),
            ("target/app-1.0.0.jar", ""),
            ("target/app-1.0.0-sources.jar", ""),
            ("target/app-1.0.0.jar.original", ""),
        ]);
        let project = detect_with_options(
            root,
            &files,
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let defaults = defaults("-Xmx2g -ea", "--debug");
        let configs = generate(&project, &defaults, Platform::Windows);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].strategy, Strategy::BootJar);
        assert_eq!(configs[0].program, "java");
        assert_eq!(
            configs[0].args,
            vec!["-Xmx2g", "-ea", "-jar", r"D:\work\app\target\app-1.0.0.jar", "--debug"]
        );
        assert_eq!(configs[0].id, "jvm-defaults:bootjar:app");
    }

    #[test]
    fn emit_all_strategies_lists_every_candidate() {
        let root = "/home/me/app";
        let files = files(&[("pom.xml", ""), ("mvnw", ""), ("target/app.jar", "")]);
        let project = detect_with_options(
            root,
            &files,
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let mut defaults = defaults("-Xmx1g", "");
        defaults.provider.emit_all_strategies = true;
        let configs = generate(&project, &defaults, Platform::Unix);
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].strategy, Strategy::BootJar);
        assert_eq!(configs[1].strategy, Strategy::SpringBootMaven);
        assert_eq!(configs[1].program, "mvnw");
        assert_eq!(configs[1].args[0], "-B");
    }

    #[test]
    fn gradle_strategy_carries_program_args_only() {
        let root = "/home/me/app";
        let files = files(&[("build.gradle", ""), ("gradlew", "")]);
        let project = detect_with_options(
            root,
            &files,
            &reader(&[("build.gradle", "plugins { id 'org.springframework.boot' version '3.3.0' }")]),
            &DetectOptions::default(),
        );
        let defaults = defaults("-Xmx1g", "--debug --trace");
        let configs = generate(&project, &defaults, Platform::Unix);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].strategy, Strategy::SpringBootGradle);
        assert_eq!(configs[0].program, "gradlew");
        assert_eq!(configs[0].args, vec!["bootRun", "--args=--debug --trace"]);
        assert_eq!(configs[0].cwd.as_deref(), Some(root));
        assert!(configs[0].name.contains("program args only"));
    }

    #[test]
    fn nothing_is_generated_without_parameters_or_when_disabled() {
        let project = detect_with_options(
            "/p",
            &files(&[("pom.xml", ""), ("mvnw", "")]),
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::default(),
        );
        assert!(generate(&project, &Defaults::schema_defaults(), Platform::Unix).is_empty());
        let mut disabled = defaults("-Xmx1g", "");
        disabled.enabled = false;
        assert!(generate(&project, &disabled, Platform::Unix).is_empty());
        let mut provider_off = defaults("-Xmx1g", "");
        provider_off.provider.enabled = false;
        assert!(generate(&project, &provider_off, Platform::Unix).is_empty());
    }

    #[test]
    fn configured_programs_win_over_the_wrapper() {
        let project = detect_with_options(
            "/p",
            &files(&[("pom.xml", ""), ("mvnw", "")]),
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let mut defaults = defaults("-Xmx1g", "");
        defaults.maven_program = "mvn".to_string();
        defaults.java_executable = "/opt/jdk/bin/java".to_string();
        let configs = generate(&project, &defaults, Platform::Unix);
        assert_eq!(configs[0].program, "mvn");
    }

    #[test]
    fn missing_wrapper_falls_back_to_nothing_for_maven() {
        let project = detect_with_options(
            "/p",
            &files(&[("pom.xml", "")]),
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let mut defaults = defaults("-Xmx1g", "");
        defaults.provider.boot_jar = false;
        assert!(generate(&project, &defaults, Platform::Unix).is_empty());
    }

    #[test]
    fn ids_are_unique_and_capped() {
        let root = "/p";
        let files = files(&[
            ("a/pom.xml", ""),
            ("b/pom.xml", ""),
            ("mvnw", ""),
        ]);
        let project = detect_with_options(
            root,
            &files,
            &reader(&[("a/pom.xml", POM), ("b/pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let mut defaults = defaults("-Xmx1g", "");
        defaults.provider.max_configurations = 1;
        let configs = generate(&project, &defaults, Platform::Unix);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].id, "jvm-defaults:spring-boot-maven:a");
    }

    #[test]
    fn extra_jar_dirs_are_searched() {
        let root = "/p";
        let files = files(&[("pom.xml", ""), ("mvnw", ""), ("build/jars/app.jar", "")]);
        let project = detect_with_options(
            root,
            &files,
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::with_extra_dirs(&["build/jars".to_string()]),
        );
        assert_eq!(project.modules[0].boot_jar.as_deref(), Some("build/jars/app.jar"));
    }

    #[test]
    fn preview_mentions_modules_and_argv() {
        let project = detect_with_options(
            r"D:\p",
            &files(&[("pom.xml", ""), ("mvnw.cmd", ""), ("target/app.jar", "")]),
            &reader(&[("pom.xml", POM)]),
            &DetectOptions::default(),
        );
        let text = preview(&project, &defaults("-Xmx1g", ""), Platform::Windows);
        assert!(text.contains("platform: windows"));
        assert!(text.contains("-Xmx1g"));
        assert!(text.contains("jvm-defaults:bootjar:"));
        assert!(text.contains("-jar"));
    }
}
