//! Combining a project's existing JVM parameters with the defaults from the
//! plugin's settings.
//!
//! The interesting part is deciding which pairs of arguments are "the same
//! option": `-Xmx1g` and `-Xmx2g` are the same switch, `-Dfoo=a` and
//! `-Dfoo=b` set the same property, but `-javaagent:a.jar` and
//! `-javaagent:b.jar` are different agents and both must stay.
//!
//! Two layers merge: the parameters a run configuration already carries, and the
//! plugin's defaults. As "defaults" they only fill gaps — an option that is
//! already set is left exactly as it is, unless the user picked `override`.
//! Arguments keep their relative order and the base keeps its positions.

use std::collections::HashMap;

/// What to do when the defaults collide with parameters that already exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Keep every existing parameter and append only the missing defaults.
    #[default]
    Append,
    /// Replace existing values of the defaulted options; keep the rest.
    Override,
}

impl Mode {
    /// Parses the setting value; unknown values fall back to `Append`.
    pub fn parse(value: &str) -> Mode {
        match value.trim().to_ascii_lowercase().as_str() {
            "override" | "replace" | "overwrite" => Mode::Override,
            _ => Mode::Append,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Append => "append",
            Mode::Override => "override",
        }
    }
}

/// Returns the identity of an argument for de-duplication purposes.
///
/// `None` means "never de-duplicate this argument" (arguments that take their
/// value as the next token, such as `-cp`/`-jar`, and non-option tokens).
pub fn option_key(arg: &str) -> Option<String> {
    if !arg.starts_with('-') || arg == "-" || arg == "--" {
        return None;
    }
    // Arguments whose value is the next token must not be collapsed: dropping
    // one of them would leave its value dangling.
    const VALUE_FOLLOWS: &[&str] = &[
        "-jar",
        "-cp",
        "-classpath",
        "-p",
        "--class-path",
        "--module-path",
        "--module",
        "-m",
        "--source",
    ];
    if VALUE_FOLLOWS.contains(&arg) {
        return None;
    }
    if let Some(rest) = arg.strip_prefix("-D") {
        if rest.is_empty() {
            return None;
        }
        let key = rest.split('=').next().unwrap_or(rest);
        return Some(format!("-D{key}"));
    }
    if let Some(rest) = arg.strip_prefix("-XX:") {
        if rest.is_empty() {
            return None;
        }
        // `-XX:+Flag` and `-XX:-Flag` toggle the same flag; `-XX:Name=value`
        // and `-XX:Name` name the same option.
        let body = rest
            .strip_prefix('+')
            .or_else(|| rest.strip_prefix('-'))
            .unwrap_or(rest);
        let key = body.split('=').next().unwrap_or(body);
        if key.is_empty() {
            return None;
        }
        return Some(format!("-XX:{key}"));
    }
    // `-Xmx1g` → `-Xmx`, `-Xss8m` → `-Xss`, `-Xint` → `-Xint`,
    // `-Xshare:auto` → `-Xshare`.
    if let Some(rest) = arg.strip_prefix("-X") {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        if !name.is_empty() {
            return Some(format!("-X{name}"));
        }
        return None;
    }
    // Everything else (including `-javaagent:…`, `-agentlib:…`, `-ea`,
    // `--add-opens=…`) is identified by its full text.
    Some(arg.to_string())
}

/// Merges `extra` into `base` according to `mode`.
///
/// `Append` is the "defaults" behavior: every argument that is already there is
/// kept, and `extra` only contributes options that are missing. `Override`
/// replaces the value of an option in place, or appends it when it is new.
pub fn merge_args(base: &[String], extra: &[String], mode: Mode) -> Vec<String> {
    match mode {
        Mode::Append => {
            let mut out: Vec<String> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            for arg in base {
                if let Some(key) = option_key(arg) {
                    seen.push(key);
                }
                out.push(arg.clone());
            }
            for arg in extra {
                match option_key(arg) {
                    // A default never overrides a value the configuration
                    // already carries: `-Dprofile=prod` in a run configuration
                    // must not be silently replaced by a `dev` default.
                    Some(key) if seen.contains(&key) => continue,
                    Some(key) => {
                        seen.push(key);
                        out.push(arg.clone());
                    }
                    None => out.push(arg.clone()),
                }
            }
            out
        }
        Mode::Override => {
            let mut out = base.to_vec();
            let mut positions: HashMap<String, usize> = HashMap::new();
            for (index, arg) in out.iter().enumerate() {
                if let Some(key) = option_key(arg) {
                    positions.entry(key).or_insert(index);
                }
            }
            for arg in extra {
                match option_key(arg) {
                    Some(key) => match positions.get(&key) {
                        Some(&index) => out[index] = arg.clone(),
                        None => {
                            positions.insert(key, out.len());
                            out.push(arg.clone());
                        }
                    },
                    None => out.push(arg.clone()),
                }
            }
            out
        }
    }
}

