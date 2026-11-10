//! Reading and patching VS Code `launch.json` files.
//!
//! OpenIDEA reads `.vscode/launch.json` next to JetBrains run configurations, so
//! giving a Java launch configuration default VM options means editing that file.
//! The file may contain comments and trailing commas; we parse it leniently but
//! always write plain JSON.

use crate::jsonc;
use crate::shell;

/// One `"type": "java"` `launch` configuration from `launch.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JavaConfig {
    /// Index in the `configurations` array, used to patch the same entry back.
    pub index: usize,
    pub name: String,
    pub config_type: String,
    pub request: String,
    pub main_class: Option<String>,
    pub project_name: Option<String>,
    pub module_name: Option<String>,
    pub vm_args: Vec<String>,
    pub args: Vec<String>,
    pub class_paths: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

impl JavaConfig {
    /// A `type: "java"` configuration that launches something (not `attach`).
    pub fn is_launch(&self) -> bool {
        self.config_type == "java" && (self.request.is_empty() || self.request == "launch")
    }
}

/// Parses all Java configurations. Other entries are ignored.
pub fn parse(text: &str) -> Result<Vec<JavaConfig>, String> {
    let value = jsonc::parse(text).map_err(|err| format!("invalid JSON: {err}"))?;
    let Some(configurations) = value.get("configurations") else {
        return Ok(Vec::new());
    };
    let Some(configurations) = configurations.as_array() else {
        return Err("'configurations' is not an array".to_string());
    };
    let mut out = Vec::new();
    for (index, entry) in configurations.iter().enumerate() {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let config_type = string_field(object, "type").unwrap_or_default();
        if config_type != "java" {
            continue;
        }
        let mut config = JavaConfig {
            index,
            name: string_field(object, "name").unwrap_or_default(),
            config_type,
            request: string_field(object, "request").unwrap_or_default(),
            main_class: string_field(object, "mainClass"),
            project_name: string_field(object, "projectName"),
            module_name: string_field(object, "moduleName"),
            vm_args: args_field(object, "vmArgs"),
            args: args_field(object, "args"),
            class_paths: args_field(object, "classPaths"),
            cwd: string_field(object, "cwd"),
            env: env_field(object, "env"),
        };
        config.name = if config.name.is_empty() {
            format!("java #{index}")
        } else {
            config.name
        };
        out.push(config);
    }
    Ok(out)
}

/// Sets `vmArgs` of the configuration at `index`, keeping its shape (a string
/// stays a string unless it had to change, an array stays an array).
pub fn set_vm_args(text: &str, index: usize, vm_args: &[String]) -> Result<String, String> {
    set_many_vm_args(text, &[(index, vm_args.to_vec())])
}

/// Sets `vmArgs` for several configurations in one pass, so a file with many
/// Java launch configurations is parsed and written once.
pub fn set_many_vm_args(
    text: &str,
    updates: &[(usize, Vec<String>)],
) -> Result<String, String> {
    let mut value = jsonc::parse(text).map_err(|err| format!("invalid JSON: {err}"))?;
    for (index, vm_args) in updates {
        let entry = value
            .get_mut("configurations")
            .and_then(|configurations| configurations.get_mut(*index))
            .ok_or_else(|| format!("no configuration at index {index}"))?;
        let object = entry
            .as_object_mut()
            .ok_or_else(|| format!("configuration {index} is not an object"))?;
        let was_array = matches!(object.get("vmArgs"), Some(serde_json::Value::Array(_)));
        let new_value = if was_array {
            serde_json::Value::Array(
                vm_args
                    .iter()
                    .map(|arg| serde_json::Value::String(arg.clone()))
                    .collect(),
            )
        } else {
            serde_json::Value::String(shell::join_args(vm_args))
        };
        object.insert("vmArgs".to_string(), new_value);
    }
    Ok(jsonc::to_writer_string(&value))
}

fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    match object.get(key) {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(other) if !other.is_null() => Some(other.to_string()),
        _ => None,
    }
}

