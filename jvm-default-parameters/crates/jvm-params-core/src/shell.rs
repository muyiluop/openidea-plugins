//! Splitting and joining of JVM command-line parameter strings.
//!
//! JVM options are usually written as one flat string (`-Xmx1g -Dfoo="a b"`), in
//! JetBrains run configurations (`<option name="VM_PARAMETERS" value="…"/>`), in
//! VS Code `launch.json` (`"vmArgs": "-Xmx1g -Dfoo=bar"`) and in plugin settings.
//! Splitting has to follow the platform convention closely enough that a value
//! written for one of those places still works when it is handed to a process:
//!
//! * whitespace separates tokens outside quotes;
//! * `"` quotes a run and inside it `\"` and `\\` are escapes;
//! * `'` quotes a run and inside it nothing is an escape;
//! * outside quotes a backslash is a literal character. This is deliberate:
//!   `-Dlog.dir=C:\logs` must survive unchanged on Windows, where the backslash
//!   is a path separator and not an escape.
//!
//! Neither function expands environment variables or globs.

/// A quote problem found by [`try_split_args`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitError {
    /// Human readable reason, suitable for a warning line in the IDE output.
    pub message: String,
    /// Byte offset in the input where the problem starts.
    pub position: usize,
}

impl SplitError {
    fn new(message: impl Into<String>, position: usize) -> Self {
        Self {
            message: message.into(),
            position,
        }
    }
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.position)
    }
}

impl std::error::Error for SplitError {}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Splits `input` into arguments, reporting unterminated quotes as an error.
///
/// Use this for user-entered settings, where a broken quote is a mistake worth
/// surfacing.
pub fn try_split_args(input: &str) -> Result<Vec<String>, SplitError> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut in_double = false;
    let mut in_single = false;
    let mut quote_start = 0usize;

    let mut chars = input.char_indices().peekable();
    while let Some((idx, c)) = chars.next() {
        if in_double {
            match c {
                '\\' => match chars.peek().map(|(_, c)| *c) {
                    Some('"') | Some('\\') => {
                        let (_, escaped) = chars.next().expect("peeked");
                        current.push(escaped);
                    }
                    _ => current.push('\\'),
                },
                '"' => in_double = false,
                _ => current.push(c),
            }
            continue;
        }
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                in_double = true;
                in_token = true;
                quote_start = idx;
            }
            '\'' => {
                in_single = true;
                in_token = true;
                quote_start = idx;
            }
            c if is_space(c) => {
                if in_token {
                    args.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            _ => {
                current.push(c);
                in_token = true;
            }
        }
    }

    if in_double || in_single {
        return Err(SplitError::new(
            if in_double {
                "unterminated double quote"
            } else {
                "unterminated single quote"
            },
            quote_start,
        ));
    }
    if in_token {
        args.push(current);
    }
    Ok(args)
}

/// Splits `input` into arguments, tolerating a missing closing quote: the
/// remainder of the string becomes part of the current argument.
///
/// Use this when reading configuration files that may contain hand-written or
/// truncated values; a malformed quote there should not drop the whole launch.
pub fn split_args(input: &str) -> Vec<String> {
    try_split_args(input).unwrap_or_else(|_| {
        // Re-run in "lenient" mode: treat an unterminated quote as one that
        // closes at the end of the input.
        let trimmed = input.trim_end();
        let closes_double = trimmed.matches('"').count() % 2 == 1;
        let closes_single = trimmed.matches('\'').count() % 2 == 1;
        let mut patched = String::from(trimmed);
        if closes_double {
            patched.push('"');
        }
        if closes_single {
            patched.push('\'');
        }
        try_split_args(&patched).unwrap_or_default()
    })
}

/// Joins arguments into one string, quoting only the arguments that need it.
///
/// The result can be split again by [`try_split_args`] on the same platform
/// rules; it is what we show in previews and put into `-D…jvmArguments="…"`.
pub fn join_args(args: &[String]) -> String {
    let mut out = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&quote_arg(arg));
    }
    out
}