/// Removes repeated options from `args`, keeping the last value of each.
pub fn dedupe_args(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    for arg in args {
        match option_key(arg) {
            Some(key) => match positions.get(&key) {
                Some(&index) => out[index] = arg.clone(),
                None => {
                    positions.insert(key, out.len());
                    out.push(arg.clone());
                }
            },
            None => out.push(arg.clone()),
        }
    }
    out
}

/// Splits the JVM properties out of a list, for previews: the `-D…` entries.
pub fn properties(args: &[String]) -> Vec<(String, String)> {
    args.iter()
        .filter_map(|arg| {
            let rest = arg.strip_prefix("-D")?;
            if rest.is_empty() {
                return None;
            }
            let (key, value) = match rest.split_once('=') {
                Some((k, v)) => (k, v),
                None => (rest, ""),
            };
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn identifies_heap_switches() {
        assert_eq!(option_key("-Xmx1g").as_deref(), Some("-Xmx"));
        assert_eq!(option_key("-Xms512m").as_deref(), Some("-Xms"));
        assert_eq!(option_key("-Xss8m").as_deref(), Some("-Xss"));
        assert_eq!(option_key("-Xint").as_deref(), Some("-Xint"));
        assert_eq!(option_key("-Xshare:auto").as_deref(), Some("-Xshare"));
    }

    #[test]
    fn identifies_properties_and_boolean_flags() {
        assert_eq!(option_key("-Dfoo=bar").as_deref(), Some("-Dfoo"));
        assert_eq!(option_key("-Dfoo").as_deref(), Some("-Dfoo"));
        assert_eq!(option_key("-XX:+UseG1GC").as_deref(), Some("-XX:UseG1GC"));
        assert_eq!(option_key("-XX:-UseG1GC").as_deref(), Some("-XX:UseG1GC"));
        assert_eq!(
            option_key("-XX:MaxGCPauseMillis=200").as_deref(),
            Some("-XX:MaxGCPauseMillis")
        );
    }

    #[test]
    fn leaves_agents_and_value_following_options_alone() {
        assert_eq!(
            option_key("-javaagent:/a.jar").as_deref(),
            Some("-javaagent:/a.jar")
        );
        assert_ne!(option_key("-javaagent:/a.jar"), option_key("-javaagent:/b.jar"));
        assert_eq!(option_key("-cp"), None);
        assert_eq!(option_key("-jar"), None);
        assert_eq!(option_key("com.example.Main"), None);
    }

    #[test]
    fn append_fills_gaps_and_keeps_explicit_values() {
        let base = v(&["-Xmx1g", "-Dfoo=a", "-ea"]);
        let extra = v(&["-Xmx2g", "-Dbar=b"]);
        assert_eq!(
            merge_args(&base, &extra, Mode::Append),
            v(&["-Xmx1g", "-Dfoo=a", "-ea", "-Dbar=b"])
        );
    }

    #[test]
    fn override_replaces_colliding_values_in_place() {
        let base = v(&["-Xmx1g", "-Dfoo=a"]);
        let extra = v(&["-Xmx2g", "-Dbar=b"]);
        assert_eq!(
            merge_args(&base, &extra, Mode::Override),
            v(&["-Xmx2g", "-Dfoo=a", "-Dbar=b"])
        );
        assert!(merge_args(&base, &[], Mode::Override) == base);
    }

    #[test]
    fn merging_an_empty_default_is_a_no_op() {
        let base = v(&["-Xmx1g"]);
        assert_eq!(merge_args(&base, &[], Mode::Append), base);
        assert!(merge_args(&[], &[], Mode::Append).is_empty());
    }

    #[test]
    fn dedupes_repeated_flags_keeping_the_last_value() {
        assert_eq!(
            dedupe_args(&v(&["-Xmx1g", "-ea", "-Xmx2g", "-ea"])),
            v(&["-Xmx2g", "-ea"])
        );
    }

    #[test]
    fn keeps_both_java_agents() {
        let args = v(&["-javaagent:jmx.jar", "-javaagent:otel.jar"]);
        assert_eq!(dedupe_args(&args), args);
    }

    #[test]
    fn parses_mode() {
        assert_eq!(Mode::parse("override"), Mode::Override);
        assert_eq!(Mode::parse(" REPLACE "), Mode::Override);
        assert_eq!(Mode::parse("append"), Mode::Append);
        assert_eq!(Mode::parse("nonsense"), Mode::Append);
        assert_eq!(Mode::default(), Mode::Append);
        assert_eq!(Mode::Override.as_str(), "override");
    }

    #[test]
    fn extracts_properties_for_previews() {
        let args = v(&["-Xmx1g", "-Dfoo=bar", "-Dbaz"]);
        assert_eq!(
            properties(&args),
            vec![
                ("foo".to_string(), "bar".to_string()),
                ("baz".to_string(), String::new())
            ]
        );
    }
}