fn args_field(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Vec<String> {
    match object.get(key) {
        Some(serde_json::Value::String(s)) => {
            if s.trim().is_empty() {
                Vec::new()
            } else {
                shell::split_args(s)
            }
        }
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn env_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Vec<(String, String)> {
    match object.get(key) {
        Some(serde_json::Value::Object(env)) => env
            .iter()
            .map(|(name, value)| {
                let text = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (name.clone(), text)
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAUNCH: &str = r#"{
  // Java launch configurations
  "version": "0.2.0",
  "configurations": [
    {
      "type": "java",
      "name": "DemoApplication",
      "request": "launch",
      "mainClass": "com.example.DemoApplication",
      "projectName": "demo",
      "vmArgs": "-Xmx512m -Dfoo=bar", /* inline comment */
      "args": "--server.port=8080",
      "cwd": "${workspaceFolder}",
      "env": { "SPRING_PROFILES_ACTIVE": "dev" },
    },
    {
      "type": "node",
      "name": "node",
      "request": "launch",
      "program": "app.js",
    },
    {
      "type": "java",
      "name": "Remote",
      "request": "attach",
      "hostName": "localhost",
      "port": 5005,
    },
  ],
}
"#;

    #[test]
    fn parses_java_launch_configurations() {
        let configs = parse(LAUNCH).unwrap();
        assert_eq!(configs.len(), 2);
        let first = &configs[0];
        assert_eq!(first.index, 0);
        assert_eq!(first.name, "DemoApplication");
        assert_eq!(first.main_class.as_deref(), Some("com.example.DemoApplication"));
        assert_eq!(first.project_name.as_deref(), Some("demo"));
        assert_eq!(first.vm_args, vec!["-Xmx512m", "-Dfoo=bar"]);
        assert_eq!(first.args, vec!["--server.port=8080"]);
        assert_eq!(first.cwd.as_deref(), Some("${workspaceFolder}"));
        assert_eq!(first.env, vec![("SPRING_PROFILES_ACTIVE".to_string(), "dev".to_string())]);
        assert!(first.is_launch());

        let second = &configs[1];
        assert_eq!(second.index, 2);
        assert!(!second.is_launch());
    }

    #[test]
    fn ignores_files_without_configurations() {
        assert!(parse("{}").unwrap().is_empty());
        assert!(parse(r#"{"configurations": "oops"}"#).is_err());
    }

    #[test]
    fn patches_vm_args_and_keeps_the_rest() {
        let updated = set_vm_args(LAUNCH, 0, &["-Xmx2g".to_string(), "-Dbar=b".to_string()]).unwrap();
        let configs = parse(&updated).unwrap();
        assert_eq!(configs[0].vm_args, vec!["-Xmx2g", "-Dbar=b"]);
        assert_eq!(configs[0].main_class.as_deref(), Some("com.example.DemoApplication"));
        assert_eq!(configs[0].args, vec!["--server.port=8080"]);
        assert_eq!(configs[0].env.len(), 1);
        // Other entries are preserved.
        assert!(updated.contains("\"type\": \"node\""));
        assert!(updated.contains("\"port\": 5005"));
        // The written file is plain JSON.
        assert!(!updated.contains("//"));
    }

    #[test]
    fn patches_vm_args_in_place_when_it_was_an_array() {
        let text = r#"{"configurations":[{"type":"java","request":"launch","name":"A","vmArgs":["-ea"]}]}"#;
        let updated = set_vm_args(text, 0, &["-Xmx2g".to_string()]).unwrap();
        let value = jsonc::parse(&updated).unwrap();
        assert_eq!(value["configurations"][0]["vmArgs"][0], "-Xmx2g");
    }

    #[test]
    fn reports_missing_index() {
        assert!(set_vm_args(LAUNCH, 9, &["-ea".to_string()]).is_err());
        assert!(set_vm_args("not json", 0, &[]).is_err());
    }
}
