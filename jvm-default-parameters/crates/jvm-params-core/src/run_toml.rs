//! `.openidea/run.toml` — the launch configurations the IDE lists.
//!
//! The IDE materialises its Spring Boot entries on every listing and drops any
//! detected configuration whose id is already saved in `.openidea/run.toml`
//! (`crates/ide-host/src/ops/run.rs::detected_and_saved`), so saving a
//! configuration with the **same id** is what adds VM options to the entry the
//! user picks in the run dialog.
//!
//! A Java configuration carries its command line in `command` + `args`, exactly
//! like the IDE's own JetBrains importer writes them:
//!
//! ```text
//! java [VM options] -cp ${classpath} <main class> [--spring.profiles.active=…] [program arguments]
//! ```
//!
//! `src/features/run/jvmTypes.ts` parses those `args` back into the form fields,
//! and `src/features/run/store.ts` resolves `${classpath}` against the module
//! named by `module` before launching, so no classpath has to be computed here.

use crate::merge::merge_args;
use crate::settings::Defaults;
use crate::shell::{join_args, split_args};
use crate::springboot::App;
use std::collections::BTreeMap;

/// Path of the file, relative to the project root.
pub const RUN_TOML: &str = ".openidea/run.toml";

/// The classpath placeholder the IDE replaces with the module's runtime classpath.
pub const CLASSPATH: &str = "${classpath}";

/// The kind the IDE gives the Spring Boot configurations it detects.
pub const KIND: &str = "spring-boot";

/// The debug adapter of the Java configurations.
pub const DEBUG_ADAPTER: &str = "java-debug";

/// One `[[configurations]]` table this plugin owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Owning module, relative to the project root; empty for the root build.
    pub module: String,
    pub command: String,
    /// `[VM options…, "-cp", "${classpath}", main class, program arguments…]`.
    pub args: Vec<String>,
    pub debug_adapter: Option<String>,
}

impl Entry {
    /// The table as TOML text, starting at the header line and ending with a newline.
    pub fn render(&self) -> String {
        let mut out = String::from("[[configurations]]\n");
        out.push_str(&format!("id = {}\n", quote(&self.id)));
        out.push_str(&format!("name = {}\n", quote(&self.name)));
        out.push_str(&format!("kind = {}\n", quote(&self.kind)));
        if !self.module.is_empty() {
            out.push_str(&format!("module = {}\n", quote(&self.module)));
        }
        out.push_str(&format!("command = {}\n", quote(&self.command)));
        if let Some(adapter) = &self.debug_adapter {
            out.push_str(&format!("debug-adapter = {}\n", quote(adapter)));
        }
        out.push_str(&format!("args = {}\n", array(&self.args)));
        out
    }

    /// The main class of the entry, read back from its command line.
    pub fn main_class(&self) -> String {
        if let Some(cp) = self.args.iter().position(|arg| arg == "-cp") {
            if let Some(main) = self.args.get(cp + 2) {
                return main.clone();
            }
        }
        String::new()
    }
}

/// The entries for the detected applications, with the configured parameters.
pub fn entries(apps: &[App], defaults: &Defaults) -> Vec<Entry> {
    let mut out = Vec::new();
    for app in apps {
        let mut args = defaults.effective_vm_args();
        args.push("-cp".to_string());
        args.push(CLASSPATH.to_string());
        args.push(app.main_class.clone());
        if !defaults.spring_profiles.is_empty() {
            args.push(format!("--spring.profiles.active={}", defaults.spring_profiles.join(",")));
        }
        args.extend(defaults.program_args.iter().cloned());
        out.push(Entry {
            id: app.id.clone(),
            name: app.name.clone(),
            kind: KIND.to_string(),
            module: app.module.clone(),
            command: "java".to_string(),
            args,
            debug_adapter: Some(DEBUG_ADAPTER.to_string()),
        });
    }
    out
}

