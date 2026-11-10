//! Pins the plugin's values to the project: `<project>/.openidea/settings.toml`.
//!
//! A plugin's settings page has no `defaultScope`, so the IDE opens it in the
//! **global** scope: what the user types there applies to every project, which
//! makes it far too easy to change every project by accident. The host reads a
//! plugin's values from the project's effective settings whenever the root is an
//! open project (and from the global settings otherwise), so writing the values
//! into the project layer once makes the project win from then on: a later edit
//! of the global value no longer reaches this project.
//!
//! The file is a plain TOML document shared with the IDE's own settings
//! (`crates/ide-workspace/src/settings.rs`: defaults, then
//! `<app-config>/settings.toml`, then `<project>/.openidea/settings.toml`), so
//! only the `[plugins."<id>"]` table is rebuilt here and every other line is
//! kept as it is. Plugin keys are written quoted (`"jvmParams.vmOptions"`) since
//! a bare key would be read as a nested table.

use crate::settings::Defaults;

/// The project layer of the IDE's settings.
pub const PROJECT_SETTINGS: &str = ".openidea/settings.toml";

/// The value of one pinned key.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Bool(bool),
    Str(String),
}

/// The keys `upsert` writes, in the order it writes them.
fn values(defaults: &Defaults) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    out.push(("jvmParams.enabled", Value::Bool(defaults.enabled)));
    out.push((
        "jvmParams.vmOptions",
        Value::Str(crate::shell::join_args(&defaults.vm_options)),
    ));
    out.push((
        "jvmParams.programArgs",
        Value::Str(crate::shell::join_args(&defaults.program_args)),
    ));
    out.push((
        "jvmParams.springProfiles",
        Value::Str(defaults.spring_profiles.join(",")),
    ));
    out.push((
        "jvmParams.mode",
        Value::Str(defaults.mode.as_str().to_string()),
    ));
    out.push((
        "jvmParams.pinToProject",
        Value::Bool(defaults.pin_to_project),
    ));
    out
}

/// The `[plugins."id"]` header this module manages.
pub fn header(plugin_id: &str) -> String {
    format!("[plugins.{}]", toml_string(plugin_id))
}

/// The table alone, one key per line, without a trailing newline.
pub fn section(plugin_id: &str, defaults: &Defaults) -> String {
    let mut out = header(plugin_id);
    for (key, value) in values(defaults) {
        out.push('\n');
        out.push_str(&toml_string(key));
        out.push_str(" = ");
        match value {
            Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
            Value::Str(s) => out.push_str(&toml_string(&s)),
        }
    }
    out
}

/// Sets the plugin's table in `text`, keeping everything else. The second value
/// says whether the text changed.
pub fn upsert(text: &str, plugin_id: &str, defaults: &Defaults) -> (String, bool) {
    let header = header(plugin_id);
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let header_line = header.trim_end();

    let start = lines
        .iter()
        .position(|line| line.trim_end() == header_line || line.trim_end() == header_line.replace("\r", "").as_str());
    match start {
        Some(start) => {
            let end = lines[start + 1..]
                .iter()
                .position(|line| line.trim_start().starts_with('['))
                .map(|offset| start + 1 + offset)
                .unwrap_or(lines.len());
            for line in &lines[..start] {
                out.push((*line).to_string());
            }
            for line in section(plugin_id, defaults).split('\n') {
                out.push(line.to_string());
            }
            for line in &lines[end..] {
                out.push((*line).to_string());
            }
        }
        None => {
            let body = text.trim_end_matches(['\n', '\r']);
            if !body.trim().is_empty() {
                for line in body.split('\n') {
                    out.push(line.to_string());
                }
                out.push(String::new());
            }
            for line in section(plugin_id, defaults).split('\n') {
                out.push(line.to_string());
            }
        }
    }

    // One trailing newline, no trailing blank lines.
    while out.last().is_some_and(|line| line.trim_end_matches('\r').is_empty()) {
        out.pop();
    }
    let mut result = out.join(newline);
    result.push_str(newline);
    let changed = result != text;
    (result, changed)
}

