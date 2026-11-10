//! Reading and patching JetBrains run configuration XML.
//!
//! JetBrains IDEs keep run configurations in `.run/*.run.xml`,
//! `.idea/runConfigurations/*.xml` and inside `.idea/workspace.xml`; OpenIDEA
//! reads those formats natively. A plugin cannot mutate the IDE's in-memory run
//! configurations, so the way to give an existing `Application` configuration
//! default JVM options is to edit the XML the IDE reads — this module parses it
//! leniently and can splice new `VM_PARAMETERS` values back in without touching
//! anything else in the file (comments, ordering and unknown options survive).
//!
//! The parser is intentionally hand-written and forgiving: these files are
//! edited by IDEs and humans, may contain comments, entity escapes, attributes
//! in any order, self-closing tags and unknown elements.

use crate::shell;

/// Types of run configuration that carry JVM options in `VM_PARAMETERS`.
pub const JAVA_CONFIG_TYPES: &[&str] = &["Application", "SpringBootApplicationConfigurationType"];

/// One `<option name="…" value="…"/>` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionEntry {
    pub name: String,
    pub value: String,
}

/// One `<configuration …>…</configuration>` block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    pub name: String,
    pub config_type: String,
    pub factory_name: String,
    /// `default="true"` marks a project-level template, not a real runnable.
    pub default: bool,
    pub folder_name: Option<String>,
    pub options: Vec<OptionEntry>,
    pub envs: Vec<(String, String)>,
    /// `<module name="…"/>` inside the configuration.
    pub module: Option<String>,
    /// Byte range of the raw `VM_PARAMETERS` value (between the quotes).
    pub vm_parameters_span: Option<(usize, usize)>,
    /// Byte range of the raw `PROGRAM_PARAMETERS` value (between the quotes).
    pub program_parameters_span: Option<(usize, usize)>,
    /// Byte offset just after the opening `<configuration …>` tag.
    pub open_tag_end: usize,
    /// Byte offset of the `</configuration>` close tag.
    pub close_tag_start: usize,
}

impl Config {
    /// The raw (unescaped) value of an option.
    pub fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|option| option.name == name)
            .map(|option| option.value.as_str())
    }

    pub fn vm_parameters(&self) -> Option<&str> {
        self.option("VM_PARAMETERS")
    }

    pub fn program_parameters(&self) -> Option<&str> {
        self.option("PROGRAM_PARAMETERS")
    }

    pub fn main_class(&self) -> Option<&str> {
        self.option("MAIN_CLASS_NAME")
    }

    pub fn module_name(&self) -> Option<&str> {
        self.option("MODULE_NAME").or(self.module.as_deref())
    }

    pub fn working_directory(&self) -> Option<&str> {
        self.option("WORKING_DIRECTORY")
    }

    pub fn env(&self, key: &str) -> Option<&str> {
        self.envs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// The JVM options split into arguments (leniently, so a stray quote does
    /// not lose the whole configuration).
    pub fn vm_args(&self) -> Vec<String> {
        self.vm_parameters().map(shell::split_args).unwrap_or_default()
    }

    /// The program arguments split into arguments.
    pub fn program_args(&self) -> Vec<String> {
        self.program_parameters()
            .map(shell::split_args)
            .unwrap_or_default()
    }

    /// Whether this is a real (non-template) JVM application configuration.
    pub fn is_java_application(&self) -> bool {
        !self.default && JAVA_CONFIG_TYPES.contains(&self.config_type.as_str())
    }
}

/// Parses every `<configuration>` block in `text`, in document order.
pub fn parse(text: &str) -> Vec<Config> {
    let mut configs = Vec::new();
    let mut pos = 0usize;
    while let Some(lt) = find_next_tag_start(text, pos) {
        let Some(tag) = parse_tag(text, lt) else {
            pos = lt + 1;
            continue;
        };
        if tag.name == "configuration" && !tag.closing && !tag.self_closing {
            let mut config = Config::default();
            config.open_tag_end = tag.end;
            config.close_tag_start = tag.end;
            fill_attributes(&mut config, &tag);
            pos = parse_configuration_body(text, tag.end, &mut config);
            configs.push(config);
            continue;
        }
        pos = tag.end.max(lt + 1);
    }
    configs
}

/// Replaces the `VM_PARAMETERS` value of `config` in `text`.
///
/// Returns the edited text and whether the configuration already had a
/// `VM_PARAMETERS` option (as opposed to one inserted at the top).
pub fn set_vm_parameters(text: &str, config: &Config, value: &str) -> (String, bool) {
    set_option_value(text, config, "VM_PARAMETERS", value)
}