/// Quotes one argument if it contains whitespace, quotes or is empty.
pub fn quote_arg(arg: &str) -> String {
    let needs_quotes = arg.is_empty()
        || arg
            .chars()
            .any(|c| is_space(c) || c == '"' || c == '\'');
    if !needs_quotes {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether a parameter string is empty once trimmed.
pub fn is_blank(input: &str) -> bool {
    input.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_parameters() {
        assert_eq!(
            try_split_args("-Xmx1g -Dfoo=bar -Dspring.profiles.active=dev").unwrap(),
            vec!["-Xmx1g", "-Dfoo=bar", "-Dspring.profiles.active=dev"]
        );
    }

    #[test]
    fn collapses_whitespace_and_newlines() {
        assert_eq!(
            try_split_args("  -Xmx1g\n\t-Dfoo=bar  \r\n ").unwrap(),
            vec!["-Xmx1g", "-Dfoo=bar"]
        );
        assert!(try_split_args("   ").unwrap().is_empty());
        assert!(try_split_args("").unwrap().is_empty());
    }

    #[test]
    fn keeps_double_quoted_runs_together() {
        // Quotes group characters and are not part of the argument, as in a shell.
        assert_eq!(
            try_split_args(r#"-Dfoo="a b" -Dbar=c"#).unwrap(),
            vec!["-Dfoo=a b", "-Dbar=c"]
        );
        assert_eq!(
            try_split_args(r#""-Xmx1g -Xms512m""#).unwrap(),
            vec!["-Xmx1g -Xms512m"]
        );
    }

    #[test]
    fn keeps_single_quoted_runs_together() {
        assert_eq!(
            try_split_args("-Dfoo='a b' -ea").unwrap(),
            vec!["-Dfoo=a b", "-ea"]
        );
    }

    #[test]
    fn keeps_windows_paths_literal() {
        assert_eq!(
            try_split_args(r"-Dlog.dir=C:\logs\app -Dcache=C:\temp\cache").unwrap(),
            vec![r"-Dlog.dir=C:\logs\app", r"-Dcache=C:\temp\cache"]
        );
    }

    #[test]
    fn keeps_jvm_arguments_quoted_as_a_block() {
        assert_eq!(
            try_split_args(r#""-Xmx2g -XX:+UseG1GC" -Dfoo=bar"#).unwrap(),
            vec!["-Xmx2g -XX:+UseG1GC", "-Dfoo=bar"]
        );
    }

    #[test]
    fn keeps_quoted_value_and_empty_quoted_token() {
        assert_eq!(
            try_split_args(r#"-Djava.util.logging.manager="x""#).unwrap(),
            vec!["-Djava.util.logging.manager=x"]
        );
        assert_eq!(try_split_args(r#""""#).unwrap(), vec![""]);
    }

    #[test]
    fn reports_unterminated_quotes_strictly() {
        let err = try_split_args("-Xmx1g -Dfoo=\"bar").unwrap_err();
        assert_eq!(err.message, "unterminated double quote");
        assert_eq!(err.position, 13);
        let err = try_split_args("-Dfoo='bar").unwrap_err();
        assert_eq!(err.message, "unterminated single quote");
    }

    #[test]
    fn lenient_split_recovers_unterminated_quotes() {
        assert_eq!(split_args("-Xmx1g -Dfoo=\"bar"), vec!["-Xmx1g", "-Dfoo=bar"]);
        assert_eq!(split_args("-Dfoo=bar'"), vec!["-Dfoo=bar"]);
        assert!(split_args("").is_empty());
    }

    #[test]
    fn joins_and_round_trips() {
        let args = vec![
            "-Xmx1g".to_string(),
            "-Dfoo=bar".to_string(),
            "-Dmessage=hello world".to_string(),
            "-Dquote=a\"b".to_string(),
            String::new(),
        ];
        let joined = join_args(&args);
        assert_eq!(joined, r#"-Xmx1g -Dfoo=bar "-Dmessage=hello world" "-Dquote=a\"b" """#);
        assert_eq!(try_split_args(&joined).unwrap(), args);
    }

    #[test]
    fn join_of_empty_is_empty() {
        assert_eq!(join_args(&[]), "");
        assert!(is_blank("  \n"));
        assert!(!is_blank("-ea"));
    }
}
