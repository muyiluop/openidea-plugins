# OpenIDEA Rust plugin SDK — host API surface (recon)

Repo: `github.com/l0ng-ai/openidea` @ `main`, read via
`https://raw.githubusercontent.com/l0ng-ai/openidea/main/<path>` (the `refs/heads/main` form and
`cdn.jsdelivr.net` both failed intermittently; when raw 200s stopped, `https://api.github.com/repos/l0ng-ai/openidea/contents/<path>`
returned the same file as base64 — that is how §15's schema files were read).

Everything below is quoted from the upstream sources as of this fetch; nothing was compiled
(no network in the shell, `wasm32-wasip2` target absent). UNCONFIRMED items are marked.

> **Read order:** §1/§3/§4/§5/§6 (SDK) are current. **§2, §7, §8 were written before the repo tree
> and the official plugins were reachable — they are superseded by §10 (platform/OS), §11
> (run-configurations worked example), §12 (mock test recipe), §13 (repo/packaging facts,
> canonical skeleton), §14 (manifest grammar) and §15 (settings schema dialect, confirmed) below.**
> Where they disagree, §10–§17 win. §16 assembles the minimal shape for our plugin, §17 lists what
> is still uncertain.

---

## 1. Crate / version facts

`sdk/rust/openidea-plugin/Cargo.toml` — full file, verbatim:

```toml
[package]
name = "openidea-plugin"
version = "0.1.0"
edition = "2024"
description = "SDK for OpenIDEA plugins: typed host API, plugin trait, and a mock host for `cargo test`"
license = "Apache-2.0"
publish = false

# Standalone: plugins depend on it by path or version; not part of the IDE workspace.
[workspace]

[lib]
path = "src/lib.rs"

[target.'cfg(target_arch = "wasm32")'.dependencies]
wit-bindgen = "0.63"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
# Reads the WIT in tests/wit_coverage.rs (the version wit-bindgen already uses).
wit-parser = "0.262"
```

* Crate name `openidea-plugin`, version `0.1.0`, **edition 2024** (needs Rust ≥ 1.85).
* `publish = false`; the comment says plugins depend on it *by path or version*, but:
* **NOT on crates.io**: `https://crates.io/api/v1/crates/openidea-plugin` → HTTP 404
  `{"errors":[{"detail":"crate `openidea-plugin` does not exist"}]}`.
* ⇒ our plugin's `Cargo.toml` must use a **path dependency** (vendored SDK) or a **git dependency**
  (`git = "https://github.com/l0ng-ai/openidea"`, `rev`/`branch = "main"`). Neither builds offline
  in this sandbox; the Lead should pick `path` + vendored copy, or leave `git` for people with network.
  Caution: `[workspace]` in the SDK manifest means it is its own workspace root — a `path` dep on a
  directory that is inside another Cargo workspace also works, but do not add it under our root
  workspace's `members`.
* `wit-bindgen = "0.63"` only on `wasm32`; `serde`/`serde_json` on all targets (the SDK's data
  types use `serde::Serialize/Deserialize`).
* The SDK ships its own `wit/` directory *inside the crate* (`path = "wit"` in every
  `wit_bindgen::generate!` call in `src/lib.rs`) — a path dependency must therefore point at the
  whole `sdk/rust/openidea-plugin` directory, not a vendored `src/` copy. Note `tests/wit_coverage.rs`
  exists upstream (references `wit-parser`), so the vendored tree should include `wit/`.
* `main.rs`/`syntax.rs`… : `src/lib.rs` declares modules `commit, details, diagnostics, endpoints,
  file_actions, host, hover, indexing, inlay_hints, mock (cfg not wasm32), providers, semantic_tokens,
  syntax, text, types (private), views, execution (pub mod + pub use), language_features, injections`.
  `types` is `mod types;` + `pub use types::*;` (reach types through the crate root).

---

## 2. Canonical plugin skeleton (early notes — **superseded by §13**; `plugins/hello/` does not exist)

The jsDelivr flat listing used here was **stale**: it listed a `plugins/hello/` package
(`Cargo.toml` 334 B, `plugin.toml` 727 B, `src/lib.rs` 2533 B, `dist/plugin.wasm` 70241 B,
`Cargo.lock`) that is **not in the repo at HEAD**. Current `plugins/` content, the real per-plugin
file set and the verbatim `Cargo.toml`/`manifest` templates are in §13 and §14; the generated
`Cargo.toml` produced by `openidea-plugin new` is described in §13.

What held up: `dist/plugin.wasm` is the built component and is committed next to `plugin.toml`, i.e.
the package layout is `plugin.toml` + `dist/plugin.wasm` (+ optional `settings.schema.json`,
`locales/<tag>.json`, `languages/*.toml`, `Cargo.toml`, `src/`).

---

## 3. `src/types.rs` — plain data types (target-independent)

File header: `//! Plain data types of the API (independent of the target).`

### Error

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not granted; names the capability (`editor.write`).
    Denied(String),
    /// The document changed since the version the edit was made for.
    Stale,
    NotFound(String),
    Invalid(String),
    Cancelled,
    Failed(String),
}
```

`Display` (exact strings):

```rust
Self::Denied(c) => write!(f, "permission denied: {c} is not granted"),
Self::Stale => f.write_str("the document changed in the meantime"),
Self::NotFound(m) => write!(f, "not found: {m}"),
Self::Invalid(m) => write!(f, "invalid: {m}"),
Self::Cancelled => f.write_str("cancelled"),
Self::Failed(m) => f.write_str(m),
```

Also `impl std::error::Error for Error {}`, `impl From<Error> for String`, `impl From<String> for Error`
(→ `Failed`), `impl From<&str> for Error` (→ `Failed`). So `err.into()` / `ok_or("msg")?` gives
`Error::Failed`; doc on `run_command` says a plain message is `Error::Failed`, and "return
`Error::Cancelled` when the user gave up".

### Levels / positions / ranges

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel { Trace, Debug, Info, Warn, Error }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationLevel { Info, Warning, Error }

/// 0-based line and UTF-16 code unit column (as in the editor and LSP).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct Position { pub line: u32, pub character: u32 }
impl Position { pub const fn new(line: u32, character: u32) -> Self }

/// Half-open range; a cursor is an empty range.
pub struct Range { pub start: Position, pub end: Position }
impl Range {
    pub const fn new(start: Position, end: Position) -> Self;
    pub fn is_empty(&self) -> bool
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit { pub range: Range, pub text: String }
```

### Document / SymbolInfo / Location

```rust
pub struct Document { pub version: u64, pub text: String }
impl Document {
    /// The text of `range` (UTF-16 columns), if it lies inside the document.
    pub fn text_in(&self, range: Range) -> Option<&str>
}

/// A symbol from the host's current in-memory project index.
pub struct SymbolInfo {
    pub name: String,
    /// Host symbol kind in camelCase (`class`, `method`, `enumMember`, …).
    pub kind: String,
    pub container: Option<String>,
    pub location: Location,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Location { pub path: String, pub range: Range }   // path is absolute

pub struct OpenEditor { pub path: String, pub language: String, pub dirty: bool, pub active: bool }

/// Find in Files options. Include and exclude are project-relative globs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextQuery {
    pub pattern: String, pub regex: bool, pub case_sensitive: bool, pub whole_word: bool,
    pub include: Vec<String>, pub exclude: Vec<String>,
}
pub struct TextMatch { pub location: Location, pub line: String }       // zero-based UTF-16 coords
pub struct TextSearch { pub matches: Vec<TextMatch>, pub limit_hit: bool }
```

### Context (what `activate` receives)

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub plugin_id: String,
    pub plugin_version: String,
    /// Plugin API version the host implements.
    pub api_version: String,
    /// Capabilities granted to the plugin (`project.read`, …).
    pub capabilities: Vec<String>,
}
impl Context { pub fn has(&self, capability: &str) -> bool }
```

### UI value types

```rust
pub struct StatusItem { pub text: String, pub tooltip: Option<String>, pub command: Option<String> }
//  `command`: "One of the plugin's commands, run on click."

pub struct PickItem { pub key: String, pub label: String, pub description: Option<String> }
impl PickItem {
    pub fn new(key: &str, label: &str) -> Self
    pub fn description(mut self, description: &str) -> Self   // builder
}

/// A notification button invoking a command declared by this plugin.
pub struct NotificationAction { pub label: String, pub command: String, pub argument: Option<String> }
impl NotificationAction {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self
    pub fn argument(mut self, argument: impl Into<String>) -> Self
}

/// A context key's value, as `when` clauses compare it (host::set_context).
pub enum ContextValue { Flag(bool), Number(f64), /* at most 1024 bytes */ Text(String) }
// From<bool>, From<f64>, From<&str>, From<String> implemented.
```

### EventKind — all variants + `name()` strings (exact)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EventKind {
    FileClosed, SettingsChanged, InputModeChanged, RunExited, TerminalExited, ContentTabAction,
    FileOpened, FileSaved, FilesChanged, EditorFocused,
    /// Needs `events.input`.
    ActionPerformed,
    VcsChanged, Scheduled,
}

impl EventKind {
    /// The name the IDE knows it by.
    pub fn name(self) -> &'static str { /* one of: */ }
}
```

| variant | `name()` |
|---|---|
| `FileClosed` | `"file-closed"` |
| `SettingsChanged` | `"settings-changed"` |
| `InputModeChanged` | `"input-mode-changed"` |
| `RunExited` | `"run-exited"` |
| `TerminalExited` | `"terminal-exited"` |
| `ContentTabAction` | `"content-tab-action"` |
| `FileOpened` | `"file-opened"` |
| `FileSaved` | `"file-saved"` |
| `FilesChanged` | `"files-changed"` |
| `EditorFocused` | `"editor-focused"` |
| `ActionPerformed` | `"action-performed"` |
| `VcsChanged` | `"vcs-changed"` |
| `Scheduled` | `"scheduled"` |

### Event / ActionPerformed / FilesChanged

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionSource { Keyboard, Menu, Toolbar, ContextMenu, Search, /* anything else */ Other }
impl ActionSource { pub fn parse(name: &str) -> Self }  // "keyboard"|"menu"|"toolbar"|"context-menu"|"search"