/// Replaces the value of one `<option name="…" value="…"/>` of `config` in
/// `text`, inserting the option right after the opening tag when it is missing.
///
/// Returns the edited text and whether the option already existed.
pub fn set_option_value(text: &str, config: &Config, name: &str, value: &str) -> (String, bool) {
    let escaped = escape_xml_attr(value);
    let span = match name {
        "VM_PARAMETERS" => config.vm_parameters_span,
        "PROGRAM_PARAMETERS" => config.program_parameters_span,
        _ => None,
    };
    if let Some((start, end)) = span {
        if end <= text.len() && start <= end {
            let mut out = String::with_capacity(text.len() + escaped.len());
            out.push_str(&text[..start]);
            out.push_str(&escaped);
            out.push_str(&text[end..]);
            return (out, true);
        }
    }
    let at = config.open_tag_end.min(text.len());
    let indent = child_indent(text, at);
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(text.len() + escaped.len() + 64);
    out.push_str(&text[..at]);
    out.push_str(&format!(
        "{newline}{indent}<option name=\"{name}\" value=\"{escaped}\" />"
    ));
    out.push_str(&text[at..]);
    (out, false)
}

/// Indentation and line ending of the first element inside the configuration
/// whose opening tag ends at `at`, so inserted options line up with the file.
fn child_indent(text: &str, at: usize) -> String {
    let rest = &text[at..];
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')).unwrap_or(rest);
    let line = rest.split(['\n', '\r']).next().unwrap_or("");
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    if indent.is_empty() { "    ".to_string() } else { indent }
}

/// Escapes a value for a double-quoted XML attribute.
pub fn escape_xml_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    out
}

/// Unescapes XML entities in an attribute value or text node.
pub fn unescape_xml(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';') else {
            out.push_str(&rest[amp..]);
            return out;
        };
        let entity = &after[..semi];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            other => {
                if let Some(hex) = other.strip_prefix("#x").or_else(|| other.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = other.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match replacement {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone)]
struct Attr {
    name: String,
    value: String,
    value_span: (usize, usize),
}

#[derive(Debug, Clone)]
struct Tag {
    name: String,
    attrs: Vec<Attr>,
    closing: bool,
    self_closing: bool,
    start: usize,
    end: usize,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
            .map(|attr| attr.value.as_str())
    }

    fn attr_span(&self, name: &str) -> Option<(usize, usize)> {
        self.attrs
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
            .map(|attr| attr.value_span)
    }
}

/// Finds the next `<` that starts an element, skipping comments and
/// declarations; returns its index.
fn find_next_tag_start(text: &str, from: usize) -> Option<usize> {
    let mut pos = from;
    while let Some(rel) = text[pos..].find('<') {
        let at = pos + rel;
        let rest = &text[at..];
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(end) => pos = at + end + 3,
                None => return None,
            }
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            match rest.find('>') {
                Some(end) => pos = at + end + 1,
                None => return None,
            }
            continue;
        }
        return Some(at);
    }
    None
}

fn parse_tag(text: &str, at: usize) -> Option<Tag> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b'<') {
        return None;
    }
    let mut i = at + 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while i < text.len() && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/') {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = text[name_start..i].to_string();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < text.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        match bytes.get(i).copied() {
            None => return None,
            Some(b'>') => {
                i += 1;
                break;
            }
            Some(b'/') => {
                if bytes.get(i + 1) == Some(&b'>') {
                    self_closing = true;
                    i += 2;
                    break;
                }
                i += 1;
            }
            Some(_) => {
                let attr_start = i;
                while i < text.len()
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                let attr_name = text[attr_start..i].to_string();
                while i < text.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                    i += 1;
                }
                let mut value = String::new();
                let mut value_span = (i, i);
                if bytes.get(i) == Some(&b'=') {
                    i += 1;
                    while i < text.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                        i += 1;
                    }
                    let quote = match bytes.get(i).copied() {
                        Some(q @ (b'"' | b'\'')) => Some(q),
                        _ => None,
                    };
                    if let Some(q) = quote {
                        i += 1;
                        let value_start = i;
                        while i < text.len() && bytes[i] != q {
                            i += 1;
                        }
                        value_span = (value_start, i.min(text.len()));
                        value = unescape_xml(&text[value_start..value_span.1]);
                        if i < text.len() {
                            i += 1; // closing quote
                        }
                    } else {
                        let value_start = i;
                        while i < text.len()
                            && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
                        {
                            i += 1;
                        }
                        value_span = (value_start, i);
                        value = unescape_xml(&text[value_start..i]);
                    }
                }
                if !attr_name.is_empty() {
                    attrs.push(Attr {
                        name: attr_name,
                        value,
                        value_span,
                    });
                }
            }
        }
    }
    Some(Tag {
        name,
        attrs,
        closing,
        self_closing,
        start: at,
        end: i,
    })
}