/// Merges `entries` into the text of an existing file: a table whose `id` matches
/// keeps the fields this plugin does not own (environment, working directory,
/// before-launch tasks, …) and gets its command line merged, every other table is
/// left exactly as it is, new tables are appended. Returns the new text and how
/// many tables were added or changed.
pub fn upsert(text: &str, entries: &[Entry], defaults: &Defaults) -> (String, usize) {
    let mut blocks = blocks(text);
    let mut changed = 0usize;
    let mut appended: Vec<String> = Vec::new();

    for entry in entries {
        match blocks.iter().position(|block| block.id.as_deref() == Some(entry.id.as_str())) {
            Some(index) => {
                let merged = merge_block(&blocks[index], entry, defaults);
                if merged != blocks[index].text {
                    changed += 1;
                    blocks[index].text = merged;
                }
            }
            None => {
                changed += 1;
                appended.push(entry.render());
            }
        }
    }

    let mut out = String::new();
    let mut written = 0usize;
    for block in blocks.into_iter() {
        let mut body = block.text;
        // One blank line of separation between tables, none at the end: that is
        // what makes a second pass see the file it wrote.
        while body.ends_with("\n\n") {
            body.pop();
        }
        if body.trim().is_empty() {
            continue;
        }
        if written > 0 {
            out.push('\n');
        }
        out.push_str(&body);
        written += 1;
    }
    for block in appended {
        if written > 0 {
            out.push('\n');
        }
        out.push_str(&block);
        written += 1;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    (out, changed)
}

/// One `[[configurations]]` table inside the file.
struct Block {
    text: String,
    id: Option<String>,
}

/// Splits the file into tables: each `[[configurations]]` block with everything
/// that follows it, and a leading block holding the header of the file (comments,
/// other tables) which is never rewritten.
fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut header = String::new();
    for line in text.split_inclusive('\n') {
        if line.trim() == "[[configurations]]" {
            out.push(Block { text: line.to_string(), id: None });
            continue;
        }
        match out.last_mut() {
            Some(block) => {
                block.text.push_str(line);
                if block.id.is_none() && line.trim_start().starts_with('i') {
                    block.id = string_value(line);
                }
            }
            None => header.push_str(line),
        }
    }
    if !header.is_empty() {
        out.insert(0, Block { text: header, id: None });
    }
    // The blank line that separates two tables belongs to the separation, not to
    // the table: keeping it here would make a rewrite of the table look like a
    // change on every pass.
    for block in &mut out {
        while block.text.ends_with("\n\n") {
            block.text.pop();
        }
    }
    out
}

/// Keeps the fields of the existing table that this plugin does not own, and
/// rewrites the command line with the configured parameters merged in.
fn merge_block(block: &Block, entry: &Entry, defaults: &Defaults) -> String {
    let existing = values(&block.text);
    let existing_args = array_value(existing.get("args")).unwrap_or_default();
    let (existing_program_args, existing_vm) = split_command_args(&existing_args);
    let classpath = classpath_of(&existing_args).unwrap_or_else(|| CLASSPATH.to_string());

    let mut args = merge_args(&existing_vm, &defaults.effective_vm_args(), defaults.mode);
    args.push("-cp".to_string());
    args.push(classpath);
    args.push(entry.main_class());
    // The profile argument comes from the settings; without one the profile the
    // configuration already carries stays.
    let existing_profiles: Vec<&String> = existing_program_args
        .iter()
        .filter(|arg| arg.starts_with("--spring.profiles.active="))
        .collect();
    if !defaults.spring_profiles.is_empty() {
        args.push(format!("--spring.profiles.active={}", defaults.spring_profiles.join(",")));
    } else {
        args.extend(existing_profiles.iter().map(|arg| (*arg).clone()));
    }
    for arg in &defaults.program_args {
        if !args.contains(arg) {
            args.push(arg.clone());
        }
    }
    for arg in &existing_program_args {
        if !existing_profiles.contains(&arg) && !args.contains(arg) {
            args.push(arg.clone());
        }
    }

    let owned = ["id", "name", "kind", "module", "command", "debug-adapter", "args"];
    let mut out = String::from("[[configurations]]\n");
    out.push_str(&format!("id = {}\n", quote(&entry.id)));
    out.push_str(&format!("name = {}\n", quote(&entry.name)));
    out.push_str(&format!("kind = {}\n", quote(&entry.kind)));
    if !entry.module.is_empty() {
        out.push_str(&format!("module = {}\n", quote(&entry.module)));
    }
    out.push_str(&format!("command = {}\n", quote(&entry.command)));
    if let Some(adapter) = &entry.debug_adapter {
        out.push_str(&format!("debug-adapter = {}\n", quote(adapter)));
    }
    out.push_str(&format!("args = {}\n", array(&args)));
    for line in block.text.lines().skip(1) {
        let key = line.split('=').next().unwrap_or_default().trim();
        if !owned.contains(&key) && !line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The simple `key = value` fields of a table, as raw text.
fn values(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines().skip(1) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.starts_with('#') {
            continue;
        }
        out.insert(key.to_string(), value.trim().to_string());
    }
    out
}

/// The VM options and the program arguments of a Java command line, i.e. what is
/// around `-cp ${classpath} <main class>`.
fn split_command_args(args: &[String]) -> (Vec<String>, Vec<String>) {
    let Some(cp) = args.iter().position(|arg| arg == "-cp" || arg == "-classpath" || arg == "--class-path") else {
        return (Vec::new(), args.to_vec());
    };
    let vm = args[..cp].to_vec();
    let main = cp + 2;
    let mut rest = Vec::new();
    if main + 1 < args.len() {
        rest.extend_from_slice(&args[main + 1..]);
    }
    (rest, vm)
}

/// The classpath token of a Java command line, when it is an explicit one.
fn classpath_of(args: &[String]) -> Option<String> {
    let cp = args.iter().position(|arg| arg == "-cp" || arg == "-classpath" || arg == "--class-path")?;
    args.get(cp + 1).cloned()
}

/// A TOML array of strings.
fn array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|value| quote(value)).collect();
    format!("[{}]", items.join(", "))
}