pub struct ActionPerformed {
    pub action: String,            // action id (`editor.duplicate-line`)
    pub title: String,
    pub source: ActionSource,
    pub shortcut: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilesChanged { pub created: Vec<String>, pub modified: Vec<String>, pub removed: Vec<String> }

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    FileOpened(String),
    FileSaved(String),
    FilesChanged(FilesChanged),
    EditorFocused(Option<String>),
    ActionPerformed(ActionPerformed),
    /// A repository (its root) changed.
    VcsChanged(String),
    /// The `on-schedule` period elapsed.
    Scheduled,
    /// A kind this SDK doesn't know: its name and JSON data.
    Other { kind: String, data: String },
}

impl Event { pub fn from_other(kind: String, data: String) -> Self }
```

`Event::from_other` decodes `"files-changed"` payloads into `Event::FilesChanged`, otherwise keeps
`Other { kind, data }`.

### ProcessOutput

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessOutput {
    /// `None` when killed by a signal.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
```

### Other types in types.rs (name-only, for completeness)
`RepositoryStatus { root, branch: Option<String>, commit, upstream, ahead: u32, behind: u32, operation: Option<String>, changed_files: u32 }`,
`FileKind { File, Directory, Symlink, #[default] Other }`, `FileStat { kind, size: u64, modified: Option<u64> }`,
`CommitChange`, `CommitDraft`, `Severity { Error, Warning, Info, Hint }`, `Diagnostic` (+ builder
`Diagnostic::new(range, severity, message)` / `.code(c)`), `RelatedInformation`, `HttpRequest`
(+`get`/`post`/`header`), `HttpResponse` (+`text()`), `FileProblems`, `Decoration`,
`FileDecoration`, `CommitInfo`, `BlameLine`, `PluginDataSource` (`serde(rename_all = "camelCase")`),
`PluginRunConfiguration` (in `execution.rs`), `SymbolInfo`, `OpenEditor`.

---

## 4. `src/lib.rs` — Plugin trait, `register_plugin!`, `API_VERSION`

### API_VERSION

```rust
/// The plugin API version this SDK targets.
pub const API_VERSION: &str = "1.3";
```

**Inconsistency to note:** every `wit_bindgen::generate!` block in the same file imports
`openidea:plugin/types@1.4.0` / `invocation@1.4.0` / `workspace@1.4.0` / `editor@1.4.0`, and several
APIs are documented "(API 1.3)"/"(API 1.4)". The `pub const API_VERSION` still says `"1.3"`.

### Plugin trait (exact signatures)

```rust
/// A plugin. One instance lives as long as the plugin's Wasm instance (until
/// it is disabled, crashes or is reclaimed when idle).
pub trait Plugin: 'static {
    /// Called once when the instance is created: subscribe to events and
    /// restore state from [`host::storage`]. An error fails the activation.
    fn activate(&mut self, _ctx: &Context) -> Result<(), Error> { Ok(()) }

    /// Called before the instance is dropped.
    fn deactivate(&mut self) {}

    /// Runs one of the plugin's commands; `Ok(Some(text))` is shown to the user.
    /// A plain message (`Err("…".into())`, or `?` on a `String` error) is
    /// [`Error::Failed`]; return [`Error::Cancelled`] when the user gave up.
    fn run_command(&mut self, id: &str, ctx: &Invocation, arg: Option<&str>) -> Result<Option<String>, Error>;

    /// A batch of subscribed events (see [`host::subscribe`]), oldest first.
    fn handle_events(&mut self, _events: &[Event], _ctx: &Invocation) {}
}
```

Note `run_command` is the only required method; `Plugin: 'static`, the type must also be `Default`
(see the macro).

### Re-exports

```rust
pub use host::{Invocation, Project, Task, TextEditor};
pub use types::*;
pub use execution::*;          // at the bottom: `pub mod execution; pub use execution::*;`
```

### `register_plugin!` grammar

```rust
/// Exports `T` (a [`Plugin`] + `Default`) as the plugin's component. With
/// `syntax`, `T` is also a [`syntax::SyntaxProvider`] for its languages.
#[macro_export]
macro_rules! register_plugin {
    ($plugin:ty $(, $export:ident)* $(,)?) => { ... };
}
```

Grammar: `register_plugin!(MyPlugin)` or `register_plugin!(MyPlugin, export1, export2, ...)` with an
optional trailing comma. On non-wasm targets the macro only does
`const _: fn() -> $plugin = <$plugin as Default>::default;` — so names/type-checking of exports
happen only for wasm, but the *ident* must still be one of the known arms (otherwise no macro arm →
compile error). Known optional export idents (`__optional_exports!` arms, exact spelling):

```
injections, language_features, mcp, run_configurations, test_results,
providers, views, indexing, semantic_tokens, file_actions, endpoints, details,
diagnostics, hover, commit_checks, inlay_hints, syntax
```

For our plugin the relevant one is `run_configurations`:

```rust
macro_rules! __run_configurations_exports {
    ($plugin:ty, $component:ident) => {
        impl $crate::run_configurations_bindings::exports::openidea::plugin::run_configurations::Guest for $component {
            fn configurations(root: String) -> Vec<...::RunConfiguration> {
                $crate::__rt::configurations::<$plugin>(&root)
            }
        }
        $crate::run_configurations_bindings::__export_configurations_world!($component with_types_in $crate::run_configurations_bindings);
    };
}
```

The WIT world is `configurations-provider` (`world: "configurations-provider"`), export macro
`__export_configurations_world!`.

### The `__rt::configurations::<T>` glue (what crosses the boundary)

```rust
pub fn configurations<T: Plugin + crate::RunConfigurations + Default>(
    root: &str,
) -> Vec<crate::run_configurations_bindings::exports::openidea::plugin::run_configurations::RunConfiguration> {
    use crate::run_configurations_bindings::exports::openidea::plugin::run_configurations::RunConfiguration as C;
    with::<T, _>(|p| p.configurations(root))
        .into_iter()
        .map(|c| C { id: c.id, name: c.name, program: c.program, args: c.args, cwd: c.cwd,
                     test: c.test, framework: c.framework })
        .collect()
}
```

* Input: the **project root as a `String`** (`root`), nothing else — no Invocation, no capabilities
  argument, and the plugin instance is fetched through `with::<T, _>` (the single lazily-created
  `T: Default` instance). ⇒ `configurations()` cannot receive `&Invocation`; it may still call
  `host::*` functions (they operate on "the invoking window / every window when there is none").
* Output: `Vec<PluginRunConfiguration>` with fields **exactly** `id, name, program, args, cwd, test,
  framework` (see §4.1).
* `with::<T,_>` swallows panics? Not shown; it is `fn with<T: Plugin + Default, R>(f: impl FnOnce(&mut T) -> R) -> R`
  in the hidden `__rt` module. UNCONFIRMED whether a panic unwinds across the WIT boundary.

### 4.1 `src/execution.rs` — RunConfigurations trait + record

```rust
//! Runs, keyed terminals and plugin-owned content editors (API 1.4).
use serde::{Deserialize, Serialize};

/// PluginRunConfiguration data exchanged with the IDE.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRunConfiguration {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub test: bool,
    pub framework: Option<String>,
}

/// Supplies complete project configurations; register with run_configurations.
pub trait RunConfigurations {
    fn configurations(&mut self, project_root: &str) -> Vec<PluginRunConfiguration>;
}
```

`TestResults` (sibling trait, same file):

```rust
pub trait TestResults {
    fn parse(&mut self, config: &str, chunk: &str) -> Vec<PluginTestEvent>;
    /// Full replacement argv for test-tree name paths, or None when filtering is unsupported.
    fn select_tests(&mut self, _config: &str, _tests: &[Vec<String>]) -> Option<Vec<String>> { None }
}
```

Also in `execution.rs`: `ContentAction`, `ContentTab`, `ContentDiff`, `ContentTabReply`,
`RunExited { id, config, status: Option<i32>, killed }`, `TerminalExited { id, key, status, closed }`,
`ContentTabAction { request_id, key, action, content }`, test events (`PluginTestEvent` enum tagged
`type`, `TestStart/TestPassed/TestFailed/TestSkipped/TestOutput`), all `serde(rename_all = "camelCase")`,
and:

```rust
/// A plugin-owned argv wrapper for project runs (API 1.4).
pub struct PluginRunTarget {
    pub id: String, pub name: String, pub program: String,
    pub args: Vec<String>, pub workdir_args: Vec<String>,
    pub container_root: Option<String>, pub commands: Vec<(String, Vec<String>)>,
}
```

`Event` accessors defined in `execution.rs`:

```rust
impl crate::Event {
    /// Completion of a run started by this plugin, even after idle reclamation.
    pub fn run_exited(&self) -> Option<RunExited>
    /// Exactly one completion for a keyed terminal operation.
    pub fn terminal_exited(&self) -> Option<TerminalExited>
    /// A toolbar action or Save from this plugin's wasm content tab.
    pub fn content_tab_action(&self) -> Option<ContentTabAction>
}
```
(all three deserialize from `Event::Other { kind, data }` with `kind == "run-exited"` /
`"terminal-exited"` / `"content-tab-action"`, via `serde_json::from_str`).

---

## 5. `src/host.rs` — the free functions (capability-gated)

Module doc:

```rust
//! The IDE functions and handles a plugin may use. Gated ones need the
//! capability in `plugin.toml` (`[capabilities] required = [...]`) and the
//! user's grant; without it they return [`Error::Denied`].
```

Two impls: `mod imp` for `wasm32` (calls the `openidea:plugin@1.0.0` WIT world) and
`#[cfg(not(target_arch = "wasm32"))] use crate::mock::current as imp;`.

### 5.1 Settings (this is the read path for our settings)

```rust
/// One of the plugin's settings (`[contributes.settings]`) as JSON: the
/// user's value (project over global) or the schema's default.
pub fn setting(key: &str) -> Option<String> { imp::setting(key) }

/// A boolean setting.
pub fn setting_bool(key: &str) -> Option<bool> { setting(key).and_then(|v| v.parse().ok()) }

/// A string setting.
pub fn setting_string(key: &str) -> Option<String> {
    let v = setting(key)?;
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    Some(crate::text::unescape_json(inner))
}

/// A number setting.
pub fn setting_number(key: &str) -> Option<f64> { setting(key).and_then(|v| v.parse().ok()) }
```