fn fill_attributes(config: &mut Config, tag: &Tag) {
    config.name = tag.attr("name").unwrap_or_default().to_string();
    config.config_type = tag.attr("type").unwrap_or_default().to_string();
    config.factory_name = tag.attr("factoryName").unwrap_or_default().to_string();
    config.default = tag
        .attr("default")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    config.folder_name = tag.attr("folderName").map(|s| s.to_string());
}

/// Parses the body of one configuration, starting at `from` (just after the
/// opening tag); returns the offset to continue from.
fn parse_configuration_body(text: &str, from: usize, config: &mut Config) -> usize {
    let mut depth = 0usize;
    let mut in_envs = false;
    let mut pos = from;
    while let Some(lt) = find_next_tag_start(text, pos) {
        let Some(tag) = parse_tag(text, lt) else {
            pos = lt + 1;
            continue;
        };
        let next = tag.end.max(lt + 1);
        if tag.closing {
            match tag.name.as_str() {
                "configuration" => {
                    if depth == 0 {
                        config.close_tag_start = tag.start;
                        return next;
                    }
                    depth -= 1;
                }
                "envs" => in_envs = false,
                _ => {}
            }
            pos = next;
            continue;
        }
        match tag.name.as_str() {
            "configuration" if !tag.self_closing => depth += 1,
            "option" => {
                let name = tag.attr("name").unwrap_or_default().to_string();
                let value = tag.attr("value").unwrap_or_default().to_string();
                if name == "VM_PARAMETERS" {
                    config.vm_parameters_span = tag.attr_span("value");
                } else if name == "PROGRAM_PARAMETERS" {
                    config.program_parameters_span = tag.attr_span("value");
                }
                config.options.push(OptionEntry { name, value });
            }
            "module" => {
                if let Some(name) = tag.attr("name") {
                    config.module = Some(name.to_string());
                }
            }
            "envs" => in_envs = true,
            "env" => {
                if in_envs {
                    if let Some(name) = tag.attr("name") {
                        config
                            .envs
                            .push((name.to_string(), tag.attr("value").unwrap_or_default().to_string()));
                    }
                }
            }
            _ => {}
        }
        pos = next;
    }
    config.close_tag_start = text.len();
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN_XML: &str = r#"<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="DemoApplication" type="SpringBootApplicationConfigurationType" factoryName="Spring Boot">
    <module name="demo" />
    <option name="SPRING_BOOT_MAIN_CLASS" value="com.example.DemoApplication" />
    <option name="VM_PARAMETERS" value="-Xmx512m -Dfoo=&quot;a b&quot;" />
    <option name="PROGRAM_PARAMETERS" value="--server.port=8080" />
    <!-- a comment with <tags> inside -->
    <option name="WORKING_DIRECTORY" value="$PROJECT_DIR$" />
    <envs>
      <env name="SPRING_PROFILES_ACTIVE" value="dev" />
    </envs>
    <method v="2">
      <option name="Make" enabled="true" />
    </method>
  </configuration>
  <configuration default="true" name="Application" type="Application" factoryName="Application">
    <option name="VM_PARAMETERS" value="-ea" />
  </configuration>
</component>
"#;

    #[test]
    fn parses_both_configurations() {
        let configs = parse(RUN_XML);
        assert_eq!(configs.len(), 2);
        let first = &configs[0];
        assert_eq!(first.name, "DemoApplication");
        assert_eq!(first.config_type, "SpringBootApplicationConfigurationType");
        assert_eq!(first.factory_name, "Spring Boot");
        assert!(!first.default);
        assert_eq!(first.module_name(), Some("demo"));
        assert_eq!(first.vm_parameters(), Some("-Xmx512m -Dfoo=\"a b\""));
        assert_eq!(first.program_parameters(), Some("--server.port=8080"));
        assert_eq!(first.working_directory(), Some("$PROJECT_DIR$"));
        assert_eq!(first.env("SPRING_PROFILES_ACTIVE"), Some("dev"));
        assert!(first.is_java_application());
        assert_eq!(first.vm_args(), vec!["-Xmx512m", "-Dfoo=a b"]);
        assert_eq!(first.program_args(), vec!["--server.port=8080"]);

        let second = &configs[1];
        assert!(second.default);
        assert!(!second.is_java_application());
    }

    #[test]
    fn ignores_non_configuration_elements_and_comments() {
        let text = r#"<component name="RunManager">
            <method v="2"><option name="Make" enabled="true" /></method>
            <configuration name="App" type="Application" factoryName="Application">
                <option name="MAIN_CLASS_NAME" value="a.B" />
            </configuration>
        </component>"#;
        let configs = parse(text);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].main_class(), Some("a.B"));
        assert!(configs[0].vm_parameters_span.is_none());
    }

    #[test]
    fn replaces_existing_vm_parameters_in_place() {
        let configs = parse(RUN_XML);
        let config = &configs[0];
        let (updated, replaced) = set_vm_parameters(RUN_XML, config, "-Xmx2g -Dbar=b");
        assert!(replaced);
        assert!(updated.contains(r#"<option name="VM_PARAMETERS" value="-Xmx2g -Dbar=b" />"#));
        assert!(!updated.contains("-Xmx512m"));
        // Everything else is untouched, including comments and the other config.
        assert!(updated.contains("<!-- a comment with <tags> inside -->"));
        assert!(updated.contains(r#"<option name="PROGRAM_PARAMETERS" value="--server.port=8080" />"#));
        assert!(updated.contains(r#"<option name="VM_PARAMETERS" value="-ea" />"#));
        assert_eq!(parse(&updated).len(), 2);
        assert_eq!(parse(&updated)[0].vm_parameters(), Some("-Xmx2g -Dbar=b"));
    }

    #[test]
    fn inserts_vm_parameters_when_missing() {
        let text = r#"<component name="ProjectRunConfigurationManager">
  <configuration name="App" type="Application" factoryName="Application">
    <option name="MAIN_CLASS_NAME" value="a.B" />
  </configuration>
</component>
"#;
        let configs = parse(text);
        let (updated, replaced) = set_vm_parameters(text, &configs[0], "-ea -Dx=1");
        assert!(!replaced);
        let reparsed = parse(&updated);
        assert_eq!(reparsed[0].vm_parameters(), Some("-ea -Dx=1"));
        assert_eq!(reparsed[0].main_class(), Some("a.B"));
        assert_eq!(parse(&updated).len(), 1);
    }

    #[test]
    fn escapes_and_unescapes_values() {
        let escaped = escape_xml_attr(r#"-Dfoo="a&b" <x>"#);
        assert_eq!(escaped, "-Dfoo=&quot;a&amp;b&quot; &lt;x&gt;");
        assert_eq!(unescape_xml(&escaped), r#"-Dfoo="a&b" <x>"#);
        assert_eq!(unescape_xml("a&amp;b&#65;&#x42;c"), "a&bABc");
        assert_eq!(unescape_xml("no entities"), "no entities");
        assert_eq!(unescape_xml("broken &amp"), "broken &amp");
        assert_eq!(unescape_xml("&unknown;"), "&unknown;");
    }

    #[test]
    fn handles_self_closing_and_garbage_input() {
        assert!(parse("").is_empty());
        assert!(parse("<component/>").is_empty());
        assert_eq!(parse("just text < > </>").len(), 0);
        let text = r#"<configuration name="A" type="Application"/>"#;
        assert!(parse(text).is_empty());
    }

    #[test]
    fn parses_workspace_xml_shape() {
        let text = r#"<project version="4">
  <component name="RunManager" selected="Application.App">
    <configuration name="App" type="Application" factoryName="Application" temporary="true">
      <option name="MAIN_CLASS_NAME" value="a.B" />
      <option name="VM_PARAMETERS" value="-ea" />
      <option name="WORKING_DIRECTORY" value="$PROJECT_DIR$/module" />
      <module name="module" />
    </configuration>
    <configuration name="UnitTest" type="JUnit" factoryName="JUnit">
      <option name="VM_PARAMETERS" value="-ea" />
    </configuration>
  </component>
</project>"#;
        let configs = parse(text);
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].working_directory(), Some("$PROJECT_DIR$/module"));
        assert!(configs[0].is_java_application());
        assert!(!configs[1].is_java_application());
    }
}