/// The strings of a TOML array of strings.
fn array_value(raw: Option<&String>) -> Option<Vec<String>> {
    let raw = raw?.trim();
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
    let mut out = Vec::new();
    let mut rest = inner;
    loop {
        let Some(at) = rest.find('"') else {
            break;
        };
        let (value, used) = parse_quoted(&rest[at..])?;
        out.push(value);
        rest = &rest[at + used..];
        rest = rest.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
        if rest.is_empty() {
            break;
        }
    }
    Some(out)
}

/// A TOML string literal at the start of `text`: the value and how many bytes it
/// took (surrounding whitespace is ignored).
fn string_value(text: &str) -> Option<String> {
    let at = text.find('"')?;
    parse_quoted(&text[at..]).map(|(value, _)| value)
}

/// Parses one `"…"` literal: the value and the number of bytes consumed.
fn parse_quoted(text: &str) -> Option<(String, usize)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((index, c)) = chars.next() {
        match c {
            '"' => return Some((out, index + 2)),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).map(|(_, c)| c).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// A TOML string literal for `value`.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The arguments of a configuration as a single line, for messages.
pub fn args_line(args: &[String]) -> String {
    join_args(args)
}

/// The arguments of a configuration, read back from its line.
pub fn args_of(line: &str) -> Vec<String> {
    split_args(line)
}

/// A Spring Boot application as the IDE detects it, for tests and previews.
pub fn example_app() -> App {
    App {
        id: "application:attach-dma:com.attach.dma.AttachDmaApplication".to_string(),
        name: "AttachDmaApplication".to_string(),
        module: "attach-dma".to_string(),
        main_class: "com.attach.dma.AttachDmaApplication".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::Mode;

    fn app() -> App {
        example_app()
    }

    fn defaults() -> Defaults {
        let mut defaults = Defaults::schema_defaults();
        defaults.vm_options = vec!["-Xmx2g".to_string(), "-Dnacos.namespace=new_dev_local".to_string()];
        defaults.program_args = vec!["--server.port=8081".to_string()];
        defaults
    }

    #[test]
    fn renders_a_configuration_the_ide_will_list() {
        let entries = entries(&[app()], &defaults());
        let text = entries[0].render();
        assert!(text.starts_with("[[configurations]]\n"), "{text}");
        assert!(text.contains("id = \"application:attach-dma:com.attach.dma.AttachDmaApplication\""), "{text}");
        assert!(text.contains("name = \"AttachDmaApplication\""), "{text}");
        assert!(text.contains("kind = \"spring-boot\""), "{text}");
        assert!(text.contains("module = \"attach-dma\""), "{text}");
        assert!(text.contains("command = \"java\""), "{text}");
        assert!(text.contains("debug-adapter = \"java-debug\""), "{text}");
        assert!(
            text.contains(
                "args = [\"-Xmx2g\", \"-Dnacos.namespace=new_dev_local\", \"-cp\", \"${classpath}\", \"com.attach.dma.AttachDmaApplication\", \"--server.port=8081\"]"
            ),
            "{text}"
        );
    }

    #[test]
    fn appends_to_an_empty_file() {
        let (text, changed) = upsert("", &entries(&[app()], &defaults()), &defaults());
        assert_eq!(changed, 1);
        assert!(text.starts_with("[[configurations]]\n"), "{text}");
    }

    #[test]
    fn keeps_the_configurations_the_user_wrote() {
        let existing = "[[configurations]]\nid = \"user:migrate\"\nname = \"Migrate\"\nkind = \"shell\"\ncommand = \"mvn\"\nargs = [\"flyway:migrate\"]\n";
        let (text, changed) = upsert(existing, &entries(&[app()], &defaults()), &defaults());
        assert_eq!(changed, 1);
        assert!(text.contains("id = \"user:migrate\""), "{text}");
        assert!(text.contains("args = [\"flyway:migrate\"]"), "{text}");
    }

    #[test]
    fn rewriting_is_idempotent() {
        let defaults = defaults();
        let (first, _) = upsert("", &entries(&[app()], &defaults), &defaults);
        let (second, changed) = upsert(&first, &entries(&[app()], &defaults), &defaults);
        assert_eq!(changed, 0, "second pass must not change anything:\n{second}");
        assert_eq!(first, second);
    }

    #[test]
    fn many_configurations_keep_the_same_separation_on_a_second_pass() {
        let defaults = defaults();
        let mut second_app = app();
        second_app.id = "application:attach-auth:com.attach.auth.AttachAuthApplication".to_string();
        second_app.name = "AttachAuthApplication".to_string();
        second_app.module = "attach-auth".to_string();
        second_app.main_class = "com.attach.auth.AttachAuthApplication".to_string();
        let apps = vec![app(), second_app];
        let (first, added) = upsert("", &entries(&apps, &defaults), &defaults);
        assert_eq!(added, 2, "{first}");
        assert_eq!(first.matches("[[configurations]]").count(), 2);
        let (second, changed) = upsert(&first, &entries(&apps, &defaults), &defaults);
        assert_eq!(changed, 0, "second pass changed the file:\n{second}");
        assert_eq!(first, second);
        assert!(first.contains("\n\n[[configurations]]"), "one blank line between tables:\n{first}");
        assert!(!first.ends_with("\n\n"), "no blank line at the end:\n{first}");
    }

    #[test]
    fn an_existing_configuration_keeps_its_own_vm_options_and_extra_fields() {
        let existing = "[[configurations]]\nid = \"application:attach-dma:com.attach.dma.AttachDmaApplication\"\nname = \"AttachDmaApplication\"\nkind = \"spring-boot\"\nmodule = \"attach-dma\"\ncommand = \"java\"\ndebug-adapter = \"java-debug\"\nargs = [\"-Xmx4g\", \"-cp\", \"${classpath}\", \"com.attach.dma.AttachDmaApplication\", \"--spring.profiles.active=dev\"]\nenv = { A = \"1\" }\n";
        let (text, changed) = upsert(existing, &entries(&[app()], &defaults()), &defaults());
        assert_eq!(changed, 1);
        assert!(text.contains("\"-Xmx4g\""), "the configuration's own value stays: {text}");
        assert!(!text.contains("\"-Xmx2g\""), "{text}");
        assert!(text.contains("\"-Dnacos.namespace=new_dev_local\""), "the missing option is added: {text}");
        assert!(text.contains("\"--server.port=8081\""), "{text}");
        assert!(text.contains("--spring.profiles.active=dev"), "the profile argument stays: {text}");
        assert!(text.contains("env = { A = \"1\" }"), "unknown fields stay: {text}");
    }

    #[test]
    fn override_mode_replaces_the_vm_options_in_place() {
        let mut defaults = defaults();
        defaults.mode = Mode::Override;
        let existing = "[[configurations]]\nid = \"application:attach-dma:com.attach.dma.AttachDmaApplication\"\nname = \"AttachDmaApplication\"\nkind = \"spring-boot\"\nmodule = \"attach-dma\"\ncommand = \"java\"\nargs = [\"-Xmx4g\", \"-cp\", \"${classpath}\", \"com.attach.dma.AttachDmaApplication\"]\n";
        let (text, _) = upsert(existing, &entries(&[app()], &defaults), &defaults);
        assert!(text.contains("\"-Xmx2g\""), "{text}");
        assert!(!text.contains("\"-Xmx4g\""), "{text}");
        assert!(text.contains("${classpath}"), "{text}");
    }

    #[test]
    fn quotes_survive_a_round_trip() {
        let mut defaults = defaults();
        defaults.vm_options = vec!["-Dname=a b".to_string(), r"-Dpath=C:\temp\x".to_string()];
        let (text, _) = upsert("", &entries(&[app()], &defaults), &defaults);
        let map = values(&text);
        let args = array_value(map.get("args")).expect("args");
        assert_eq!(
            args,
            vec![
                "-Dname=a b",
                r"-Dpath=C:\temp\x",
                "-cp",
                "${classpath}",
                "com.attach.dma.AttachDmaApplication",
                "--server.port=8081",
            ]
        );
        assert_eq!(string_value(&quote("a\"b\\c")).as_deref(), Some("a\"b\\c"));
    }

    #[test]
    fn reads_a_command_line_back_into_its_parts() {
        let args = vec![
            "-Xmx2g".to_string(),
            "-cp".to_string(),
            "${classpath}".to_string(),
            "com.attach.dma.AttachDmaApplication".to_string(),
            "--server.port=8081".to_string(),
        ];
        let (program, vm) = split_command_args(&args);
        assert_eq!(vm, vec!["-Xmx2g"]);
        assert_eq!(program, vec!["--server.port=8081"]);
        assert_eq!(classpath_of(&args).as_deref(), Some("${classpath}"));
        assert_eq!(
            args_line(&args),
            "-Xmx2g -cp ${classpath} com.attach.dma.AttachDmaApplication --server.port=8081"
        );
    }
}