* `setting()` returns the raw **JSON text** of the value (`imp::setting` → `wit::settings::get(key)`).
* `setting_bool` relies on JSON literal `true`/`false` parsing via `str::parse::<bool>`.
* `setting_string` requires the JSON to be a quoted string; unescapes with `text::unescape_json`.
* `setting_number` parses the raw JSON number text with `f64::from_str`.
* **There is no `setting_array`/`setting_object` helper and no "set" API** — read the JSON yourself
  with `serde_json` for arrays/objects. No per-key `settings namespace` beyond these 4 functions.
* Scoping: "the user's value (**project over global**) or the schema's default" — the precedence is
  handled by the host; the plugin cannot ask for a specific scope.
* Change notification: `EventKind::SettingsChanged` (`"settings-changed"`), subscribe in `activate`:
  `host::subscribe(EventKind::SettingsChanged)?` then handle in `handle_events`. Needs `events`.
  (Mock: `MockHost::set_setting`, and events are not auto-emitted — see §6.)

### 5.2 Storage

```rust
/// The plugin's persistent key-value store (≤ 64 MiB of UTF-8 keys and values).
/// Needs `storage`; rejected writes preserve the stored data.
pub mod storage {
    pub fn get(key: &str) -> Result<Option<String>, Error>
    pub fn set(key: &str, value: &str) -> Result<(), Error>
    pub fn delete(key: &str) -> Result<(), Error>
    /// Keys starting with `prefix`, sorted.
    pub fn keys(prefix: &str) -> Result<Vec<String>, Error>
}
```

Flat, plugin-global keyspace (no per-project scoping in the API); 64 MiB total. No JSON helpers —
`serde_json` yourself. Flat free functions also exist at `host::storage_get/set/delete/keys`
(`pub fn storage_get(key: &str) -> Result<Option<String>, Error>` etc.).

### 5.3 UI

```rust
pub fn log(level: LogLevel, message: &str)
/// Shows a notification balloon in the window the command or events came from
/// (every window when there is none).
pub fn notify(level: NotificationLevel, title: &str, body: &str)
/// [`notify`] with up to three buttons; each runs one of this plugin's declared commands in the
/// balloon's window and dismisses the balloon. An invalid action or an undeclared command is an
/// error and nothing is shown.
pub fn notify_with_actions(level: NotificationLevel, title: &str, body: &str,
                           actions: &[NotificationAction]) -> Result<(), Error>
/// Shows or updates the plugin's status bar item `id` (at most 8).
pub fn set_status_item(id: &str, item: &StatusItem)
pub fn remove_status_item(id: &str)
/// Overrides an item in the invoking window, until removed or the window/plugin closes.
pub fn set_window_status_item(id: &str, item: &StatusItem) -> Result<(), Error>
pub fn remove_window_status_item(id: &str) -> Result<(), Error>
/// Copies `text` to the system clipboard. Needs `clipboard`.
pub fn copy_to_clipboard(text: &str) -> Result<(), Error>
/// Opens an `http://` or `https://` URL in the user's default browser.
pub fn open_url(url: &str) -> Result<(), Error>
/// Shows a list and waits for the chosen key, or `None` when dismissed. ... Needs no capability.
pub fn pick(title: &str, items: &[PickItem]) -> Result<Option<String>, Error>
/// Like [`pick`], with any number of items chosen.
pub fn pick_many(title: &str, items: &[PickItem]) -> Result<Option<Vec<String>>, Error>
/// Asks for a line of text, pre-filled with `value`; `None` when dismissed. Same rules as [`pick`].
pub fn input(prompt: &str, value: Option<&str>) -> Result<Option<String>, Error>
/// Asks for text over several lines ... confirms on Cmd/Ctrl+Enter.
pub fn input_multiline(prompt: &str, value: Option<&str>) -> Result<Option<String>, Error>
/// Asks for confirmation: `true` on OK, `false` on Cancel or Escape.
pub fn confirm(title: &str, body: &str) -> Result<bool, Error>
/// Chooses a directory on the UI machine. `None` when dismissed; no capability.
pub fn pick_directory(title: &str, initial: Option<&str>) -> Result<Option<String>, Error>
/// Brings one of the plugin's declared views to the front ...
pub fn reveal_view(view: &str) -> Result<(), Error>
/// The IDE's UI language as a BCP 47 tag (`en`, `fr`, `zh-CN`, …) ...
pub fn locale() -> String
/// Sets the plugin's context key that `when` clauses test as `plugin.<id>.<key>` ...
/// The key is 1-64 of `A-Za-z0-9_-`; at most 100 keys.
pub fn set_context(key: &str, value: impl Into<ContextValue>) -> Result<(), Error>
pub fn remove_context(key: &str)
/// Requests the normal File > Open flow ... (`false`) or a new window (`true`). No capability.
pub fn open_project(path: &str, new_window: bool) -> Result<(), Error>
```

`pick`/`input`/`confirm` semantics worth copying into UI text: "Works in a command/event of a
window, or while resolving a selected code or file action in its editor's window; `Error::Invalid`
otherwise. Waiting doesn't count against the call's budget, but the plugin handles nothing else
meanwhile." `input_multiline` (and `open_terminal`, `open_log_tab`) are "only from a command or
event of a window".

Budget: "While a task is open, the current call may run for up to 60 s instead of 5 s."

### 5.4 Project / files / process

```rust
/// A project open in the IDE.
pub struct Project { inner: imp::ProjectRes }
impl Project {
    /// Absolute path of the root. Needs `project.read`.
    pub fn root(&self) -> Result<String, Error>
    /// Reads a UTF-8 file inside the project (absolute or root-relative); unsaved editor contents win.
    pub fn read_file(&self, path: &str) -> Result<String, Error>          // project.read
    /// Replaces the text of a file inside the project ... at most 16 MiB ... Needs `project.write`.
    pub fn write_file(&self, path: &str, text: &str) -> Result<(), Error>
    /// Whether `path` names an existing file or directory inside the project — including hidden
    /// and ignored ones — without reading any content. Needs `project.read`.
    pub fn exists(&self, path: &str) -> Result<bool, Error>
    pub fn stat(&self, path: &str) -> Result<FileStat, Error>
    /// Project files whose root-relative path matches `glob` (`**/*Mapper.xml`), at most `limit`,
    /// as absolute paths; ignored files are skipped. Needs `project.read`.
    pub fn find_files(&self, glob: &str, limit: u32) -> Result<Vec<String>, Error>
    /// Find in Files ... Requires project.read; ... a 10,000 match cap.
    pub fn search_text(&self, query: &TextQuery, limit: u32) -> Result<TextSearch, Error>
    /// Fuzzy search of this project's current in-memory index. Needs `project.read`.
    pub fn symbols(&self, query: &str, limit: u32) -> Result<Vec<SymbolInfo>, Error>
    /// Runs the approved `process:<program>` in this project's root.
    pub fn run_process(&self, program: &str, args: &[String], stdin: Option<&str>) -> Result<ProcessOutput, Error>
}

/// Projects open in any window. Needs `project.read`.
pub fn projects() -> Result<Vec<Project>, Error>
/// The deepest open project containing an absolute document path, including in providers.
pub fn project_of(path: &str) -> Option<Project>

pub mod fs {
    /// The plugin's private persistent directory (0700, owner-only ACL on Windows), removed on uninstall.
    pub fn plugin_dir() -> Result<String, Error>
    pub struct DirEntry { pub name: String, pub kind: FileKind }
    /// A text file, decoded as the editor decodes it, at most 16 MiB; unsaved editor contents win.
    pub fn read_file(path: &str) -> Result<String, Error>
    /// The bytes of a file ..., at most 16 MiB.
    pub fn read_bytes(path: &str) -> Result<Vec<u8>, Error>
    pub fn stat(path: &str) -> Result<FileStat, Error>
    /// The entries directly under a directory, sorted by name, at most 4096.
    pub fn read_dir(path: &str) -> Result<Vec<DirEntry>, Error>
    /// Replaces the text of a file, or creates it, at most 16 MiB ...
    pub fn write_file(path: &str, text: &str) -> Result<(), Error>
    pub fn write_bytes(path: &str, bytes: &[u8]) -> Result<(), Error>
}
```

`fs` doc: "Files outside the open projects... Outside `fs::plugin_dir`, paths need an
`fs.read:<path>` (reading) or `fs.write:<path>` (writing) grant covering it... Paths are absolute,
or start with `~/` for the user's home."

Process (the key primitive for a "run JVM with these args" plugin):

```rust
/// A program with its arguments and the environment added to the inherited one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// Variables added to the inherited environment: at most 64, names at most 256 bytes,
    /// values at most 4 KiB, no NUL or '=' in names, no NUL in values. ... Variables are never
    /// replaced or removed.
    pub env: Vec<(String, String)>,
}
impl Command {
    /// Selects the program approved by a `process:<program>` grant.
    pub fn new(program: &str) -> Self
    pub fn arg(mut self, arg: &str) -> Self
    pub fn args<I, S>(mut self, args: I) -> Self where I: IntoIterator<Item = S>, S: AsRef<str>
    /// Adds a variable; an inherited or duplicate name is an error at spawn.
    pub fn env(mut self, name: &str, value: &str) -> Self
}

/// Runs a [`Command`] to its end (API 1.3). Its environment only adds new variables;
/// an existing key fails with an error naming it.
pub fn run_command(cmd: &Command, project: Option<&Project>, stdin: Option<&str>) -> Result<ProcessOutput, Error>
/// Starts a [`Command`] and keeps it running behind a [`Child`] (at most 8 per plugin instance,
/// API 1.3). Closing the invoking window ends its processes.
pub fn start(cmd: &Command, project: Option<&Project>) -> Result<Child, Error>
/// Runs the approved `process:<program>` in the plugin's data directory
/// (see [`Project::run_process`] to run it in a project).
pub fn run_process(program: &str, args: &[String], stdin: Option<&str>) -> Result<ProcessOutput, Error>

