//! Writing the defaults into the run-configuration files the IDE already reads.
//!
//! The provider interface can add launch configurations, but it cannot change the
//! configurations the IDE itself created (a Maven/Spring Boot configuration in
//! `.idea/workspace.xml`, a `.run/*.run.xml` file, or a `.vscode/launch.json`
//! entry). The IntelliJ *JVM Default Options* feature works by injecting
//! `VM_PARAMETERS` into exactly those files — so this plugin can do the same:
//! read the file, merge the defaults into each Java configuration, and write the
//! file back with everything else untouched.
//!
//! Patching is idempotent: the merge deduplicates by option key, so a file that
//! already carries the defaults is left byte-identical.

use crate::jetbrains;
use crate::merge;
use crate::settings::Defaults;
use crate::shell;
use crate::vscode;

/// Run-configuration files the IDE reads, as `find_files` globs.
pub fn run_file_globs() -> Vec<&'static str> {
    vec![
        ".run/*.run.xml",
        ".idea/runConfigurations/*.xml",
        ".vscode/launch.json",
    ]
}

/// The workspace file that also holds run configurations; patched by path
/// because it is not a run configuration file as such.
pub const WORKSPACE_XML: &str = ".idea/workspace.xml";

/// Result of patching one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patched {
    pub text: String,
    /// Number of configurations whose parameters changed.
    pub updated: usize,
}

/// Patches whichever format `path` names: `.json` is a VS Code `launch.json`,
/// anything else is JetBrains XML.
pub fn patch_file(path: &str, text: &str, defaults: &Defaults) -> Result<Patched, String> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".json") {
        patch_launch_json(text, defaults)
    } else if lower.ends_with(".xml") {
        Ok(patch_run_xml(text, defaults))
    } else {
        Err(format!("{path}: not a run configuration file"))
    }
}

/// Merges the defaults into every Java application configuration of a JetBrains
/// run-configuration file.
pub fn patch_run_xml(text: &str, defaults: &Defaults) -> Patched {
    let vm_defaults = defaults.effective_vm_args();
    let program_defaults = defaults.program_args.clone();
    let configs = jetbrains::parse(text);
    let mut updated_text = text.to_string();
    let mut updated = 0usize;

    // Back to front: earlier byte offsets stay valid while editing later blocks.
    for config in configs.iter().rev() {
        if !config.is_java_application() {
            continue;
        }
        let changed = merge_config(&mut updated_text, config, &vm_defaults, &program_defaults, defaults);
        if changed {
            updated += 1;
        }
    }
    Patched {
        text: updated_text,
        updated,
    }
}

/// Merges the defaults into one JetBrains configuration's `VM_PARAMETERS` and
/// `PROGRAM_PARAMETERS`. Returns whether anything changed.
pub fn merge_config(
    text: &mut String,
    config: &jetbrains::Config,
    vm_defaults: &[String],
    program_defaults: &[String],
    defaults: &Defaults,
) -> bool {
    let mut changed = false;

    let existing_vm = config.vm_args();
    let merged_vm = merge::merge_args(&existing_vm, vm_defaults, defaults.mode);
    if merged_vm != existing_vm {
        let value = shell::join_args(&merged_vm);
        let (next, _) = jetbrains::set_vm_parameters(text, config, &value);
        *text = next;
        changed = true;
    }

    // Only touch the program arguments when the user configured some: an empty
    // setting must not blank out what the configuration already has.
    if !program_defaults.is_empty() {
        let existing_program = config.program_args();
        let merged_program = merge::merge_args(&existing_program, program_defaults, defaults.mode);
        if merged_program != existing_program {
            let value = shell::join_args(&merged_program);
            let (next, _) = jetbrains::set_option_value(text, config, "PROGRAM_PARAMETERS", &value);
            *text = next;
            changed = true;
        }
    }
    changed
}

