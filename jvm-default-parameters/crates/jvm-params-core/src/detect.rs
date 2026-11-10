//! Detecting the Java modules of a project and how to launch them.
//!
//! A plugin only gets a project root and a few file-system calls, so detection is
//! deliberately cheap: it looks at the build files that matter (`pom.xml`,
//! `build.gradle`, `build.gradle.kts`), the wrapper scripts next to them, and the
//! jars a previous build left in `target/` or `build/libs/`. Text scanning is
//! enough to answer the only question we ask — "is this a Spring Boot module, and
//! which jar or wrapper launches it?" — and keeps the crate dependency free.

use std::collections::BTreeMap;

/// Which build tool owns a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Maven,
    Gradle,
}

impl Build {
    pub fn as_str(self) -> &'static str {
        match self {
            Build::Maven => "maven",
            Build::Gradle => "gradle",
        }
    }
}

/// Wrapper scripts found next to a build file; both platform flavors are kept
/// because the plugin cannot ask the IDE which OS it runs on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wrappers {
    /// Relative path to `mvnw` (Unix shell script).
    pub mvnw_unix: Option<String>,
    /// Relative path to `mvnw.cmd` (Windows batch file).
    pub mvnw_cmd: Option<String>,
    /// Relative path to `gradlew` (Unix shell script).
    pub gradlew_unix: Option<String>,
    /// Relative path to `gradlew.bat` (Windows batch file).
    pub gradlew_bat: Option<String>,
}

impl Wrappers {
    pub fn is_empty(&self) -> bool {
        self.mvnw_unix.is_none()
            && self.mvnw_cmd.is_none()
            && self.gradlew_unix.is_none()
            && self.gradlew_bat.is_none()
    }
}

/// One detected Java module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    /// Directory relative to the project root, `""` for the root itself,
    /// with `/` separators.
    pub dir: String,
    /// Display name: the directory's last segment, or the root's name.
    pub name: String,
    pub build: Build,
    /// Whether the build file applies Spring Boot.
    pub spring_boot: bool,
    pub wrappers: Wrappers,
    /// Relative path of the most likely Spring Boot jar from a previous build.
    pub boot_jar: Option<String>,
}

impl Module {
    pub fn dir_relative(&self) -> &str {
        &self.dir
    }
}

/// The paths a project scan produced, as returned by the host's `find_files`.
/// They may be absolute (under the project root) or already relative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    pub poms: Vec<String>,
    pub gradle_builds: Vec<String>,
    pub wrappers: Vec<String>,
    pub jars: Vec<String>,
}

/// Reads a project file by its relative path; the plugin wraps
/// `Project::read_file`, the tests use a map.
pub trait Reader {
    fn read(&self, relative_path: &str) -> Option<String>;
}

/// A simple in-memory [`Reader`], useful for tests and for dry runs.
#[derive(Debug, Clone, Default)]
pub struct MapReader {
    files: BTreeMap<String, String>,
}

impl MapReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, path: &str, contents: &str) -> Self {
        self.files.insert(path.to_string(), contents.to_string());
        self
    }
}

impl Reader for MapReader {
    fn read(&self, relative_path: &str) -> Option<String> {
        self.files.get(relative_path).cloned()
    }
}

/// The result of a project scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: String,
    pub modules: Vec<Module>,
}

impl Project {
    /// Whether the project contains a Spring Boot module.
    pub fn has_spring_boot(&self) -> bool {
        self.modules.iter().any(|module| module.spring_boot)
    }
}

/// Where to look for a runnable jar, relative to a module directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectOptions {
    /// Directories searched for jar files, in order of preference.
    pub jar_dirs: Vec<String>,
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self {
            jar_dirs: vec!["target".to_string(), "build/libs".to_string()],
        }
    }
}

impl DetectOptions {
    /// The default directories plus any extra ones the user configured, without
    /// duplicates and normalized to `/` separators.
    pub fn with_extra_dirs(extra: &[String]) -> Self {
        let mut jar_dirs = Self::default().jar_dirs;
        for dir in extra {
            let normalized = dir.trim().replace('\\', "/");
            let normalized = normalized.trim_matches('/');
            if !normalized.is_empty() && !jar_dirs.iter().any(|known| known == normalized) {
                jar_dirs.push(normalized.to_string());
            }
        }
        Self { jar_dirs }
    }
}

/// Scans `files` (all paths relative to `root`, or absolute below it).
///
/// Modules are discovered from build files; a module without a Spring Boot
/// marker is kept because it may still have a runnable jar.
pub fn detect(root: &str, files: &Files) -> Project {
    detect_with_options(root, files, &MapReader::new(), &DetectOptions::default())
}