pub struct Child { /* private */ }
impl Child {
    /// What it printed since the last read, up to 1 MiB per stream ...; `status` is `Some` once it ended.
    pub fn poll(&self, max_bytes: u32) -> Result<Chunk, Error>
    pub fn write(&self, text: &str) -> Result<(), Error>
    pub fn close_stdin(&self) -> Result<(), Error>
    /// Ends it; `status` becomes known right away (-1 when a signal ended it).
    pub fn kill(&self) -> Result<(), Error>
    /// Waits up to `timeout_ms` for it to end (0 only checks), then reads it.
    pub fn wait(&self, timeout_ms: u32) -> Result<Chunk, Error>
}
pub struct Chunk { pub stdout: String, pub stderr: String, pub status: Option<i32> }
```

**Environment variables exist** (`Command::env`) but only *add* new names: "Existing names in the
prepared child environment (including login-shell PATH) fail with an error naming the key. Windows
names compare without case. Variables are never replaced or removed." (This matters for
`JAVA_TOOL_OPTIONS`-style tricks — you can set them only if not already present.)

`Task` (status-bar progress, extends the call budget to 60 s):

```rust
/// Long-running work shown in the status bar; dropping it ends it.
pub struct Task { /* private */ }
impl Task {
    /// `done` of `total` units (`None`: indeterminate), with a short status.
    pub fn report(&self, done: u32, total: Option<u32>, message: Option<&str>)
    /// The user cancelled it: stop soon and return.
    pub fn is_cancelled(&self) -> bool
}
/// Starts a task (see [`Task`]).
pub fn begin_task(title: &str, cancellable: bool) -> Task
```

### 5.5 Invocation (what a command/event gets)

```rust
/// Where a command or event batch comes from: the window's project and focused editor.
pub struct Invocation<'a> { inner: imp::InvocationRef<'a> }
impl<'a> Invocation<'a> {
    /// The window's project.
    pub fn project(&self) -> Option<Project>
    /// The window's focused editor.
    pub fn editor(&self) -> Option<TextEditor>
    /// The files and folders the command was run on, as absolute paths. Empty when it was not
    /// run on any, or when the menu was for a tab without a file (a virtual document).
    pub fn selected_paths(&self) -> Vec<String>
}
```

⇒ project root: `ctx.project().ok_or("no project")?.root()?`. There is **no** `ctx.project_root()`.

`TextEditor`:

```rust
impl TextEditor {
    pub fn path(&self) -> String
    pub fn language(&self) -> String
    pub fn document(&self) -> Result<Document, Error>          // editor.read
    pub fn selections(&self) -> Result<Vec<Range>, Error>      // editor.read
    /// ... in `version`'s coordinates, not overlapping ... Returns the new version. Needs `editor.write`.
    pub fn apply_edits(&self, version: u64, edits: &[TextEdit]) -> Result<u64, Error>
    /// Replaces every selection with `f(selected text)` in one undoable change, retrying once ...
    pub fn transform_selections(&self, mut f: impl FnMut(&str) -> String) -> Result<u64, Error>
    pub fn set_selections(&self, ranges: &[Range]) -> Result<(), Error>
    pub fn reveal(&self, range: Range) -> Result<(), Error>
}
```

### 5.6 Events

```rust
/// Delivers `kind` events to [`crate::Plugin::handle_events`] for the rest of the session
/// (also after the instance is reclaimed: an event activates the plugin again).
/// Needs `events` (`events.input` for `ActionPerformed`).
pub fn subscribe(kind: EventKind) -> Result<(), Error>
pub fn unsubscribe(kind: EventKind)
/// Subscribe to project-relative path globs for file-opened, file-saved or files-changed.
/// Replaces the previous filter. Empty globs pause delivery; `subscribe` restores all paths.
pub fn subscribe_paths(kind: &str, globs: &[&str]) -> Result<(), Error>
```

### 5.7 Run / terminal / misc (API 1.4, from host.rs)

```rust
/// Starts an owned run configuration in the invoking window. Its id receives run-exited.
pub fn start_run(config: &str, args: &[String], debug: bool) -> Result<String, Error>
/// Registers/replaces a read-only project target. Requires the program's process grant.
pub fn register_run_target(project: &Project, target: &crate::PluginRunTarget) -> Result<(), Error>
pub fn unregister_run_target(project: &Project, id: &str)
/// Opens a terminal tab ... running the approved `process:<program>` in the project root.
pub fn open_terminal(title: &str, program: &str, args: &[String]) -> Result<(), Error>
/// Opens/reuses a keyed, approved terminal with append-only environment variables.
pub fn open_terminal_session(key: &str, title: &str, cmd: &Command) -> Result<String, Error>
pub fn write_terminal(key: &str, text: &str) -> Result<(), Error>
/// Opens (or focuses) the read-only log tab `key` ...
pub fn open_log_tab(key: &str, title: &str, log: &crate::details::ProcessLog) -> Result<(), Error>
pub fn open_location(path: &str, line: u32, column: u32) -> Result<(), Error>
/// Executes a stable action or plugin command in the invoking window (API 1.4).
/// Requires high-risk `ide.commands`, except for this plugin's declared commands.
pub fn execute_command(id: &str, arg: Option<&str>) -> Result<Option<String>, Error>
/// Appends to a plugin-owned Output tab's 1 MiB ring buffer.
pub fn output_append(channel: &str, text: &str) -> Result<(), Error>
pub fn output_clear(channel: &str) -> Result<(), Error>
pub fn output_reveal(channel: &str) -> Result<(), Error>
pub fn publish_diagnostics(path: &str, items: &[Diagnostic]) -> Result<(), Error>
pub fn fetch(request: &HttpRequest) -> Result<HttpResponse, Error>
pub fn secret_set(name: &str, value: &str) -> Result<(), Error>
pub fn secret_delete(name: &str) -> Result<(), Error>
pub fn secret_is_set(name: &str) -> bool
/// Stdio bridge argv for the invoking project, none while the built-in MCP server is off.
pub fn mcp_stdio_command() -> Option<Vec<String>>
```

---

## 6. `src/mock.rs` — native test host (`cargo test` on the host target)

`#[cfg(not(target_arch = "wasm32"))] pub mod mock;`. Module doc:

```rust
//! A mock IDE for testing plugins with `cargo test` on the host target.
//!
//! [`MockHost::install`] makes the [`crate::host`] functions of the current
//! thread talk to a fresh mock: it records what the plugin did, keeps one
//! editor with a versioned document, and enforces the capabilities you give
//! it like the real host (a missing one returns [`Error::Denied`]).
```

Thread-local: `static CURRENT: RefCell<Option<Rc<RefCell<State>>>>` — `MockHost::install()` installs it
for the current thread; `Drop` uninstalls. Calling any `host::` fn without an installed mock panics:
`"no mock host: call openidea_plugin::mock::MockHost::install() first"`.

### 6.1 Recipe (compiles as written, from the SDK's own docs/tests)

```rust
use openidea_plugin::{mock::MockHost, host, Error, Invocation, Plugin, register_plugin};

#[test]
fn configurations_finds_spring_boot() {
    let mock = MockHost::install();          // grants ALL caps but NOT process:/net:/fs.read:
    mock.add_file("pom.xml", "<project/>");  // a project file Project::read_file/find_files can see
    mock.add_dir("src");                     // a project dir exists()/stat() report
    mock.set_setting("jvm.vmOptions", "\"-Xmx1g -Dfoo=bar\"");   // JSON text, see 6.2
    mock.set_project_root(Some("/project")); // default is already Some("/project")

    let cfg = MyPlugin::default().configurations("/project");    // RunConfigurations::configurations
    assert_eq!(cfg[0].program, "java");
    assert!(cfg[0].args.iter().any(|a| a == "-Xmx1g"));
}

#[test]
fn command_uses_the_host() {
    let mock = MockHost::install();
    mock.open_editor("/project/a.txt", "text", "one two", &[(0, 0, 0, 3)]);  // (line,col,line,col)+
    let mut plugin = MyPlugin::default();
    plugin.run_command("shout", &mock.invocation(), None).unwrap();
    assert_eq!(mock.editor_text(), "ONE TWO");
    assert_eq!(mock.notifications()[0].title, "t");   // if the command notified
    host::notify(openidea_plugin::NotificationLevel::Info, "t", "b");
}
```

`mock.invocation()` returns `Invocation<'static>`; the mock's project root defaults to `/project`.

### 6.2 Setup API (constructor + fakes)

```rust
impl MockHost {
    /// Installs a mock granting the usual capabilities, with project `/project`.
    pub fn install() -> Self
    /// Grants exactly these capabilities.
    pub fn with_capabilities(self, caps: &[&str]) -> Self        // whole list replaced
    /// The invoking window's project (`None`: no project).
    pub fn set_project_root(&self, root: Option<&str>)
    pub fn add_project(&self, root: &str)                        // extra projects for host::projects()
    pub fn add_file(&self, path: &str, text: &str)               // project file (rel or abs)
    pub fn add_dir(&self, path: &str)
    pub fn add_external_file(&self, path: &str, text: &str)      // fs::* outside projects
    pub fn select_paths(&self, paths: &[&str])                   // Invocation::selected_paths
    pub fn set_symbols(&self, root: &str, symbols: Vec<SymbolInfo>)
    pub fn open_editor(&self, path: &str, language: &str, text: &str, selections: &[(u32,u32,u32,u32)])
    pub fn type_in_editor(&self, text: &str)                     // user types: version += 1
    /// The value of one of the plugin's settings, as JSON (`true`, `"text"`).
    pub fn set_setting(&self, key: &str, json: &str)
    pub fn set_locale(&self, tag: &str)                          // host::locale(), default "en"
    pub fn set_vcs_status(&self, status: Option<RepositoryStatus>)
    pub fn set_vcs_reads(&self, commits: Vec<CommitInfo>, path: &str, blame: Vec<BlameLine>, patch: Option<String>)
    pub fn set_vcs_remotes(&self, remotes: &[(&str, &str)])
    pub fn set_restricted(&self, restricted: bool)
    pub fn set_commit_draft(&self, draft: Option<CommitDraft>)
    pub fn edit_commit_message(&self, message: &str)             // revision += 1
    pub fn fail_fetch(&self, error: Option<Error>)
    pub fn answer_next_prompt(&self, answer: Option<&[&str]>)    // pick/input/confirm answers
    pub fn cancel_tasks(&self)
    /// What running `process:<program>` does: `(args, stdin) → output`.
    pub fn on_process(&self, program: &str,
                      handler: impl Fn(&[String], Option<&str>) -> Result<ProcessOutput, Error> + 'static)
    pub fn on_search(&self, handler: impl Fn(&TextQuery, u32) -> Result<TextSearch, Error> + 'static)
    pub fn on_command(&self, id: &str, handler: impl Fn(Option<&str>) -> Result<Option<String>, Error> + 'static)
    pub fn on_own_command(&self, id: &str, handler: impl Fn(...) -> ... + 'static)  // plugin's own cmd
    pub fn on_fetch(&self, handler: impl Fn(&HttpRequest) -> Result<HttpResponse, Error> + 'static)
    pub fn declare_secret(&self, name: &str)
    pub fn set_secret(&self, name: &str, value: &str)
    pub fn set_mcp_stdio_command(&self, command: Option<Vec<String>>)
    pub fn set_run_configurations(&self, configs: Vec<crate::PluginRunConfiguration>)
    pub fn queue_loopback_query(&self, query: Option<&str>)
    /// A context as `activate` receives it.
    pub fn context(&self, plugin_id: &str) -> Context
}
```