/// Merges the defaults into every Java launch configuration of `launch.json`.
pub fn patch_launch_json(text: &str, defaults: &Defaults) -> Result<Patched, String> {
    let vm_defaults = defaults.effective_vm_args();
    let configs = vscode::parse(text)?;
    let mut updates: Vec<(usize, Vec<String>)> = Vec::new();
    for config in &configs {
        if !config.is_launch() {
            continue;
        }
        let merged = merge::merge_args(&config.vm_args, &vm_defaults, defaults.mode);
        if merged != config.vm_args {
            updates.push((config.index, merged));
        }
    }
    if updates.is_empty() {
        return Ok(Patched {
            text: text.to_string(),
            updated: 0,
        });
    }
    let updated = updates.len();
    let text = vscode::set_many_vm_args(text, &updates)?;
    Ok(Patched { text, updated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::Mode;

    fn defaults(vm: &str, program: &str) -> Defaults {
        let mut defaults = Defaults::schema_defaults();
        defaults.vm_options = shell::split_args(vm);
        defaults.program_args = shell::split_args(program);
        defaults.mode = Mode::Append;
        defaults
    }

    const WORKSPACE: &str = r#"<project version="4">
  <component name="RunManager" selected="Spring Boot.Demo">
    <configuration name="Demo" type="SpringBootApplicationConfigurationType" factoryName="Spring Boot">
      <module name="demo" />
      <option name="SPRING_BOOT_MAIN_CLASS" value="com.example.DemoApplication" />
      <option name="VM_PARAMETERS" value="-Xmx512m" />
      <option name="PROGRAM_PARAMETERS" value="--server.port=8080" />
      <method v="2">
        <option name="Make" enabled="true" />
      </method>
    </configuration>
    <configuration name="Plain" type="Application" factoryName="Application">
      <option name="MAIN_CLASS_NAME" value="com.example.Main" />
    </configuration>
    <configuration default="true" type="Application" factoryName="Application">
      <option name="VM_PARAMETERS" value="-ea" />
    </configuration>
    <configuration name="Tests" type="JUnit" factoryName="JUnit">
      <option name="VM_PARAMETERS" value="-ea" />
    </configuration>
  </component>
</project>
"#;

    #[test]
    fn patches_application_and_spring_boot_configurations() {
        let patched = patch_run_xml(WORKSPACE, &defaults("-Xmx1g -Dfoo=bar", ""));
        assert_eq!(patched.updated, 2);
        let configs = jetbrains::parse(&patched.text);
        let demo = configs
            .iter()
            .find(|config| config.name == "Demo")
            .expect("demo");
        assert_eq!(demo.vm_args(), vec!["-Xmx512m", "-Dfoo=bar"]);
        // Options that were not touched survive.
        assert_eq!(demo.main_class(), None);
        assert_eq!(demo.option("SPRING_BOOT_MAIN_CLASS"), Some("com.example.DemoApplication"));
        assert_eq!(demo.program_args(), vec!["--server.port=8080"]);
        // The template and the JUnit configuration keep their parameters.
        let template = configs
            .iter()
            .find(|config| config.default)
            .expect("template");
        assert_eq!(template.vm_args(), vec!["-ea"]);
        let tests = configs
            .iter()
            .find(|config| config.name == "Tests")
            .expect("tests");
        assert_eq!(tests.vm_args(), vec!["-ea"]);
        // The inserted option goes into the configuration that had none.
        let plain = configs
            .iter()
            .find(|config| config.name == "Plain")
            .expect("plain");
        assert_eq!(plain.vm_args(), vec!["-Xmx1g", "-Dfoo=bar"]);
    }

    #[test]
    fn patching_twice_is_idempotent() {
        let defaults = defaults("-Xmx1g -Dfoo=bar", "");
        let once = patch_run_xml(WORKSPACE, &defaults);
        let twice = patch_run_xml(&once.text, &defaults);
        assert_eq!(twice.text, once.text);
        assert_eq!(twice.updated, 0);
    }

    #[test]
    fn override_mode_replaces_existing_vm_parameters() {
        let mut defaults = defaults("-Xmx2g", "");
        defaults.mode = Mode::Override;
        let patched = patch_run_xml(WORKSPACE, &defaults);
        let configs = jetbrains::parse(&patched.text);
        let demo = configs.iter().find(|config| config.name == "Demo").unwrap();
        assert_eq!(demo.vm_args(), vec!["-Xmx2g"]);
    }

    #[test]
    fn program_arguments_are_patched_only_when_configured() {
        let patched = patch_run_xml(WORKSPACE, &defaults("", "--debug --trace"));
        let configs = jetbrains::parse(&patched.text);
        let demo = configs.iter().find(|config| config.name == "Demo").unwrap();
        assert_eq!(demo.program_args(), vec!["--server.port=8080", "--debug", "--trace"]);
    }

    #[test]
    fn spring_profiles_become_a_system_property() {
        let mut defaults = Defaults::schema_defaults();
        defaults.spring_profiles = vec!["dev".to_string()];
        let patched = patch_run_xml(WORKSPACE, &defaults);
        let configs = jetbrains::parse(&patched.text);
        let demo = configs.iter().find(|config| config.name == "Demo").unwrap();
        assert_eq!(
            demo.vm_args(),
            vec!["-Xmx512m", "-Dspring.profiles.active=dev"]
        );
    }

    #[test]
    fn nothing_is_patched_without_parameters() {
        let patched = patch_run_xml(WORKSPACE, &Defaults::schema_defaults());
        assert_eq!(patched.text, WORKSPACE);
        assert_eq!(patched.updated, 0);
    }

    const LAUNCH: &str = r#"{
  "version": "0.2.0",
  "configurations": [
    { "type": "java", "name": "Demo", "request": "launch", "mainClass": "com.example.Demo" },
    { "type": "java", "name": "Other", "request": "launch", "vmArgs": "-Xmx512m" },
    { "type": "java", "name": "Remote", "request": "attach", "vmArgs": "-Xmx512m" }
  ]
}
"#;

    #[test]
    fn patches_every_java_launch_configuration() {
        let patched = patch_launch_json(LAUNCH, &defaults("-Xmx1g", "")).unwrap();
        assert_eq!(patched.updated, 1);
        let configs = vscode::parse(&patched.text).unwrap();
        assert_eq!(configs[0].vm_args, vec!["-Xmx1g"]);
        // The configuration that already sets the same switch keeps its value.
        assert_eq!(configs[1].vm_args, vec!["-Xmx512m"]);
        // `attach` configurations are left alone.
        assert_eq!(configs[2].vm_args, vec!["-Xmx512m"]);
        // Re-patching changes nothing.
        let twice = patch_launch_json(&patched.text, &defaults("-Xmx1g", "")).unwrap();
        assert_eq!(twice.updated, 0);
        assert_eq!(twice.text, patched.text);
    }

    #[test]
    fn dispatch_by_extension() {
        let patched = patch_file(".vscode/launch.json", LAUNCH, &defaults("-Xmx1g", "")).unwrap();
        assert_eq!(patched.updated, 1);
        let patched = patch_file(".run/Demo.run.xml", WORKSPACE, &defaults("-Xmx1g", "")).unwrap();
        assert_eq!(patched.updated, 1);
        assert!(patch_file("readme.md", "", &Defaults::schema_defaults()).is_err());
    }

    #[test]
    fn invalid_json_is_an_error_not_a_panic() {
        assert!(patch_launch_json("{ not json", &defaults("-Xmx1g", "")).is_err());
    }

    #[test]
    fn broken_xml_degrades_to_no_changes() {
        let patched = patch_run_xml("<project><component", &defaults("-Xmx1g", ""));
        assert_eq!(patched.updated, 0);
        assert_eq!(patched.text, "<project><component");
    }
}
