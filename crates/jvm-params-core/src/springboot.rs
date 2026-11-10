//! Spring Boot applications detected the way the IDE detects them.
//!
//! The IDE materialises one launch configuration per `@SpringBootApplication` main
//! class as `java -cp ${classpath} <class>` in the owning module, with the id
//! `application:<module>:<class>` (or `application:<class>` for the build at the
//! project root) and the simple class name as the name — see
//! `crates/ide-run/src/detect.rs::spring_boot_apps` upstream. Those configurations
//! carry no VM options at all, and a plugin cannot edit them, so the way to add
//! options to them is to write configurations with the **same id** into
//! `.openidea/run.toml`: the host lists saved configurations and drops the
//! detected ones whose id is already saved (`detected_and_saved`).
//!
//! This module therefore mirrors that detection: modules whose build applies the
//! Spring Boot plugin and whose shallowest package declares the application
//! class.

use crate::detect::{self, Reader};

/// How many source files are read per module, as upstream does.
const SCAN_BUDGET: usize = 400;

/// Largest source file that is read, as upstream does.
const MAX_SOURCE: usize = 256 * 1024;

/// One detected Spring Boot application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// `application:<module>:<class>`, the id the IDE gives the same application.
    pub id: String,
    /// The simple class name, which is what the IDE names the configuration.
    pub name: String,
    /// Owning module, relative to the project root; empty for the root build.
    pub module: String,
    /// Binary name of the main class.
    pub main_class: String,
}

/// Source files considered for detection: paths relative to the project root,
/// below `src/main/java` or `src/main/kotlin`.
pub fn source_globs() -> Vec<&'static str> {
    vec!["**/src/main/java/**/*.java", "**/src/main/kotlin/**/*.kt"]
}