/// Like [`detect`], but reads build files through `reader` to find Spring Boot.
pub fn detect_with_reader(root: &str, files: &Files, reader: &dyn Reader) -> Project {
    detect_with_options(root, files, reader, &DetectOptions::default())
}

/// Like [`detect_with_reader`], with the jar search directories configurable.
pub fn detect_with_options(
    root: &str,
    files: &Files,
    reader: &dyn Reader,
    options: &DetectOptions,
) -> Project {
    let mut by_dir: BTreeMap<String, Module> = BTreeMap::new();
    let mut wrapper_dirs: Vec<String> = Vec::new();
    let mut wrappers_by_dir: BTreeMap<String, Wrappers> = BTreeMap::new();

    for wrapper in &files.wrappers {
        let Some(rel) = relative_path(root, wrapper) else {
            continue;
        };
        let Some((dir, file_name)) = split_parent(&rel) else {
            continue;
        };
        let entry = wrappers_by_dir.entry(dir.clone()).or_default();
        match file_name.as_str() {
            "mvnw" => entry.mvnw_unix = Some(rel.clone()),
            "mvnw.cmd" => entry.mvnw_cmd = Some(rel.clone()),
            "gradlew" => entry.gradlew_unix = Some(rel.clone()),
            "gradlew.bat" => entry.gradlew_bat = Some(rel.clone()),
            _ => continue,
        }
        if !wrapper_dirs.contains(&dir) {
            wrapper_dirs.push(dir);
        }
    }

    for pom in &files.poms {
        let Some(rel) = relative_path(root, pom) else {
            continue;
        };
        let Some((dir, _)) = split_parent(&rel) else {
            continue;
        };
        let contents = reader.read(&rel).unwrap_or_default();
        let spring_boot = pom_is_spring_boot(&contents);
        by_dir.insert(
            dir.clone(),
            Module {
                name: module_name(root, &dir),
                dir,
                build: Build::Maven,
                spring_boot,
                wrappers: Wrappers::default(),
                boot_jar: None,
            },
        );
    }

    for gradle in &files.gradle_builds {
        let Some(rel) = relative_path(root, gradle) else {
            continue;
        };
        let Some((dir, file_name)) = split_parent(&rel) else {
            continue;
        };
        // A directory can hold both build.gradle and build.gradle.kts; the Kotlin
        // one wins only when it is the only one.
        if file_name != "build.gradle" && file_name != "build.gradle.kts" {
            continue;
        }
        if by_dir.contains_key(&dir) {
            continue;
        }
        let contents = reader.read(&rel).unwrap_or_default();
        by_dir.insert(
            dir.clone(),
            Module {
                name: module_name(root, &dir),
                dir: dir.clone(),
                build: Build::Gradle,
                spring_boot: gradle_is_spring_boot(&contents),
                wrappers: Wrappers::default(),
                boot_jar: None,
            },
        );
    }

    let modules: Vec<Module> = by_dir.into_values().collect();
    let modules = modules
        .into_iter()
        .map(|mut module| {
            module.wrappers = nearest_wrappers(&module.dir, &wrappers_by_dir);
            module.boot_jar = best_jar(root, files, &module.dir, &options.jar_dirs);
            module
        })
        .collect();

    Project {
        root: root.to_string(),
        modules,
    }
}

fn module_name(root: &str, dir: &str) -> String {
    if dir.is_empty() {
        let normalized = root.replace('\\', "/");
        let last = normalized
            .split('/')
            .filter(|segment| !segment.is_empty())
            .next_back()
            .unwrap_or("project");
        return last.to_string();
    }
    dir.rsplit('/').next().unwrap_or(dir).to_string()
}

/// Walks up from `dir` to the root and returns the wrappers of the nearest
/// directory that has any.
fn nearest_wrappers(dir: &str, wrappers_by_dir: &BTreeMap<String, Wrappers>) -> Wrappers {
    let mut current = dir.to_string();
    loop {
        if let Some(found) = wrappers_by_dir.get(&current) {
            if !found.is_empty() {
                return found.clone();
            }
        }
        if current.is_empty() {
            return Wrappers::default();
        }
        current = match current.rfind('/') {
            Some(index) => current[..index].to_string(),
            None => String::new(),
        };
    }
}

