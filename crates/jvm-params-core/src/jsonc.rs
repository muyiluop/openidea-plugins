//! Lenient JSON-with-comments support for `launch.json` and `.vscode` files.
//!
//! VS Code accepts comments and trailing commas in its JSON files, and people
//! use both. `serde_json` does not, so we strip them (without touching string
//! contents) before parsing, and can put them back only if the caller asks for
//! it — we always write plain JSON.

/// Removes `//` and `/* */` comments and trailing commas from `input`.
pub fn strip(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some((_, c)) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' => match chars.peek().map(|(_, c)| *c) {
                Some('/') => {
                    chars.next();
                    while let Some((_, c)) = chars.peek().copied() {
                        if c == '\n' {
                            break;
                        }
                        chars.next();
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for (_, c) in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        prev = c;
                    }
                    out.push(' ');
                }
                _ => out.push(c),
            },
            ',' => {
                // Drop the comma if the next significant character closes a
                // container.
                let mut lookahead = chars.clone();
                let mut keep = true;
                while let Some((_, next)) = lookahead.next() {
                    if next.is_whitespace() {
                        continue;
                    }
                    if next == '}' || next == ']' {
                        keep = false;
                    }
                    break;
                }
                if keep {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Parses JSON that may contain comments and trailing commas.
pub fn parse(input: &str) -> Result<serde_json::Value, serde_json::Error> {
    match serde_json::from_str(input) {
        Ok(value) => Ok(value),
        Err(first) => serde_json::from_str(&strip(input)).map_err(|_| first),
    }
}

/// Serializes a value as pretty JSON with a trailing newline.
pub fn to_writer_string(value: &serde_json::Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let value = parse(r#"{"a":1}"#).unwrap();
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn strips_line_and_block_comments() {
        let text = r#"{
            // a line comment
            "a": 1, /* block
            comment */ "b": "http://example.com//x"
        }"#;
        let value = parse(text).unwrap();
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"], "http://example.com//x");
    }

    #[test]
    fn strips_trailing_commas() {
        let value = parse(r#"{"a": [1, 2,], "b": {"c": 3,},}"#).unwrap();
        assert_eq!(value["a"][1], 2);
        assert_eq!(value["b"]["c"], 3);
    }

    #[test]
    fn keeps_escaped_quotes_inside_strings() {
        let value = parse(r#"{"a": "he said \"hi\", // not a comment"}"#).unwrap();
        assert_eq!(value["a"], "he said \"hi\", // not a comment");
    }

    #[test]
    fn broken_json_is_an_error() {
        assert!(parse(r#"{"a": }"#).is_err());
    }

    #[test]
    fn writes_pretty_json_with_newline() {
        let value = serde_json::json!({"a": 1});
        assert_eq!(to_writer_string(&value), "{\n  \"a\": 1\n}\n");
    }
}