/// Spring Boot applications of `project`, reading the sources in `sources`
/// through `reader`.
pub fn apps(project: &detect::Project, sources: &[String], reader: &dyn Reader) -> Vec<App> {
    let mut out = Vec::new();
    for module in project.modules.iter().filter(|m| m.spring_boot) {
        let dir = module.dir.trim_end_matches('/');
        // Shallowest packages first: the application class sits in the root package
        // of the code it scans, exactly as upstream reads it.
        let mut candidates: Vec<&String> = sources.iter().filter(|source| in_main_tree(source, dir)).collect();
        candidates.sort_by_key(|source| depth_of(source, dir));
        let mut found: Vec<(usize, String)> = Vec::new();
        let mut read = 0usize;
        for source in candidates {
            if read >= SCAN_BUDGET {
                break;
            }
            let Some(text) = reader.read(source) else {
                continue;
            };
            if text.len() > MAX_SOURCE {
                continue;
            }
            read += 1;
            if let Some(class) = main_class(&text, source) {
                found.push((depth_of(source, dir), class));
            }
        }
        let Some(shallowest) = found.iter().map(|(depth, _)| *depth).min() else {
            continue;
        };
        let mut classes: Vec<String> = found
            .into_iter()
            .filter(|(depth, _)| *depth == shallowest)
            .map(|(_, class)| class)
            .collect();
        classes.sort();
        classes.dedup();
        for class in classes {
            out.push(app(dir, &class));
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn app(module: &str, class: &str) -> App {
    let name = class.rsplit(['.', '$']).next().unwrap_or(class).to_owned();
    let id = if module.is_empty() {
        format!("application:{class}")
    } else {
        format!("application:{}:{class}", module.replace('\\', "/"))
    };
    App { id, name, module: module.to_owned(), main_class: class.to_owned() }
}

/// Whether `source` lies in the main source tree of `module`.
fn in_main_tree(source: &str, module: &str) -> bool {
    let source = source.replace('\\', "/");
    let prefix = if module.is_empty() {
        String::new()
    } else {
        format!("{}/", module.replace('\\', "/").trim_matches('/'))
    };
    let Some(rest) = source.strip_prefix(&prefix) else {
        return false;
    };
    ["src/main/java/", "src/main/kotlin/"]
        .iter()
        .any(|tree| rest.starts_with(tree) || rest == &tree[..tree.len() - 1])
}

/// Directory depth below the module's main source tree; the shallower, the closer
/// to the root package the application class lives in.
fn depth_of(source: &str, module: &str) -> usize {
    let source = source.replace('\\', "/");
    let prefix = if module.is_empty() {
        String::new()
    } else {
        format!("{}/", module.replace('\\', "/").trim_matches('/'))
    };
    let rest = source.strip_prefix(&prefix).unwrap_or(&source);
    let rest = ["src/main/java/", "src/main/kotlin/"]
        .iter()
        .find_map(|tree| rest.strip_prefix(tree))
        .unwrap_or(rest);
    rest.matches('/').count()
}

/// The binary name of the `@SpringBootApplication` main class declared in `source`,
pub fn main_class(source: &str, path: &str) -> Option<String> {
    let code: Vec<&str> = source.lines().map(|line| line.split("//").next().unwrap_or_default()).collect();
    let code = code.join("\n");
    let package = code
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("package "))
        .map(|rest| rest.trim_end_matches(';').trim().to_owned())
        .filter(|package| !package.is_empty());
    let qualify = |name: &str| match &package {
        Some(package) => format!("{package}.{name}"),
        None => name.to_owned(),
    };

    if path.ends_with(".kt") {
        // A top-level `fun main` compiles into the file facade (`AppKt`, or the
        // name given by `@file:JvmName`), and the annotation sits on the class
        // the facade delegates to, so the file itself needs none.
        let top_level_main = code
            .lines()
            .any(|line| line.starts_with("fun main(") || line.starts_with("fun main ("));
        if top_level_main {
            let facade = code
                .lines()
                .find_map(|line| line.trim().strip_prefix("@file:JvmName("))
                .and_then(|rest| rest.split('"').nth(1))
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    let stem = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".kt");
                    let mut chars = stem.chars();
                    let upper: String = chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect())
                        .unwrap_or_default();
                    format!("{upper}Kt")
                });
            return Some(qualify(&facade));
        }
        if !code.contains("@SpringBootApplication") || !code.contains("fun main(") {
            return None;
        }
    }

    let at = code.find("@SpringBootApplication")?;
    if !path.ends_with(".kt") && !code.contains("void main(") {
        return None;
    }
    let class: String = code[at..]
        .split_whitespace()
        .skip_while(|token| *token != "class")
        .nth(1)?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if class.is_empty() {
        return None;
    }
    Some(qualify(&class))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{detect_with_reader, MapReader};

    fn project(reader: &MapReader) -> detect::Project {
        let files = detect::Files {
            poms: vec!["pom.xml".to_string(), "attach-dma/pom.xml".to_string()],
            ..Default::default()
        };
        detect_with_reader("D:\\work\\app", &files, reader)
    }

    #[test]
    fn finds_an_application_in_a_module() {
        let reader = MapReader::new()
            .with(
                "pom.xml",
                "<project><artifactId>parent</artifactId><packaging>pom</packaging></project>",
            )
            .with(
                "attach-dma/pom.xml",
                "<project><build><plugins><plugin><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>",
            );
        let project = project(&reader);
        let sources = vec![
            "attach-dma/src/main/java/com/attach/dma/AttachDmaApplication.java".to_string(),
            "attach-dma/src/main/java/com/attach/dma/service/DmaService.java".to_string(),
        ];
        let reader = reader
            .with(
                "attach-dma/src/main/java/com/attach/dma/AttachDmaApplication.java",
                "package com.attach.dma;\n\n@SpringBootApplication\npublic class AttachDmaApplication {\n    public static void main(String[] args) {}\n}\n",
            )
            .with(
                "attach-dma/src/main/java/com/attach/dma/service/DmaService.java",
                "package com.attach.dma.service;\n\n@SpringBootApplication\npublic class DmaService {\n    public static void main(String[] args) {}\n}\n",
            );
        let apps = apps(&project, &sources, &reader);
        assert_eq!(apps.len(), 1, "only the shallowest package counts: {apps:?}");
        assert_eq!(apps[0].id, "application:attach-dma:com.attach.dma.AttachDmaApplication");
        assert_eq!(apps[0].name, "AttachDmaApplication");
        assert_eq!(apps[0].module, "attach-dma");
    }

    #[test]
    fn ignores_modules_without_the_plugin() {
        let reader = MapReader::new()
            .with("pom.xml", "<project><artifactId>parent</artifactId></project>")
            .with("attach-dma/pom.xml", "<project><artifactId>attach-dma</artifactId></project>");
        let project = project(&reader);
        let sources = vec!["attach-dma/src/main/java/a/App.java".to_string()];
        let reader = reader.with(
            "attach-dma/src/main/java/a/App.java",
            "package a;\n@SpringBootApplication\npublic class App { public static void main(String[] a) {} }\n",
        );
        assert!(apps(&project, &sources, &reader).is_empty());
    }

    #[test]
    fn root_build_ids_have_no_module() {
        let reader = MapReader::new().with(
            "pom.xml",
            "<project><build><plugins><plugin><artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>",
        );
        let project = project(&reader);
        let sources = vec!["src/main/java/demo/App.java".to_string()];
        let reader = reader.with(
            "src/main/java/demo/App.java",
            "package demo;\n@SpringBootApplication\npublic class App { public static void main(String[] a) {} }\n",
        );
        let apps = apps(&project, &sources, &reader);
        assert_eq!(apps.len(), 1, "{apps:?}");
        assert_eq!(apps[0].id, "application:demo.App");
        assert_eq!(apps[0].module, "");
    }

    #[test]
    fn kotlin_top_level_main_uses_the_file_facade() {
        let src = "package demo\n\nfun main(args: Array<String>) {}\n";
        assert_eq!(main_class(src, "src/main/kotlin/demo/Shop.kt").as_deref(), Some("demo.ShopKt"));
        let src = "package demo\n@file:JvmName(\"Runner\")\nfun main() {}\n";
        assert_eq!(main_class(src, "src/main/kotlin/demo/Run.kt").as_deref(), Some("demo.Runner"));
    }

    #[test]
    fn a_class_without_main_is_not_an_application() {
        let src = "package a;\n@SpringBootApplication\npublic class App { void run() {} }\n";
        assert_eq!(main_class(src, "src/main/java/a/App.java"), None);
    }
}