### 6.3 Assertions (logs / notifications / commands / runs)

```rust
    pub fn notifications(&self) -> Vec<Notification>                      // {level, title, body}
    pub fn notifications_with_actions(&self) -> Vec<ActionNotification>  // {notification, actions}
    pub fn logs(&self) -> Vec<(LogLevel, String)>
    pub fn status_items(&self) -> BTreeMap<String, StatusItem>
    pub fn context_keys(&self) -> BTreeMap<String, ContextValue>
    pub fn storage(&self) -> BTreeMap<String, String>
    pub fn subscriptions(&self) -> BTreeSet<EventKind>
    pub fn path_subscriptions(&self) -> BTreeMap<String, Vec<String>>
    pub fn tasks(&self) -> Vec<MockTask>          // {title, reports: Vec<(u32,Option<u32>,Option<String>)>, cancelled, ended}
    pub fn editor_text(&self) -> String
    pub fn editor_version(&self) -> u64
    pub fn clipboard(&self) -> Option<String>
    pub fn opened_urls(&self) -> Vec<String>
    pub fn opened_projects(&self) -> Vec<(String, bool)>
    pub fn opened_locations(&self) -> Vec<(String, u32, u32)>
    pub fn revealed_views(&self) -> Vec<String>
    pub fn revealed_ranges(&self) -> Vec<Range>
    pub fn diagnostics(&self) -> BTreeMap<String, Vec<Diagnostic>>
    pub fn provider_diagnostics(&self) -> BTreeMap<String, Vec<Diagnostic>>
    pub fn output(&self) -> BTreeMap<String, String>
    pub fn requests(&self) -> Vec<HttpRequest>
    pub fn fetches(&self) -> u32
    pub fn terminal_writes(&self) -> Vec<(String, String)>
    pub fn terminals(&self) -> Vec<MockTerminal>          // {title, program, args}
    pub fn log_tabs(&self) -> Vec<MockLogTab>             // {key, title, log: ProcessLog}
    pub fn run_targets(&self) -> BTreeMap<(String, String), crate::PluginRunTarget>
    pub fn configuration_runs(&self) -> Vec<(String, String, Vec<String>, bool)>  // (op id, config id, args, debug)
    pub fn process_runs(&self) -> Vec<ProcessRun>         // {program, args, env, cwd: Option<String>}
    pub fn commit_draft(&self) -> Option<CommitDraft>
    pub fn decorations(&self) -> BTreeMap<(String, String), Vec<Decoration>>
    pub fn file_decorations(&self) -> BTreeMap<(String, String), FileDecoration>
    pub fn refreshed_views(&self) -> Vec<String>
    pub fn view_messages(&self) -> Vec<(String, String)>
    /// Simulates exit, releasing the key for another operation.
    pub fn finish_terminal(&self, key: &str) -> Option<crate::Event>
    pub fn provider_document(&self) -> Option<crate::providers::Document>
    pub fn refresh_diagnostics(&self, provider: &mut impl crate::diagnostics::DiagnosticsProvider)
```

Public mock structs: `Notification { level, title, body }`,
`ActionNotification { notification, actions }`, `MockTask`, `ProcessRun`, `MockTerminal`,
`MockLogTab`.

### 6.4 Capability behaviour in the mock (exact)

```rust
/// Every capability a plugin can hold, except `process:`/`net:`/`fs.read:`.
const ALL: &[&str] = &[
    "ide.commands", "database.sources", "project.read", "editor.read", "editor.write",
    "vcs.read", "vcs.write", "storage", "events", "events.input", "clipboard",
];
```

* `install()` → capabilities = `ALL`, project root `/project`.
* `with_capabilities(&[])` → everything gated returns `Error::Denied("<cap>")` (e.g.
  `host::storage::get` → `Denied("storage")`, `subscribe(ActionPerformed)` → `Denied("events.input")`).
* `process:<program>` is required *per program*: `host::run_process("java", …)` under a mock that
  lacks `process:java` → `Denied("process:java")`; with it, and no `on_process` handler →
  `NotFound("java (MockHost::on_process)")`. Use `with_capabilities(&["process:java", "project.read"])`.
* `run_process` (no `Command`) records `env: Vec::new()`; `run_command`/`start` record the env and
  the mock **prepends `KEY=VALUE\n` lines to the returned stdout** (so a test can assert the env
  arrived), and enforce ≤64 vars / ≤256-byte names / ≤4 KiB values / no dup names.
* `MAX_CHILDREN = 8` for `host::start`.
* Mock's `fs::plugin_dir()` is `/openidea/plugin-data/mock/files` (private, no grants needed);
  traversal outside it is denied even with an explicit grant.
* Mock `setting(key)` is literally `state.settings.get(key).cloned()` — **no schema defaults, no
  project/global precedence**; a test must `set_setting(...)` for every key it reads.
  `host::setting_bool("x")` with `set_setting("x", "true")` → `Some(true)`.

---

## 7. Settings schema dialect (early notes — **superseded by §15, which is CONFIRMED**)

Early hypotheses, kept only for provenance:

* `host::setting(key)` doc: "One of the plugin's settings (`[contributes.settings]`) as JSON: the
  user's value (**project over global**) **or the schema's default**." ⇒ the JSON file is a schema
  that also carries defaults, and the IDE can scope a value per project vs global.
* `setting_string()` expects a JSON **string** (strips `"`…`"`, unescapes); `setting_bool()` expects
  a bare `true`/`false`; `setting_number()` a bare number. So values are stored as JSON scalars, and
  arrays/objects come back as JSON text.
* Was UNCONFIRMED at the time: JSON Schema draft-07 with `x-` UI extensions vs VS Code-style
  `contributes.configuration` vs a bespoke widget vocabulary.
* **Answer: see §15.1–§15.2 — JSON-Schema-shaped subset + one `group` UI extension, no draft id.**

---

## 8. Manifest grammar / contributions

Early notes — **superseded by §14, which is confirmed from real manifests** (and there is no
`plugins/hello/` at HEAD; see §13):

* `[capabilities] required = [...]` — exact key per `host.rs` module doc.
* `[contributes.settings]` is a **table** with a `schema` key pointing at a JSON file.
* `contributes.mcpTools` (from `lib.rs`: "Implements tools declared in contributes.mcpTools").
* `[[contributes.secrets]]` ("declared secret ... declared under `[[contributes.secrets]]`").
* Views: `host::reveal_view("panel")`, `post_view_message(view, message)`; `views::Views` trait.

---

## 9. Gaps / risks for a plugin like ours

1. **No env-var replacement** — `Command::env` only *adds* names; an already-present name fails the
   spawn ("Variables are never replaced or removed"). So `JAVA_TOOL_OPTIONS` can only be set if the
   IDE has not already put it in the child env.
2. **No `Invocation` in `configurations()`** — the export receives only `project_root: &str`. Any
   project state must be re-derived from the filesystem/`host::` calls inside the call.
3. **No way to mutate built-in run configurations.** The plugin can only (a) *return* its own
   configurations, and (b) register an argv wrapper (`PluginRunTarget`: program/args/workdir_args/
   container_root/commands) for project runs. There is no API to read or edit the IDE's Maven/Spring
   run configs. (See task-2 `.research/api-run.md` for the run side.)
4. **Processes need explicit `process:<program>` grants** (per program, e.g. `process:java`,
   `process:./mvnw`?). Whether a relative wrapper path like `./mvnw` is grantable, and how the
   capability string is normalized, is UNCONFIRMED.
5. **`Project::run_process` has no env** (`program, args, stdin` only) — use `host::run_command` with
   `Command::env` and `Some(&project)` if env is needed.
6. **Storage is plugin-global**, 64 MiB, no project scoping; per-project state must be keyed by path.
7. **Settings read path is 4 scalar helpers**; arrays (e.g. a list of VM options) must be parsed by
   the plugin with `serde_json` from `host::setting(key)`'s raw JSON.
8. **Version skew**: `API_VERSION = "1.3"` while the crate imports WIT `@1.4.0`; `engines.openidea`
   in the manifest must therefore probably be `^1.3`/`^1.4` — confirm against README (§8).