/// Picks the jar a launch would use: not a sources/javadoc/plain artifact, from
/// one of the configured jar directories of this module.
fn best_jar(root: &str, files: &Files, dir: &str, jar_dirs: &[String]) -> Option<String> {
    // Jar directory order is the preference order; inside one directory the
    // alphabetically first artifact wins (artifacts carry the version).
    for candidate_dir in jar_dirs {
        let candidate_dir = if dir.is_empty() {
            candidate_dir.clone()
        } else {
            format!("{dir}/{candidate_dir}")
        };
        let mut candidates: Vec<String> = Vec::new();
        for jar in &files.jars {
            let Some(rel) = relative_path(root, jar) else {
                continue;
            };
            let Some((jar_dir, file_name)) = split_parent(&rel) else {
                continue;
            };
            if jar_dir != candidate_dir {
                continue;
            }
            let lower = file_name.to_ascii_lowercase();
            if !lower.ends_with(".jar")
                || lower.contains("-sources")
                || lower.contains("-javadoc")
                || lower.contains("-tests")
                || lower.contains("-plain")
                || lower.ends_with(".jar.original")
            {
                continue;
            }
            candidates.push(rel);
        }
        if !candidates.is_empty() {
            candidates.sort();
            return candidates.into_iter().next();
        }
    }
    None
}

/// Whether a `pom.xml` applies Spring Boot.
pub fn pom_is_spring_boot(contents: &str) -> bool {
    let compact: String = contents.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("<artifactId>spring-boot-maven-plugin</artifactId>")
        || compact.contains("<artifactId>spring-boot-starter-parent</artifactId>")
        || compact.contains("<parent><groupId>org.springframework.boot</groupId>")
        || compact.contains("spring-boot-starter-web")
        || compact.contains("spring-boot-starter")
}

/// Whether a Gradle build applies Spring Boot.
pub fn gradle_is_spring_boot(contents: &str) -> bool {
    contents.contains("org.springframework.boot")
}

/// Returns `path` relative to `root` with `/` separators, or `None` when it is
/// outside the root.
pub fn relative_path(root: &str, path: &str) -> Option<String> {
    let normalized_root = normalize(root);
    let normalized_path = normalize(path);
    if normalized_path == normalized_root {
        return Some(String::new());
    }
    if let Some(rest) = normalized_path.strip_prefix(&(normalized_root.clone() + "/")) {
        return Some(rest.to_string());
    }
    // The host may report a differently-cased root (Windows) or a path that is
    // already relative to the project.
    if normalized_root.len() == normalized_path.len() {
        let lower_root = normalized_root.to_ascii_lowercase();
        let lower_path = normalized_path.to_ascii_lowercase();
        if lower_path == lower_root {
            return Some(String::new());
        }
    }
    let lower_prefix = normalized_root.to_ascii_lowercase() + "/";
    if normalized_path.len() >= lower_prefix.len()
        && normalized_path[..lower_prefix.len()].eq_ignore_ascii_case(&lower_prefix)
    {
        return Some(normalized_path[lower_prefix.len()..].to_string());
    }
    if !normalized_root.is_empty() && !is_absolute(&normalized_path) {
        // Already relative to the project root.
        return Some(normalized_path.trim_start_matches("./").to_string());
    }
    None
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || path.get(1..2) == Some(":")
}

fn normalize(path: &str) -> String {
    let mut text = path.replace('\\', "/");
    while text.ends_with('/') && text.len() > 1 {
        text.pop();
    }
    if text.starts_with("//?/") {
        text = text[4..].to_string();
    }
    text
}

/// Splits `a/b/c.txt` into `("a/b", "c.txt")`; the root is `("", "pom.xml")`.
fn split_parent(relative_path: &str) -> Option<(String, String)> {
    if relative_path.is_empty() {
        return None;
    }
    match relative_path.rfind('/') {
        Some(index) => Some((
            relative_path[..index].to_string(),
            relative_path[index + 1..].to_string(),
        )),
        None => Some((String::new(), relative_path.to_string())),
    }
}