/// A TOML basic string, quoted and escaped.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::Mode;
    use crate::settings::key_names;

    fn defaults() -> Defaults {
        let mut d = Defaults::schema_defaults();
        d.vm_options = vec!["-Xmx2g".to_string(), "-Dfile.encoding=UTF-8".to_string()];
        d.program_args = vec!["--server.port=8081".to_string()];
        d.spring_profiles = vec!["dev".to_string(), "local".to_string()];
        d.mode = Mode::Override;
        d
    }

    #[test]
    fn renders_the_table_the_ide_reads() {
        let text = section("acme.jvm", &defaults());
        assert_eq!(
            text,
            concat!(
                "[plugins.\"acme.jvm\"]\n",
                "\"jvmParams.enabled\" = true\n",
                "\"jvmParams.vmOptions\" = \"-Xmx2g -Dfile.encoding=UTF-8\"\n",
                "\"jvmParams.programArgs\" = \"--server.port=8081\"\n",
                "\"jvmParams.springProfiles\" = \"dev,local\"\n",
                "\"jvmParams.mode\" = \"override\"\n",
                "\"jvmParams.pinToProject\" = true",
            )
        );
    }

    #[test]
    fn appends_to_an_empty_file() {
        let (text, changed) = upsert("", "acme.jvm", &defaults());
        assert!(changed);
        assert!(text.starts_with("[plugins.\"acme.jvm\"]\n"));
        assert!(text.ends_with("true\n"));
    }

    #[test]
    fn keeps_the_other_tables_of_the_project() {
        let before = concat!(
            "[editor]\n",
            "fontSize = 15\n",
            "\n",
            "[files]\n",
            "\"fileType.bicep\" = \"bicep\"\n",
        );
        let (text, changed) = upsert(before, "acme.jvm", &defaults());
        assert!(changed);
        assert!(text.starts_with(before.trim_end_matches('\n')));
        assert!(text.contains("[plugins.\"acme.jvm\"]\n\"jvmParams.enabled\" = true"));
        assert!(text.ends_with("true\n"));
    }

    #[test]
    fn rewriting_is_idempotent() {
        let (once, _) = upsert("[editor]\nfontSize = 15\n", "acme.jvm", &defaults());
        let (twice, changed) = upsert(&once, "acme.jvm", &defaults());
        assert!(!changed);
        assert_eq!(once, twice);
    }

    #[test]
    fn a_changed_value_rewrites_only_the_plugin_table() {
        let (once, _) = upsert("[editor]\nfontSize = 15\n", "acme.jvm", &defaults());
        let mut other = defaults();
        other.vm_options = vec!["-Xmx4g".to_string()];
        let (twice, changed) = upsert(&once, "acme.jvm", &other);
        assert!(changed);
        assert!(twice.contains("\"jvmParams.vmOptions\" = \"-Xmx4g\""));
        assert!(twice.starts_with("[editor]\nfontSize = 15\n"));
        assert_eq!(twice.matches("[plugins.").count(), 1);
    }

    #[test]
    fn keeps_crlf_files_crlf() {
        let (text, _) = upsert("[editor]\r\nfontSize = 15\r\n", "acme.jvm", &defaults());
        assert!(text.contains("\r\n[plugins.\"acme.jvm\"]\r\n"));
        assert!(!text.contains("\n[plugins") || text.contains("\r\n[plugins"));
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        let mut d = Defaults::schema_defaults();
        d.vm_options = vec!["-Dpath=C:\\temp\\x".to_string()];
        let text = section("acme.jvm", &d);
        assert!(text.contains("\"jvmParams.vmOptions\" = \"-Dpath=C:\\\\temp\\\\x\""));
    }

    #[test]
    fn the_written_keys_are_the_keys_the_plugin_reads() {
        let names = key_names();
        for (key, _) in values(&Defaults::schema_defaults()) {
            assert!(
                names.contains(&key),
                "{key} is written to the project but the plugin never reads it"
            );
        }
    }
}