9. **SDK is not on crates.io** ⇒ no `openidea-plugin = "0.1"` dependency; need path or git dep.
10. **Docs are thin on generation**: no `openidea-plugin new` template found in the enumerable tree
    (the listing truncated before `sdk/`, `tools/`, `wit/`); `plugins/README.md` mentions
    `openidea-plugin build` (per `lib.rs` doc: "Build with `openidea-plugin build` (or `cargo build
    --release --target wasm32-wasip2`)").

---

# PART II — verified after enumerating `sdk/rust/openidea-plugin/wit`, `plugins/official/*` and `plugins/README.md`

## 10. Platform / OS detection — **THERE IS NONE** (answer to Lead Q1)

Grepped every WIT interface that could carry it — `host.wit` (log, ui, storage, command-runner,
output), `io.wit` (diagnostics, net, secrets, process, ipc, auth), `workspace.wit` (workspace, editor,
invocation, vcs), `types.wit`, `fs.wit`, `events.wit`, `run.wit` — plus all of `sdk/rust/openidea-plugin/src/host.rs`:

* **No `platform()`, no `os`, no `windows`, no `path-style`, no separator helper, no
  `target_os`/`cfg!` in any import.** `host.rs` mentions Windows only incidentally:
  `src/host.rs:1123` ("Windows names compare without case" — env vars), `src/host.rs:1433`
  ("owner-only ACL on Windows" — `fs::plugin_dir`), `src/host.rs:1575` ("Windows named pipe" — `ipc`).
* The WIT file set is: commit, database, decorations, details, diagnostics, endpoints, events,
  exports, file-actions, fs, host, hover, indexing, injections, inlay, io, language-features, mcp,
  plugin (world), providers, run-targets, run, semantic, types, views, workspace. **No `ui.wit`** —
  the `ui` interface lives inside `host.wit`. Nothing OS-shaped anywhere.
* `Context` (`types.wit`) = `plugin-id`, `plugin-version`, `api-version`, `capabilities`. Nothing else.

**Consequences for us (real design constraint):**
1. A WASM component cannot see the desktop OS. Do **not** branch on `cfg!(target_os)` (it compiles
   the wasip2 value) and do not ship both `mvnw`/`mvnw.cmd` based on a guess.
2. **Recommended probe** — let the filesystem decide, it is both correct and portable:
   `project.exists("mvnw.cmd")` / `project.exists("mvnw")` / `project.exists("gradlew.bat")` /
   `project.exists("gradlew")`, then `project.stat(..).kind` if needed. `exists`/`stat` need
   `project.read`, "include hidden and ignored ones", and accept root-relative paths.
3. Secondary (weak, UNCONFIRMED) signals: `fs::plugin_dir()` returns an absolute path, so a drive
   letter implies a Windows desktop — but for a **remote** project the process runs on the *server*
   (`process` doc: "in `cwd`'s root"; `ipc` speaks of "the project machine"), so the desktop OS is
   the wrong question anyway. Prefer the file probe.
4. `host::locale()` gives a BCP 47 UI language only — not OS.

## 11. `register_plugin!` + run configurations — worked example (answer to Lead Q2)

Real upstream plugin `plugins/official/test-runner/` (API 1.4 example) uses exactly our feature.

`plugins/official/test-runner/src/lib.rs`, verbatim excerpt:

```rust
use openidea_plugin::{
    Error, Invocation, Plugin, PluginRunConfiguration, PluginTestEvent, RunConfigurations, TestFailed,
    TestPassed, TestResults, TestStart, host, register_plugin,
};

#[derive(Default)]
pub struct TestRunner { pending: String }

impl Plugin for TestRunner {
    fn run_command(&mut self, _id: &str, _ctx: &Invocation, arg: Option<&str>) -> Result<Option<String>, Error> {
        let args = arg.into_iter().map(str::to_owned).collect::<Vec<_>>();
        host::start_run("tests", &args, false).map(Some)
    }
}

impl RunConfigurations for TestRunner {
    fn configurations(&mut self, _root: &str) -> Vec<PluginRunConfiguration> {
        vec![PluginRunConfiguration {
            id: "tests".into(),
            name: "Example tests".into(),
            program: "openidea-example-tests".into(),
            test: true,
            framework: Some("plugin".into()),
            ..Default::default()
        }]
    }
}

register_plugin!(TestRunner, run_configurations, test_results, language_features);
```

Note `..Default::default()` for `args`/`cwd` — `PluginRunConfiguration` derives `Default`.

`plugins/official/test-runner/plugin.toml`, verbatim:

```toml
id = "openidea.test-runner"
name = "Test Runner (API Example)"
version = "0.1.0"
description = "Small plugin API 1.4 example; see the package README."
authors = ["OpenIDEA contributors"]
license = "Apache-2.0"
engines.openidea = "^1.4"
wasm = "dist/plugin.wasm"
activation = ["on-command:*"]

[capabilities]
required = ["process:openidea-example-tests"]
[[contributes.commands]]
id = "test"
title = "Example: Run Tests"
[[contributes.runConfigurations]]
files = ["example.tests"]
[[contributes.providers]]
kinds = ["gutter-actions"]
languages = ["plaintext", "text"]
```

**Manifest stanza:** `[[contributes.runConfigurations]]` with a single key **`files`** — "marker
files for the optional `run-configurations` export (API 1.4)" per the README contribution table.
Upstream comment in the plugin README: "Copy `example.tests` to a trusted project. **Its presence
enables** the dynamic `run-configurations` export." (i.e. the IDE only calls `configurations()` when
one of the marker files exists.)

**Host-side contract** (`wit/run.wit`, verbatim):

```wit
/// Optional dynamic configurations, refreshed alongside run detectors.
interface run-configurations {
    /// A configuration running an approved process in the project sandbox.
    record run-configuration {
        id: string, name: string, program: string, args: list<string>,
        cwd: option<string>, test: bool, framework: option<string>,
    }
    /// Returns the complete set for this project root.
    configurations: func(project-root: string) -> list<run-configuration>;
}
```

* **Refresh trigger:** "refreshed alongside run detectors" (WIT) and the README table row for
  `RunConfigurations::configurations(root)`: "Full project list, **refreshed on configuration
  listing**." ⇒ the call returns the *complete* list each time; there is no add/remove. Trigger it
  by changing a run detector's marker file (`files`), so a project that gains `pom.xml`/`mvnw`
  re-lists.
* **Program resolution:** each entry "runs an approved process in the project sandbox" ⇒ the
  `program` must be a pinned `process:<program>` grant (README: "Only the pinned program of a
  `process:` grant; runs in the project root of a trusted project"). Executing a config is
  `host::start_run(config, args, debug)` — `debug = true` "requires a debugger".
* **`id` rules:** UNCONFIRMED (WIT says only "the complete set"; `start_run` returns its own
  operation id which receives `run-exited`). Treat ids as plugin-stable strings, unique per
  `(project, program)`.
* **Capability needed:** UNCONFIRMED whether *listing* needs `process:` (reading files to build the
  list needs `project.read`; running needs `process:<program>`). Declare both.
* **`files` globs**: UNCONFIRMED whether globs (e.g. `**/pom.xml`) or exact root-relative names;
  test-runner uses a bare name (`example.tests`), solidity's `runDetectors` uses bare names too
  (`foundry.toml`, `hardhat.config.ts`).
* Mocking: `mock.set_run_configurations(plugin.configurations("/project"));` then
  `mock.configuration_runs()` → `Vec<(op id: String, config id: String, args: Vec<String>, debug: bool)>`.

Related but different — `[[contributes.runDetectors]]` (no code, API 1.4), `plugins/official/solidity/plugin.toml`:

```toml
[[contributes.runDetectors]]
id = "forge-test"
name = "forge test"
files = ["foundry.toml"]
command = "forge"
args = ["test"]
test = true
```

⇒ **For a no-code MVP the manifest alone can inject a run configuration** (`command` + `args`), and
`runConfigurations` + the Rust export is only needed for *dynamic* per-project argv.

## 12. `MockHost` — compilable test recipe (answer to Lead Q3)

Module path: `openidea_plugin::mock` (`src/lib.rs` has `#[cfg(not(target_arch = "wasm32"))] pub mod mock;`),
type `openidea_plugin::mock::MockHost`. Everything below is copied from the SDK's own doc-tests and
the two official plugin test modules, so it compiles as written.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;

    #[test]
    fn configurations_read_the_project() {
        let mock = MockHost::install()                     // -> Self; grants ALL but process:/net:/fs.read:
            .with_capabilities(&["project.read", "process:mvn"]); // consumes+returns Self; replaces the list
        mock.add_file("pom.xml", "<project/>");            // project-relative or absolute
        mock.add_dir("src");
        mock.add_external_file("/etc/x.conf", "...");      // fs::* outside projects
        mock.set_project_root(Some("/project"));           // default already Some("/project"); None = no project
        mock.set_setting("jvm.vmOptions", "\"-Xmx1g\"");    // RAW JSON TEXT, no schema default in the mock
        mock.on_process("mvn", |args, stdin| { /* -> Result<ProcessOutput, Error> */ Ok(Default::default()) });

        let mut plugin = MyPlugin::default();
        let cfgs = plugin.configurations("/project");      // call the trait method directly, no host needed
        assert_eq!(cfgs[0].program, "mvn");

        mock.set_run_configurations(cfgs.clone());         // what start_run() will resolve
        plugin.run_command("run", &mock.invocation(), None).unwrap();
        assert_eq!(mock.configuration_runs()[0].1, "spring-boot");  // (op id, config id, args, debug)
        assert!(mock.process_runs().is_empty());
    }

    #[test]
    fn command_touches_host_state() {
        let mock = MockHost::install();
        mock.open_editor("/project/a.txt", "text", "one two", &[(0, 0, 0, 3)]); // (line,col,line,col)+
        let mut plugin = MyPlugin::default();
        plugin.run_command("shout", &mock.invocation(), None).unwrap();
        assert_eq!(mock.editor_text(), "ONE TWO");
        assert_eq!(mock.notifications()[0].title, "t");
    }
}
```

Signature notes (all take `&self` unless stated):

| call | signature |
|---|---|
| install | `pub fn install() -> Self` (drops → uninstall; thread-local) |
| capabilities | `pub fn with_capabilities(self, caps: &[&str]) -> Self` |
| project files | `add_file(&self, path: &str, text: &str)`, `add_dir(&self, path: &str)`, `add_external_file(&self, path: &str, text: &str)` |
| projects | `set_project_root(&self, root: Option<&str>)`, `add_project(&self, root: &str)` |
| invocation | `pub fn invocation(&self) -> Invocation<'static>` |
| settings | `pub fn set_setting(&self, key: &str, json: &str)` |
| editor | `open_editor(&self, path: &str, language: &str, text: &str, selections: &[(u32, u32, u32, u32)])`, `editor_text()`, `editor_version()`, `type_in_editor(&self, text: &str)` |
| runs | `set_run_configurations(&self, configs: Vec<crate::PluginRunConfiguration>)`, `configuration_runs()` |
| processes | `on_process(&self, program: &str, handler: impl Fn(&[String], Option<&str>) -> Result<ProcessOutput, Error> + 'static)`, `process_runs()` |
| assertions | `notifications()`, `logs()`, `storage()`, `status_items()`, `subscriptions()`, `context_keys()`, `clipboard()`, `opened_urls()`, `run_targets()`, `output()`, `request.selection/paths` via `select_paths(&self, paths: &[&str])` |
| misc | `set_locale(&self, tag: &str)` (default `"en"`), `set_symbols(&self, root: &str, symbols: Vec<SymbolInfo>)`, `on_search`, `on_command`, `on_fetch`, `declare_secret`/`set_secret`, `finish_terminal(&self, key: &str) -> Option<Event>`, `context(&self, plugin_id: &str) -> Context` |