/// Joins a project root and a relative path using the separator style of `root`.
pub fn join_path(root: &str, relative: &str) -> String {
    if relative.is_empty() {
        return root.to_string();
    }
    let separator = if root.contains('\\') { '\\' } else { '/' };
    let mut out = root.trim_end_matches(['/', '\\']).to_string();
    for segment in relative.split(['/', '\\']).filter(|s| !s.is_empty()) {
        out.push(separator);
        out.push_str(segment);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const POM_SPRING: &str = r#"<project><parent><groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-starter-parent</artifactId></parent>
        <build><plugins><plugin><groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-maven-plugin</artifactId></plugin></plugins></build></project>"#;
    const POM_PLAIN: &str = r#"<project><artifactId>plain</artifactId></project>"#;
    const GRADLE_SPRING: &str = r#"plugins { id 'org.springframework.boot' version '3.2.0' }"#;

    fn reader() -> MapReader {
        MapReader::new()
            .with("pom.xml", POM_SPRING)
            .with("common/pom.xml", POM_PLAIN)
            .with("app/build.gradle", GRADLE_SPRING)
    }

    fn files() -> Files {
        Files {
            poms: vec!["pom.xml".into(), "common/pom.xml".into()],
            gradle_builds: vec!["app/build.gradle".into()],
            wrappers: vec!["mvnw".into(), "mvnw.cmd".into(), "app/gradlew".into()],
            jars: vec![
                "target/demo-1.0.jar".into(),
                "target/demo-1.0-sources.jar".into(),
                "target/demo-1.0.jar.original".into(),
                "app/build/libs/app-0.1.jar".into(),
                "app/build/libs/app-0.1-plain.jar".into(),
            ],
        }
    }

    #[test]
    fn detects_maven_and_gradle_modules() {
        let project = detect_with_reader("D:/proj", &files(), &reader());
        assert_eq!(project.modules.len(), 3);
        let root_module = project.modules.iter().find(|m| m.dir.is_empty()).unwrap();
        assert_eq!(root_module.build, Build::Maven);
        assert!(root_module.spring_boot);
        assert_eq!(root_module.name, "proj");
        assert_eq!(root_module.wrappers.mvnw_unix.as_deref(), Some("mvnw"));
        assert_eq!(root_module.wrappers.mvnw_cmd.as_deref(), Some("mvnw.cmd"));
        assert_eq!(root_module.boot_jar.as_deref(), Some("target/demo-1.0.jar"));
        assert!(project.has_spring_boot());

        let common = project.modules.iter().find(|m| m.dir == "common").unwrap();
        assert_eq!(common.build, Build::Maven);
        assert!(!common.spring_boot);
        // The wrapper is inherited from the root.
        assert_eq!(common.wrappers.mvnw_unix.as_deref(), Some("mvnw"));
        assert_eq!(common.boot_jar, None);

        let app = project.modules.iter().find(|m| m.dir == "app").unwrap();
        assert_eq!(app.build, Build::Gradle);
        assert!(app.spring_boot);
        assert_eq!(app.wrappers.gradlew_unix.as_deref(), Some("app/gradlew"));
        assert_eq!(app.boot_jar.as_deref(), Some("app/build/libs/app-0.1.jar"));
    }

    #[test]
    fn absolute_paths_are_made_relative() {
        let files = Files {
            poms: vec![r"D:\proj\pom.xml".into(), r"D:\proj\service\pom.xml".into()],
            gradle_builds: vec![],
            wrappers: vec![r"D:\proj\mvnw.cmd".into()],
            jars: vec![r"D:\proj\target\app.jar".into()],
        };
        let project = detect_with_reader(r"D:\proj", &files, &MapReader::new());
        assert_eq!(project.modules.len(), 2);
        let root = project.modules.iter().find(|m| m.dir.is_empty()).unwrap();
        assert_eq!(root.wrappers.mvnw_cmd.as_deref(), Some("mvnw.cmd"));
        assert_eq!(root.boot_jar.as_deref(), Some("target/app.jar"));
        assert_eq!(
            project.modules.iter().find(|m| m.dir == "service").unwrap().name,
            "service"
        );
    }

    #[test]
    fn files_outside_the_root_are_ignored() {
        let files = Files {
            poms: vec!["/other/pom.xml".into()],
            ..Files::default()
        };
        let project = detect_with_reader("/proj", &files, &MapReader::new());
        assert!(project.modules.is_empty());
    }

    #[test]
    fn spring_boot_markers() {
        assert!(pom_is_spring_boot(POM_SPRING));
        assert!(!pom_is_spring_boot(POM_PLAIN));
        assert!(gradle_is_spring_boot(GRADLE_SPRING));
        assert!(!gradle_is_spring_boot("plugins { id 'java' }"));
    }

    #[test]
    fn joining_paths_uses_the_root_style() {
        assert_eq!(join_path(r"D:\proj", "app/target"), r"D:\proj\app\target");
        assert_eq!(join_path("/proj", "app/target"), "/proj/app/target");
        assert_eq!(join_path("/proj/", ""), "/proj/");
        assert_eq!(join_path("D:/proj", "app"), "D:/proj/app");
    }

    #[test]
    fn relative_path_handles_odd_input() {
        assert_eq!(relative_path("/p", "/p/a/pom.xml").as_deref(), Some("a/pom.xml"));
        assert_eq!(relative_path("/p", "/p").as_deref(), Some(""));
        assert_eq!(relative_path("/p", "a/pom.xml").as_deref(), Some("a/pom.xml"));
        assert_eq!(relative_path("/p", "/q/a/pom.xml"), None);
        assert_eq!(relative_path("D:/p", r"D:\p\a\pom.xml").as_deref(), Some("a/pom.xml"));
        assert_eq!(relative_path("D:/p", r"d:\P\a\pom.xml").as_deref(), Some("a/pom.xml"));
        assert_eq!(relative_path("/p", r"\\?\C:\p\a").as_deref(), None);
    }
}