Traps: mock `setting()` is a bare map lookup (**no schema default, no project/global precedence**) —
`set_setting` every key your `configurations()` reads, or it sees `None`; `process:<program>` is
matched per program (`Denied("process:mvn")` without it, `NotFound("mvn (MockHost::on_process)")` with
the grant but no handler); `run_command` prepends `KEY=VALUE\n` lines to stdout; at most 8 children;
no installed mock ⇒ panic `"no mock host: call openidea_plugin::mock::MockHost::install() first"`.

## 13. Repo layout / packaging facts

* **`plugins/hello/` does not exist at HEAD.** `plugins/` contains only `.gitignore`, `README.md`
  (182,349 B), `build.sh` (6,000 B), `official/`. `official/` has 18 plugins: ai-completion,
  blame-annotations, checkstyle, container-runner, csv-editor, excalidraw, git-toolbox, graphviz,
  harbor-night, key-promoter, license-header, material-icons, mybatis, regex-tester, solidity,
  string-tools, test-runner, todo-finder.
* **Canonical plugin skeleton = official plugin layout** (verbatim files in §7.1/§7.2 of the
  container-runner below). Per-plugin: `Cargo.toml`, `Cargo.lock`, `plugin.toml`, `src/lib.rs`,
  `dist/plugin.wasm` (committed), optional `settings.schema.json`, `languages/*.toml`, `locales/*`.
* `plugins/official/container-runner/Cargo.toml` — verbatim, use as the template:

```toml
[package]
name = "openidea-plugin-openidea-container-runner"
version = "0.1.0"
edition = "2024"
publish = false

# A plugin builds on its own (target wasm32-wasip2), outside any IDE workspace.
[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
openidea-plugin = { path = "../../../sdk/rust/openidea-plugin" }

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
panic = "abort"
```

* **Toolchain:** `openidea-plugin new <name>` generates the package; `openidea-plugin build` runs
  `cargo build --release --target wasm32-wasip2`, copies the component to the manifest's `wasm` path
  and runs `check`; `check` validates manifest + every contribution + SVG + component imports against
  the IDE's WIT; `link` writes `links.json` into `$OPENIDEA_PLUGINS_DIR` (default
  `<app-data>/ai.l0ng.ai/openidea/plugins`); also `unlink`, `links`. The CLI ships as
  `crates/ide-host` bin `openidea-plugin` (`cargo install --path crates/ide-host --bin openidea-plugin`).
* **Generated `Cargo.toml` dependency form** (README §1): `openidea-plugin = { git = "…", tag = "vX.Y.Z" }`
  against this repo's release tag; with `--sdk <path to sdk/rust/openidea-plugin>` it is a path dep
  instead. Given no crates.io release (§1), our plugin must use **path** (vendored) or **git+tag**.
* Test-plugin fixtures upstream: `crates/ide-plugin/tests/fixtures/{hello,jobs,language-toy,misbehave,semantic-tokens,hover,commit,inlay-hints,file-templates,symbols}` — better examples than the official ones for edge cases.
* Host implementation sizes (for further reading): `crates/ide-plugin/src/runtime.rs` 112,132 B,
  `wit_host.rs` 90,533 B, `types.rs` 27,454 B, `sandbox.rs` 23,503 B, `actor.rs` 15,866 B,
  `execution.rs` 4,456 B, `language_features.rs` 3,141 B, `indexing.rs` 1,609 B, `lib.rs` 1,187 B.
* `plugins/README.md` is 182 KB (§1 Quick start at line 43, §2 The package line 83, Contribution
  points table line 138, File templates line 218, Background associations 250, Activation 265,
  Capabilities 286, §3 Writing code 366, API 1.4 368, The API 489, Database sources 587). The first
  72,261 bytes / 630 lines are decoded at `.research/raw/plugins-README.part1.md`.

## 14. Manifest grammar (confirmed, replaces §8)

Top level (`plugins/official/regex-tester/plugin.toml`, verbatim):

```toml
id = "openidea.regex-tester"
name = "Regex Tester"
version = "0.1.0"
description = "…"
authors = ["OpenIDEA contributors"]
license = "Apache-2.0"
change-notes = "…"
categories = ["tools"]        # ≤3: languages, themes, icon-themes, linters, vcs, ai, frameworks, tools, other
engines.openidea = "^1.0"     # API gate; `^1.4` for run configs/run targets/what follows
wasm = "dist/plugin.wasm"     # omit for data-only plugins
activation = ["on-view:regex", "on-command:*"]

[capabilities]
required = ["editor.read"]
```

* `id` grammar `[a-z0-9][a-z0-9_-]*`, optionally dotted namespaces (README). Command ids
  `[a-z0-9][a-z0-9._-]*`.
* `[[contributes.commands]]`: `id`, `title`, `category?`, `when?`.
* `[contributes.settings]` — **a table, not an array** (regex-tester, csv-editor, settings-pages
  fixture): `title = "…"` + either `schema = "settings.schema.json"` (single page) or, from API 1.3,
  `[[contributes.settings.pages]] id/title/schema` entries and optional
  `[[contributes.settings.pages.actions]] title/command`. Full grammar and the schema JSON dialect in
  §15. (The task brief's `[[contributes.settings]]` array form is wrong — official plugins use the
  table form.)
* `[[contributes.runConfigurations]]`: `files = [...]` (marker files; enables the Rust export).
* `[[contributes.runDetectors]]`: `id`, `name`, `files`, `command`, `args?`, `test?`, `framework?`.
* `[[contributes.menus]]`: `menu` (`editor.context`, `projectView.context`, `editorTab.context`,
  `viewItem.context`, `tools`, `commit.message`; API 1.4: `editor.title`, `mainToolbar`,
  `problems.context`, `searchResults.context`), `view?`, `command`, `when?`, `submenu?` (API 1.3).
* `[[contributes.keymaps]]`: `command`, `key` (`Ctrl+Alt+C`, chords `Meta+K Meta+C`), `mac?`.
* `[[contributes.statusBar]]`: `id`, `text`, `tooltip?`, `command?`, `align?`, `priority?`.
* `[[contributes.views]]`: `id`, `title`, `icon?`, `location` (`left`/`right`/`bottom`/`services`),
  `web?`, `group?`, `when?` (API 1.3).
* `[[contributes.mcpTools]]`: `name`, `description`, `inputSchema` (package JSON-Schema path),
  `readOnly`, `destructive` (API 1.4).
* `[[contributes.secrets]]`: `name`, `label`, `description?`.
* `[[contributes.providers]]`: `kinds` (e.g. `gutter-actions`), `languages`.
* Others in the README table: `themes` (`path`, `editor-scheme?`), `iconThemes` (`id`, `label`,
  `path`), `languages` (`path` = `languages/<id>.toml`), `snippets` (`language`, `path`),
  `customEditors` (`id`, `title`, `patterns`, `priority`, `layout?`, `web?`), `fileIcons`,
  `fileTemplates` (`id`, `name`, `fileName?`, `extension`, `path`, `variables = [{name,prompt}]`),
  `commitChecks` (`id`, `title`), `documentIndexers` (`extensions`).
* **`when` clauses** can test `viewItem`, `viewItemKind`, `workspaceContains:<exact path>` (no
  globs), and plugin keys `plugin.<id>.<key>`; unknown keys read as `false` (plugin still installs).
* **Translations:** manifest strings + view strings are translated from `locales/<tag>.json`
  (BCP 47, matching `host::locale()`); strings the plugin builds itself must be translated by hand.
* Version gates: `icon` ⇒ `^1.2`; `viewItem.context` menu ⇒ `^1.2`; `commit.message` menu,
  `submenu`, `fileTemplates.fileName`/`variables`, `settings.pages` ⇒ `^1.3`; run configs, run
  targets, terminal sessions, content tabs, ipc/auth, mcpTools, `[[contributes.runDetectors]]` ⇒ `^1.4`.
* Manifest is validated as a whole: one invalid contribution ⇒ nothing installs. Unknown
  contribution types from a newer API are skipped with a warning. `0.1`-format manifests
  (`openidea-api = "0.1"`, `permissions`) still load, but their code no longer runs.
* All paths are package-relative and may not leave the package.

## 15. Settings schema dialect — CONFIRMED (JSON-Schema-shaped subset + one UI extension)

It is **not** a bespoke widget vocabulary and **not** full JSON Schema: it is a plain `{ "properties":
{…} }` object whose leaves use standard JSON-Schema keywords, plus one IDE extension key `group`.
No `$schema`, `$id`, `$ref`, `required`, `oneOf`, `allOf` appears in any real file seen.

### 15.1 Verbatim examples

`plugins/official/regex-tester/settings.schema.json` (272 B, whole file):

```json
{
  "properties": {
    "ignoreCase": { "type": "boolean", "title": "Ignore case by default", "default": false },
    "multiLine": { "type": "boolean", "title": "^ and $ match at line breaks by default", "default": true }
  }
}
```

`plugins/official/csv-editor/settings.schema.json` (476 B, whole file — shows `description`,
`enum` and a string default):

```json
{
  "properties": {
    "firstRowIsHeader": {
      "type": "boolean",
      "title": "First row is the header",
      "description": "Show the first record as column names instead of a row.",
      "default": true
    },
    "delimiter": {
      "type": "string",
      "title": "Delimiter",
      "description": "auto: tab for .tsv files, else the most frequent of , ; | in the first line.",
      "enum": ["auto", ",", ";", "|", "tab"],
      "default": "auto"
    }
  }
}
```

`crates/ide-plugin/tests/fixtures/settings-pages/templates.json` (367 B, whole file — `array` of
scalars with a non-empty default, and the `group` extension):

```json
{
  "properties": {
    "templates": {
      "type": "array", "title": "Commit templates", "group": "Templates",
      "items": { "type": "string" }, "default": ["{type}: {summary}"]
    },
    "types": {
      "type": "array", "title": "Allowed commit types", "group": "Templates",
      "items": { "type": "string" }, "default": ["feat", "fix", "docs"]
    }
  }
}
```

`crates/ide-plugin/tests/fixtures/settings-pages/llm.json` (835 B) — nested `array` of `object`,
every scalar type, `enum`, `group`:

```json
{
  "properties": {
    "profiles": {
      "type": "array", "title": "Model profiles", "group": "Provider",
      "items": {
        "type": "object",
        "properties": {
          "name":        { "type": "string",  "title": "Name", "default": "Local" },
          "provider":    { "type": "string",  "title": "Provider", "enum": ["local", "openai"], "default": "local" },
          "baseUrl":     { "type": "string",  "title": "Base URL", "default": "http://localhost:11434" },
          "model":       { "type": "string",  "title": "Model", "default": "example-model" },
          "enabled":     { "type": "boolean", "title": "Enabled", "default": true },
          "temperature": { "type": "number",  "title": "Temperature", "default": 0.7 },
          "limit":       { "type": "integer", "title": "Token limit", "default": 2048 }
        }
      }
    }
  }
}
```

⇒ "an editable table of objects" and "a list of strings" are both first-class, IDE-rendered. **This
is exactly what a list of VM options needs** (`{"type": "array", "items": {"type": "string"}}`).

### 15.2 Dialect rules (as observed)

| keyword | where | meaning |
|---|---|---|
| `properties` | root and every `object` | ordered list of settings (rendered in file order) |
| `type` | each property | `boolean`, `string`, `number`, `integer`, `array`, `object` (all six observed) |
| `title` | each property | row label (also the page label if used at page level) |
| `description` | each property | secondary help text under the row |
| `default` | each property | initial value; type-matched (`false`, `"auto"`, `2048`, `0.7`, `["feat","fix"]`); read through `host::setting` when the user has not set anything |
| `enum` | string property | renders a dropdown; values are the raw JSON values |
| `items` | array property | `{"type":"string"}` or a full `object` with `properties` (table editor) |
| `group` | any property | **UI extension**: a section heading; consecutive properties with the same `group` render together (NOT standard JSON Schema) |

UNCONFIRMED: `required`, `minimum`/`maximum`, `format`, `pattern`, `oneOf`, `$ref`, `hidden`/`dependsOn`
conditional visibility, and any widget override (e.g. textarea vs one-line input). None appear in
the four real files we read; assume unsupported and keep schemas to the table above.

### 15.3 How values arrive in the plugin

* `host::setting(key)` → raw JSON text: string ⇒ `"\"auto\""`, bool ⇒ `true`, number ⇒ `2048`,
  array/object ⇒ the whole JSON array/object text (the settings-pages fixture README: "it reads the
  saved JSON array"). Use `serde_json::from_str` for lists.
* Helpers only cover scalars: `setting_bool`, `setting_string`, `setting_number` (§5.1).
* Defaults are applied by the host, so `setting(key)` is `Some(default)` even for untouched keys
  (mock excepted — see §6/§12: the mock has no defaults).

### 15.4 `[contributes.settings]` forms (two, both real)

Single page (regex-tester / csv-editor):

```toml
[contributes.settings]
title = "Regex Tester"
schema = "settings.schema.json"
```

Multi-page (API 1.3) — `crates/ide-plugin/tests/fixtures/settings-pages/plugin.toml`, verbatim:

```toml
id = "settings-pages"
name = "Model Profiles"
version = "0.1.0"
engines.openidea = "^1.3"
wasm = "dist/plugin.wasm"
activation = ["on-command:*"]

[[contributes.commands]]
id = "test"
title = "Test model profile"

[contributes.settings]
title = "Model Profiles"

[[contributes.settings.pages]]
id = "templates"
title = "Commit Template"
schema = "templates.json"

[[contributes.settings.pages]]
id = "llm"
title = "LLM Settings"
schema = "llm.json"

[[contributes.settings.pages.actions]]
title = "Test"
command = "test"
```

UI behaviour (fixture README, paraphrased): pages appear under **Settings ‣ Plugins ‣ <settings.title>**;
edits are pending until Apply/OK; Cancel discards; running a page action first **saves** pending
edits, and Cancel afterwards does not undo them. Inline errors are shown in the page. Keep API keys
in `[[contributes.secrets]]`, never in a settings table cell.

## 16. Putting it together — minimal shape for a run-configurations + settings plugin

Derived strictly from §13/§14/§15; nothing here was compiled.

`Cargo.toml` (mirror `plugins/official/container-runner/Cargo.toml`, §13):

```toml
[package]
name = "openidea-plugin-jvm-params"     # convention: openidea-plugin-<dir-name>
version = "0.1.0"
edition = "2024"
publish = false

[workspace]                              # plugins build standalone (target wasm32-wasip2)

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
# SDK is NOT on crates.io (§1): path (vendored) or git+tag.
openidea-plugin = { path = "../../../sdk/rust/openidea-plugin" }

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
panic = "abort"
```

`plugin.toml`:

```toml
id = "openidea.jvm-params"
name = "JVM Parameters"
version = "0.1.0"
description = "…"
authors = ["…"]
license = "Apache-2.0"
engines.openidea = "^1.4"               # run configs need 1.4
wasm = "dist/plugin.wasm"
activation = ["on-command:*"]

[capabilities]
required = ["project.read", "process:mvn", "process:java"]   # + storage if state is kept

[[contributes.commands]]
id = "apply"
title = "JVM Parameters: Apply to run configuration"

[contributes.settings]
title = "JVM Parameters"
schema = "settings.schema.json"

[[contributes.runConfigurations]]
files = ["pom.xml", "build.gradle", "build.gradle.kts"]      # marker files (refresh trigger)
```

`settings.schema.json`:

```json
{
  "properties": {
    "vmOptions": {
      "type": "array", "title": "VM options", "group": "JVM",
      "items": { "type": "string" },
      "default": ["-Xmx1g", "-Dfile.encoding=UTF-8"]
    },
    "wrapper": {
      "type": "string", "title": "Build wrapper", "group": "JVM",
      "enum": ["auto", "mvnw", "mvn", "gradlew", "gradle"], "default": "auto"
    }
  }
}
```

Reading it in code: `host::setting("vmOptions")` → `Some("[\"-Xmx1g\",…]")`;
`serde_json::from_str::<Vec<String>>(&v)?` (`Error` has `From<String>`/`From<&str>` → `Failed`, so
map the serde error with `.map_err(|e| e.to_string())?`).

`src/lib.rs` outline:

```rust
use openidea_plugin::{
    register_plugin, Error, Invocation, Plugin, PluginRunConfiguration, RunConfigurations, host,
};

#[derive(Default)]
struct JvmParams;

impl Plugin for JvmParams {
    fn run_command(&mut self, id: &str, ctx: &Invocation, _arg: Option<&str>) -> Result<Option<String>, Error> {
        let project = ctx.project().ok_or("no project")?;
        let w = host::setting_string("wrapper").unwrap_or_else(|| "auto".into());
        // "auto" ⇒ probe files instead of an OS check (§10)
        let program = match w.as_str() {
            "mvnw" => "mvnw", "gradlew" => "gradlew",
            _ if project.exists("mvnw.cmd")? || project.exists("mvnw")? => "mvnw",
            _ if project.exists("gradlew.bat")? || project.exists("gradlew")? => "gradlew",
            _ => "mvn",
        };
        let out = host::run_command(
            &host::Command::new(program).arg("-v"), Some(&project), None)?;
        Ok(Some(out.stdout))
    }
}

impl RunConfigurations for JvmParams {
    fn configurations(&mut self, _root: &str) -> Vec<PluginRunConfiguration> {
        vec![PluginRunConfiguration {
            id: "jvm".into(), name: "JVM (plugin)".into(),
            program: "mvn".into(), args: vec!["spring-boot:run".into()],
            test: false, cwd: None, framework: None,
        }]
    }
}

register_plugin!(JvmParams, run_configurations);
```

`dist/plugin.wasm` is produced by `openidea-plugin build` (or
`cargo build --release --target wasm32-wasip2` + copy) and is committed next to `plugin.toml`.

## 17. Remaining uncertainty (for whoever continues)

1. `plugins/README.md` is 182 KB and only its first 72 KB are decoded
   (`.research/raw/plugins-README.part1.md`, 630 lines); the Settings and Translations prose sections
   sit beyond the fetch cap. Grammar above is from real manifests instead, so this only affects prose
   nuance (e.g. whether `group` has siblings).
2. Not confirmed: widget overrides, conditional visibility, `required`/`minimum`/`pattern` support in
   the settings dialect (§15.2), and whether `runConfigurations.files` accepts globs.
3. `id` uniqueness rules for `PluginRunConfiguration` ids are not documented upstream; treat as
   stable, unique per `(project, program)`.
4. `crates/ide-plugin/src/` at HEAD contains only actor, execution, indexing, language_features,
   lib.rs, runtime (112 KB), sandbox, types, wit_host — the manifest/`plugin.toml` parser is in
   another crate (the old `manifest.rs` path now 404s). If the exact validation rules are needed,
   read `runtime.rs` (its first ~200 lines list modules) and `sandbox.rs`.
5. `host::runtime` code paths for `start_run` (what happens on `process:` denial, whether `debug`
   requires a DAP provider) were not read; `crates/ide-plugin/src/execution.rs` (4,456 B) is the
   entry point.
