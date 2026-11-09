# Run-configuration provider API — semantics & host behaviour

Scope: what a Rust→WASM plugin can do with `contributes.runConfigurations` / `contributes.runDetectors` /
`contributes.runTargets`, and every host-side behaviour around `host::start_run` + `run-exited`.
Repo: `github.com/l0ng-ai/openidea` @ `main`, API 1.4.0. Installed IDE: `D:\DevSoft\OpenIDEA\openidea.exe`.

Method note (what could not be read): `plugins/README.md` (§6 "Run configurations" onward) was **not
obtainable**. jsDelivr's *data API* file list for `@main` is stale (README listed as 9160 B; no `wit/`,
`sdk/`, `plugins/official/` at all). `raw.githubusercontent.com` truncates the ~100 KB+ file just before
"#### Additional language features (API 1.4)"; `cdn.jsdelivr.net` 301-redirects that path to
raw.githubusercontent (web_fetch refuses cross-origin redirects); GitHub HTML blob view returns the whole
file (same truncation); `gitcdn.link` / `raw.githack.com` / `api.allorigins.win` / `r.jina.ai` /
`api.codetabs.com` all failed. A sibling agent's saved copy (`plugins-README.part1.md`, 72 776 B, 630
lines) stops at §3 "The API", i.e. also before §6. **Everything below is therefore taken from the
implementation**, which is authoritative; doc quotes are marked with their file.

---

## 0. TL;DR verdicts

| Question | Answer |
|---|---|
| Can a plugin config be a normal runnable for a Java app? | **Yes, but only as a raw argv launcher.** It is listed next to the IDE's own detected configs, kind `plugin`, id `plugin:<plugin-id>:<your-id>`. The IDE applies **no** JDK/classpath/Spring logic to it — you must put `-cp`/`-jar`/`@argfile` etc. in `args` yourself. |
| Debug? | **No.** Plugin configs are created with `debug_adapter: None`; `start_run(..., debug=true)` fails with `this configuration has no debugger`. |
| Program field | One bare program name (e.g. `java`, `mvn`) **or an absolute path**; single token, no whitespace, ≤256 B. `./mvnw`/`mvnw.cmd` are *invalid as capability values*; project-local executables go through the **project pin** mechanism (see §5). |
| Args | Verbatim argv, **no shell**, no globbing, no quoting rules. |
| Env vars | **Impossible** — the WIT record has no env field and the host builds `RunConfig.env` empty. |
| Refresh | `configurations()` is re-called on **every Run-dialog listing** (`ops::run::configs`) for each trusted project whose marker files exist, under a **2 s wall-clock budget**. Result is cached host-side per `(plugin, project-root)`. |
| Idle reclaim | Configs **survive** reclamation (host-side `BTreeMap`, and `run-exited` is an *owned* event). |
| Capability | `process:<program>` — one grant **per program token**, `Risk::High`, exact match only. |

---

## 1. The data contract (WIT + SDK)

`wit/run.wit` (package `openidea:plugin@1.4.0`):

```wit
/// Optional dynamic configurations, refreshed alongside run detectors.
interface run-configurations {
    /// A configuration running an approved process in the project sandbox.
    record run-configuration {
        id: string,
        name: string,
        program: string,
        args: list<string>,
        cwd: option<string>,
        test: bool,
        framework: option<string>,
    }
    /// Returns the complete set for this project root.
    configurations: func(project-root: string) -> list<run-configuration>;
}

interface run {
    use types.{error};
    /// Starts an owned configuration, appending args. Debug requires a debugger.
    /// Only from a window command/event. The returned id receives run-exited.
    start: func(config: string, args: list<string>, debug: bool) -> result<string, error>;
}
```

`wit/plugin.wit`:

```wit
world plugin-run-configurations { include plugin; export run-configurations; }
/// Export-only configurations world, composed with plugin by SDK macros.
world configurations-provider { export run-configurations; }
world plugin-test-results { include plugin; export test-results; }
world test-results-provider { export test-results; }
```

Rust SDK (`sdk/rust/openidea-plugin/src/execution.rs`), all `#[serde(rename_all = "camelCase")]`:

```rust
pub struct PluginRunConfiguration {
    pub id: String, pub name: String, pub program: String, pub args: Vec<String>,
    pub cwd: Option<String>, pub test: bool, pub framework: Option<String>,
}
pub struct RunExited { pub id: String, pub config: String, pub status: Option<i32>, pub killed: bool }

/// Supplies complete project configurations; register with run_configurations.
pub trait RunConfigurations { fn configurations(&mut self, project_root: &str) -> Vec<PluginRunConfiguration>; }

/// Completion of a run started by this plugin, even after idle reclamation.
pub fn run_exited(&self) -> Option<RunExited>;
```

Registration: `register_plugin!(MyPlugin, run_configurations)` (also `test_results`, `language_features`, …).
Manifest stake: `[[contributes.runConfigurations]] files = ["<marker>"]` — **requires `wasm = "..."` and
`engines.openidea = "^1.4"`**.

---

## 2. Where the host turns a plugin record into a RunConfig (the decisive file)

`crates/ide-host/src/plugins/run_detectors.rs` — module doc:

> "Run configurations detected outside this crate: plugins' `runDetectors`… The command is a `process:`
> capability like a language server's: the configuration is offered only while it is granted, and starts
> the approved file (checked right before it starts)."

Static detectors use this manifest shape (verbatim from that file's doc comment):

```toml
[[contributes.runDetectors]]
id = "forge-test"
name = "forge test"
files = ["foundry.toml"]       # any of these at the project root
command = "forge"
args = ["test"]
test = true                    # results go to the test tree
```

Validation of a static detector (`RunDetectorsPoint::prepare`): `validate_item_id(id, "run detector id")`;
`name` 1–128 bytes; `files` 1–16, each `check_relative`; **`command` must not contain whitespace**
("`command` is one program name (arguments go in `args`)"); `args` ≤ 64; `framework` requires
`needs_api("runDetectors.framework", 4)` and is 1–64 chars; the point registers
`Prepared::…capability(Capability::Process(command))` — i.e. **the manifest declaration itself demands the
`process:<command>` grant**.

Dynamic provider output is validated by `validate_configurations` in
`crates/ide-host/src/plugins/execution.rs`:

```rust
let mut ids = std::collections::BTreeSet::new();
if items.len() > 1000 || serde_json::to_vec(items).is_ok_and(|b| b.len() > 4 << 20) {
    return Err(HostError::Invalid("too many run configurations".into()));
}
for c in items {
    ide_extension::validate_item_id(&c.id, "run configuration id").map_err(HostError::Invalid)?;
    label(&c.name, 128)?;                                  // 1..128 UTF-16, no control chars
    if !ids.insert(&c.id)
        || c.program.is_empty()
        || c.program.len() > 256
        || c.program.contains(char::is_whitespace)
        || c.args.len() > 256
        || c.args.iter().any(|a| a.len() > 4096 || a.contains('\0'))
    { return Err(HostError::Invalid("invalid run configuration".into())); }
    if let Some(cwd) = &c.cwd { ide_plugin::resolve_new_in_root(root, cwd).map_err(HostError::Invalid)?; }
}
```

On invalid output the host logs `tracing::warn!(%plugin, %error, "invalid run configurations")` and
**keeps the previous set** for that `(plugin, root)` (the entry is not re-inserted, and the `retain` pass
only drops plugins that no longer qualify — see §3).

Conversion to `ide_run::RunConfig` (same file, `ExternalDetection::detect`): `kind = "plugin"`,
`command = c.program`, `args = c.args`, `cwd = root.join(c.cwd).unwrap_or(root)`, `env = {}` (always
empty), `detected = true`, `is_test = c.test`, `debug_adapter = None`, `allow_parallel = false`,
`extras.params["framework"] = c.framework`, and finally

```rust
config.id = config_id(plugin, &c.id);            // format!("{EXTERNAL_PREFIX}{plugin}:{detector}")
```

with `pub const EXTERNAL_PREFIX: &str = "plugin:";` in `crates/ide-run/src/external.rs` — doc:
"Id prefix of externally detected configurations (`plugin:<plugin>:<id>`)".
⇒ **`id` need only be unique within your own returned vec; the host namespaces it by plugin id.**
Reuse the same `id` across refreshes; changing it orphans the user's saved override/suppression.

Semantics per field, settled:

| field | meaning |
|---|---|
| `id` | local id; host prefixes `plugin:<plugin-id>:`. Must be non-duplicated in one response (`validate_item_id`, same validator as command ids → lowercase-ish slug, no `:`), stable across refreshes. |
| `name` | 1–128 UTF-16 units, no control chars; shown verbatim in the Run dialog. |
| `program` | **one token**, no whitespace, ≤256 B: bare name to be resolved/pinned, or absolute path. Needs a granted `process:<program>` (exact match). |
| `args` | ≤256 items, each ≤4096 B, no NUL. Passed as argv **without a shell** (see §4). |
| `cwd` | project-root-**relative**; `resolve_new_in_root` rejects escapes. Omitted/`null` ⇒ project root. Absolute cwd is not accepted (resolves relative to root). |
| `test` | becomes `RunConfig.is_test` — `/// Test runner configs render results in the test tree.` |
| `framework` | stored as `extras.params["framework"]`. **Only the literal `"plugin"` makes the IDE hand you `test-results.parse` / `select-tests`.** |

---

## 3. Lifecycle: when is `configurations()` called, what is cached, what wins

`crates/ide-host/src/plugins/execution.rs`:

```rust
/// Dynamic configurations opt in with marker files, just like static detectors.
pub(super) fn configurations_declaration(pkg, value) -> Result<UiDecl, String> {
    pkg.manifest.needs_api("contributes.runConfigurations", 4)?;
    if pkg.manifest.wasm.is_none() || decl.files.is_empty() || decl.files.len() > 16 {
        return Err("runConfigurations needs code and 1-16 marker files".into());
    }
    …
}

/// Refreshes configuration exports on the same listing path as static detectors.
pub fn refresh_run_configurations(&self, root: &Path) {
    if !self.shared.workspace().is_some_and(|w| w.trusted(root)) { return; }
    let owners: BTreeSet<_> = self.ui.items("runConfigurations", None).into_iter()
        .filter(|d| d.decl["files"].as_array().is_some_and(|f| f.iter().filter_map(|f| f.as_str())
                                                          .any(|f| root.join(f).is_file())))
        .map(|d| d.plugin_id).collect();
    self.shared.run_configurations.write().retain(|(p, r), _| r != root || owners.contains(p));
    for plugin in owners {
        let invocation = ide_plugin::Invocation { workspace_root: Some(root.into()), ..Default::default() };
        let result = self.code_call(&plugin, invocation, "run-configurations", |rt, ctx| {
            rt.run_configurations(ctx.workspace_root.unwrap().to_string_lossy().into_owned(),
                                  Duration::from_secs(2))
        });
        let Ok(items) = result else { continue };
        if let Err(error) = validate_configurations(root, &items) { tracing::warn!(…); continue; }
        self.shared.run_configurations.write().insert((plugin, root.into()), items);
    }
}
```

Deductions (all forced by the code above):

1. **Marker-gated, per trusted project.** No marker file at the project root ⇒ `configurations()` is never
   called. Untrusted project ⇒ never called (`refresh_run_configurations` returns early; and
   `ops::run::configs` calls `host.require_trust(window)` first anyway).
2. **Refresh trigger = listing the Run configurations.** `crates/ide-host/src/ops/run.rs`:
   ```rust
   /// Detected configs followed by user-defined ones (user ids win on conflict).
   pub fn configs(host: &Host, window: &str) -> IdeResult<Vec<RunConfig>> {
       host.require_trust(window)?;
       let project = host.project(window)?;
       if let Some(plugins) = host.plugin_host() { plugins.refresh_run_configurations(&project.root); }
       project_configs(&project.root, &project.settings.effective(), &host.config_dir)
   }
   ```
   Every Run-dialog open/refresh (`run/configs`) re-invokes the export. There is **no** per-file-change
   re-invocation in this path; other callers may exist (`plugins/mod.rs`, `files_changed.rs` were not read
   in full — UNCONFIRMED, see §8).
3. **Cost budget = 2 s wall clock** for the whole `configurations()` call. A cold instance compiling against
   it is a real risk; keep the function to `host::setting*` + at most a couple of `Project::read_file`/
   `find_files`, or pre-compute into a cached `Vec` and return it (it is a `&mut self` trait method, so you
   may memoise). If it times out or errors, the previous set stays — the Run dialog just shows stale data.
4. **Imports are available.** The call is an ordinary `code_call` with
   `Invocation { workspace_root: Some(root), .. }` and the plugin runtime's full import set (settings, fs,
   project/workspace, storage…). So **yes**, `host::setting_string(..)`, `Project::read_file`,
   `Project::find_files` etc. may be used *inside* `configurations()`. There is no "no-imports" restriction
   — only the 2 s budget.
5. **Merge, not replace, and no shadowing of the IDE's own detect.** The final list is
   `ide_run::detect_with_profiles(root, …)` (built-in detectors + `external::detect`, i.e. plugin static
   detectors *and* your dynamic records) followed by user configs from `.openidea/run.toml`. Plugin
   configs are additional entries with kind `plugin`; they never replace a built-in
   `application`/`spring-boot`/`junit` config, and a user config with the *same id* wins
   (`detected_and_saved` filters detected ones out when `saved.iter().any(|u| u.id == d.id)` or when the id
   is in the suppressed list).
6. **User edit / pin / hide.** The user can (a) duplicate or hand-write a config with the identical id
   `plugin:<plugin-id>:<your-id>` in `.openidea/run.toml` and edit it freely — that shadows your generated
   entry until deleted; (b) remove/hide it, which lands in `ide_run::load_suppressed_config_ids` and keeps
   it hidden across refreshes; (c) via Settings › Run Targets for `runTargets`. Your record itself is
   read-only and regenerated on every refresh.
7. **Idle reclaim does not lose them.** They live in host state
   `DynamicConfigurations = Arc<RwLock<BTreeMap<(String, PathBuf), Vec<ide_plugin::PluginRunConfiguration>>>>`,
   read by `ExternalDetection::detect` without calling back into the plugin. Same answer for the WIT
   `run_configurations` semantics doc: "Targets survive idle reclamation, and disappear on unregister or
   plugin stop" (that line is about `runTargets`, but the storage shape for configurations is the same and
   `finish_operation`/`run_exited` are explicitly documented as surviving reclamation).
   What does disappear: everything, if the plugin is *disabled* — `launch()` then reports
   `"the plugin that detected this configuration is no longer enabled"`, and `refresh_…` drops the cached
   set for that root once the plugin no longer owns the marker.

---

## 4. `start_run` → `run-exited` round trip

### 4.1 Calling

```rust
/// Starts an owned run configuration in the invoking window. Its id receives run-exited.
pub fn start_run(config: &str, args: &[String], debug: bool) -> Result<String, Error>
```

* `config` = the **full configuration id string as the IDE knows it**, i.e. your record's id with the host
  prefix: `plugin:<plugin-id>:<your-id>`. The host splits it back apart:
  ```rust
  let Some((plugin, id)) = config.strip_prefix(ide_run::EXTERNAL_PREFIX).and_then(|s| s.split_once(':')) else { … };
  let owned = workspace.run_configuration(client, plugin, id)?;   // crates/ide-host/src/plugins/execution.rs
  ```
  (`plugin:` + `split_once(':')` ⇒ your record id must not itself contain `:`.)
* `args` are **appended after the configuration's own `args`** — WIT doc: "Starts an owned configuration,
  appending args." (test-runner does exactly this to append the test-name filter.)
* `debug = true` requires `debug_adapter`. Your configs always have `debug_adapter: None`, and
  `prepare_debug_run` returns `Err(IdeError::invalid("this configuration has no debugger"))`. Passage of
  `true` is therefore useless; pass `false`.
* **"Only from a window command/event"** (WIT). The call needs an `Invocation` with a window/client and a
  workspace root; the host derives the project from `client` (`self.shared.project_of(client)`). Calling it
  from inside `configurations()` (no window command) is not the supported path — expose a
  `[[contributes.commands]]` action, or react to an event, and call from there.
* The returned `String` is an **operation id** (`plugin-op-<n>`), not a run id. It is what arrives back in
  `RunExited.id`. At most **32 concurrent operations per plugin** ("at most 32 active operations").

### 4.2 Launch path inside the host

`crates/ide-host/src/ops/run.rs::start_inner` (the one place the argv is finalised):

```rust
if config.detected && config.id.starts_with(ide_run::EXTERNAL_PREFIX) && let Some(plugins) = host.plugin_host() {
    let provider = plugins.run_detection();
    let binary = provider.launch(&config).map_err(|e| IdeError::new(ErrorKind::PermissionDenied, e))?;
    config.command = binary.to_string_lossy().into_owned();   // resolved approved absolute path
    config.detected = false;
}
…
sink = plugins.run_operation_sink(window, &mut config, sink)?;   // rewrites command again, see below
…
host.processes.start(&project.root, &config, &options, sink, …)
```

`PluginRunDetection::launch` (`run_detectors.rs`) — the *only* place the program is approved:

```rust
let pin = self.project_pins.read().get(&(plugin, cap, root)).cloned()
    .or_else(|| self.pins.read().get(&(plugin, cap)).cloned())
    .ok_or_else(|| "run program is no longer granted".to_owned())?;   // dynamic records
…
let pin = self.pin(&plugin, &d, config.cwd…).ok_or_else(|| {
    format!("allow process:{} for plugin {plugin} in Settings › Plugins to run this configuration", d.command)
})?;
pin.verify().map(Path::to_path_buf)                                    // static detectors
```

⇒ program resolution is **pin-based, not PATH-based at launch**: the grant captured an absolute,
hash-verified executable at approval time, and `pin.verify()` re-checks it right before every start
(`/// Resolves and rechecks the pin immediately before starting …`). If the binary changed on disk the
start fails. `ProcessManager::start` reports `ErrorKind::PermissionDenied` with the reason string.

### 4.3 `run-exited`

`crates/ide-host/src/plugins/execution.rs`:

```rust
pub fn finish_operation(&self, client: &str, id: &str, status: Option<i32>, stopped: bool) -> bool {
    let event = match operation.kind {
        OperationKind::Run { config, .. } => WorkbenchEvent::RunExited(ide_plugin::RunExited {
            id: id.into(), config, status, killed: stopped }),
        OperationKind::Terminal { key, .. } => WorkbenchEvent::TerminalExited(…),
    };
    self.post_owned_event(&operation.plugin, client, event);
```

* `id` = the operation id returned by `start_run`; `config` = the configuration id you passed to
  `start_run`; `status` = `Option<i32>` process exit code (`ide_core::IdeEvent::RunExited { code, .. }`);
  `killed = true` when the operation was marked closed — i.e. the user pressed Stop
  (`ops::run::stop` → `plugins.run_operation_stopped(id)`) or the terminal was closed.
* Delivery is an **owned event**: `post_owned_event` "bypasses subscriptions, just like completions of
  owned operations", targeted at `(plugin, client)` and queued in an `EventQueue`. You therefore do not
  need any `[capabilities]` entry for it, but you must be **active** to receive it — declare
  `activation = ["on-event:run-exited", …]`. (`EventKind::run_exited` needs no capability:
  `Scheduled | RunExited | TerminalExited | ContentTabAction => None` in
  `crates/ide-plugin/src/types.rs::EventKind::capability`.) SDK doc: "Completion of a run started by this
  plugin, even after idle reclamation."
* Exit code semantics: `status: None` + `killed: true` = terminated by the IDE; `status: Some(0)` = clean.
* Duplicate/foreign reports are ignored: `if !operations.get(id).is_some_and(|op| op.client == client)
  { return false; }`.

### 4.4 Manifest requirements (checklist)

```toml
[plugin]
id = "com.example.jvmargs"
name = "JVM Args"
version = "0.1.0"
engines.openidea = "^1.4"
wasm = "dist/plugin.wasm"
activation = ["on-command:*", "on-event:run-exited"]   # commands you expose + the completion event

[capabilities]
required = ["process:java"]        # one entry per program token in `program`; Risk::High → own dialog

[[contributes.commands]]
id = "runWithArgs"
title = "Run with JVM Default Options"

[[contributes.runConfigurations]]
files = ["pom.xml"]               # 1–16 project-root markers; presence gates configurations()
```
* No `project.read`, no `editor.*`: `start_run` and the export need nothing beyond the `process:` grants.
  (`Project::read_file` inside `configurations()` is a `project.read` operation — add `project.read` if you
  read project files there.)
* `[[contributes.runDetectors]]` (static) needs no code/activation at all, but its `command` becomes the
  required capability automatically.

---

## 5. Program resolution / capability grant — the practical rules

`crates/ide-extension/src/capability.rs`:

```rust
"process" => {
    if arg.contains(['/', '\\']) && !std::path::Path::new(arg).is_absolute() {
        return Err(format!("{s:?}: use a program name or an absolute path"));
    }
    Ok(Self::Process(arg.to_owned()))
}
```
* `process:java` ✔, `process:C:\Program Files\Eclipse Adoptium\jdk-21\bin\java.exe` ✔,
  `process:/usr/bin/java` ✔, **`process:./mvnw` ✘** (rejected at manifest parse), `process:mvnw` ✔ (bare).
* `Process` is `Risk::High`; `describe()` → `"Run {bin} — equivalent to running arbitrary code on this
  machine"`. Separate grant dialog, not the install-page list. `covers()` is exact-match only — no path
  prefixes, no globs.
* Approval UI copy (container-runner README, authoritative for the flow): "Only approved executables run;
  selecting one in Settings › Plugins reapproves/pins it. No shell command strings are evaluated." Together
  with `ProcessManager`'s hash check this means: **the user picks the executable file once; a hash pin is
  stored; later changes to that file, or a different PATH resolution, refuse to run.**

### Project-local launchers (`mvnw`, `gradlew`, `mvnw.cmd`)

Pins exist in two scopes (`run_detectors.rs`):

```rust
type Pins = Arc<RwLock<HashMap<(String, String), ProgramPin>>>;          // (plugin, "process:<prog>")
self.project_pins.read().get(&(plugin.into(), cap.to_string(), root.into())).cloned()   // project first
    .or_else(|| self.pins.read().get(&(plugin.to_owned(), cap.to_string())).cloned())
```
plus `PluginApi::approved_program_in_project(&program, Some(root))` used by `register_target`, and
`PluginHost::program_for_project(plugin, program, root)` used by `prepare_run_target`,
`prepare_debug_run` and `run_operation_sink`.

⇒ A project-relative wrapper is expressible as **`program = "mvnw"` (bare token) + a project-scoped pin**
for that trusted project; the same plugin id in another project without its own pin falls back to the
global pin (or refuses). What is *not* expressible is `program = "./mvnw"` (illegal capability value) — such
a record is filtered out of the listing entirely, because `detect()` skips dynamic entries with no pin.
The exact UI that creates a project pin ("use the project's wrapper") lives in `api.rs` / the settings UI
and was **not read** (162 KB file, beyond the fetch cap) — see §8.

---

## 6. Official precedents (copyable)

`plugins/official/` contains 18 plugins: ai-completion, blame-annotations, checkstyle, container-runner,
csv-editor, excalidraw, git-toolbox, graphviz, harbor-night, key-promoter, license-header, material-icons,
mybatis, regex-tester, solidity, string-tools, test-runner, todo-finder. **There is no java / jvm / spring /
gradle / maven / run-anything official plugin** — i.e. no in-tree precedent for injecting JVM options, and
nothing to copy for classpath handling.

* **Static detector** — `plugins/official/solidity/plugin.toml`:
  `id = "forge-test"`, `name = "forge test"`, `files = ["foundry.toml"]`, `command = "forge"`,
  `args = ["test"]`, `test = true`. No code, no activation; the plugin still must be granted `process:forge`.
* **Dynamic provider + tests** — `plugins/official/test-runner`:
  ```rust
  impl RunConfigurations for TestRunner {
      fn configurations(&mut self, _root: &str) -> Vec<PluginRunConfiguration> {
          vec![PluginRunConfiguration { id: "tests".into(), name: "Example tests".into(),
               program: "openidea-example-tests".into(), test: true,
               framework: Some("plugin".into()), ..Default::default() }]
      }
  }
  register_plugin!(TestRunner, run_configurations, test_results, language_features);
  // command handler:
  let id = host::start_run("tests", &args, false)?;   // ← literal id, not "plugin:…"; see note
  ```
  manifest: `activation = ["on-command:*"]`, `[capabilities] required = ["process:openidea-example-tests"]`,
  `[[contributes.runConfigurations]] files = ["example.tests"]`, `engines.openidea = "^1.4"`.
  README: "Put this directory on PATH **before installing/granting** the plugin, so
  `process:openidea-example-tests` resolves to the included executable… Its presence enables the dynamic
  `run-configurations` export." Mock test asserts `mock.configuration_runs()[0].2 == vec!["adds"]` (index 2
  = the appended args) and that the selected test names become the complete argv for rerun.
  ⚠️ Note the literal `"tests"` in `start_run`: the host's own id is `plugin:openidea.test-runner:tests`.
  Either the runtime qualifies a bare id for an owned configuration, or that example is stale — **UNCONFIRMED**
  (§8). For our plugin, pass the fully-qualified `plugin:<id>:<local>` form; it is unambiguous.
* **`runTargets` wrapper (containers)** — `plugins/official/container-runner`:
  ```rust
  host::register_run_target(&project, &PluginRunTarget {
      id: "example".into(), name: "Example wrapper".into(),
      program: PROGRAM("openidea-example-wrapper").into(),
      args: vec!["exec".into()], workdir_args: vec!["--cwd".into(), "{cwd}".into()],
      container_root: None, commands: vec![] })?;
  ```
  capabilities `["project.read", "process:openidea-example-wrapper"]`; targets are read-only project
  registrations that "survive idle reclamation, and disappear on unregister or plugin stop"; they appear in
  Settings › Run Targets, not in the Run dialog. Useful only if we ever wrap Maven/Gradle via a container.

---

## 7. Does the IDE's built-in Java support consume plugin configs? (the decisive question)

**No, it does not *merge into* them — but it does *list* them, and they run as plain argvs.**

Evidence chain:

1. The Run dialog list is `ops::run::configs` → `project_configs` → `detected_and_saved` →
   `ide_run::detect_with_profiles(...)`. That function is the built-in detector set
   (`package.json`, `Cargo.toml`, `build.gradle(.kts)`, `pom.xml`, `go.mod`, `pyproject/pytest`,
   `Makefile`, plus the JVM modules `java_cmdline.rs`, `gradle_jvm.rs`, `junit.rs`, `tomcat.rs`) **and** the
   external provider (`external::detect`). So plugin configs are in the same list as
   `application`/`spring-boot`/`junit` entries, but as **separate entries** with `kind = "plugin"`.
2. `RunConfig.kind` doc: `/// npm | cargo | gradle | maven | go | python | pytest | make | shell …` — the
   kind selects per-type post-processing. `start_inner` applies it only to
   `maven`, `gradle`, `tomcat`, `go`, `kind == "application" | "spring-boot"` paths; for `plugin` it only
   runs the generic `ide_sdk::apply::resolve_run_command(&mut config, root, &settings)` (which resolves a
   bare command such as `godot` against SDK settings — it does **not** add a JDK, a classpath, or
   `JAVA_TOOL_OPTIONS`).
3. The JVM classpath mechanism in the IDE is `RunExtras.module` — `/// Owning module, independent of the
   working directory; also supplies a JVM application's classpath.` — and it is populated by the built-in
   `application`/`spring-boot` configs, **not** by plugin records (the WIT record has no `module` field and
   `PluginRunDetection` never sets it).
4. `ops::run::commands` (`mod commands` in `ops/run.rs`) drives the configuration *editor*, and the
   built-in JVM types get their own form (`RunExtras.params`: `/// Values of the configuration type's own
   form fields (the command line derives from them).`). Plugin configs have no such form.

**Consequences for a "JVM Default Options" plugin**

* A record with `program: "java"` **is** runnable from the Run dialog and from `start_run`; it executes the
  approved `java` binary with exactly your `args`. It is shown under the plugin's config list, kind
  `plugin`, and behaves like `Run | Debug`… minus Debug.
* To launch a Spring Boot / plain `main()` app you must therefore assemble the whole argv yourself:
  `["-cp", <classpath>, <JvmOptions…>, "com.acme.Main"]` or `["@<argfile>"]`. There is **no** way to attach
  options to a config the IDE's own java/jdtls layer owns, and **no** hook to add JVM options to
  `application`/`spring-boot` configs (nothing in the WIT/`RunConfig` model offers "modify an existing
  config").
* Building the classpath is on us: `Project::find_files` / `read_file` in `configurations()`, or shell out to
  the approved `mvn`/`gradle` wrapper via `host::run_command` (capability `process:mvn`) — the latter costs
  seconds and blows the 2 s `configurations()` budget, so cache it (e.g. `storage`/`fs` in the plugin dir,
  keyed by `pom.xml` mtime) and/or refresh it from a command/`files-changed` subscription.
* `test = true` + `framework = "plugin"` puts results in the test tree *only* through our own
  `test-results.parse` (+ `select-tests`), which requires the project to be trusted and the config id to be
  `plugin:<plugin>:<id>`; chunk ≤64 KiB per call, events ≤4 MiB, `select-tests` input ≤10 000 lists / 4 MiB.

---

## 8. Gaps, risks, UNCONFIRMED

**Hard gaps in the record**
1. **No environment variables.** `run-configuration` has no `env`; `PluginRunDetection` sets
   `env: Default::default()`. So `JAVA_TOOL_OPTIONS` / `MAVEN_OPTS` tricks are **unusable** from the config.
   (`RunConfig.env` + `resolved_env` exist for built-in configs only.) Workaround: obtain env from the
   *user's shell* is impossible too — but `host::run_command`/`Command{program,args,env}` does support `env`
   for direct process runs (not for `start_run`).
2. **No before-run / build hook.** `RunExtras.before_launch` (`Vec<BeforeLaunchTask>`, kinds
   `run|build|tool|command|toolWindow`) is not reachable from a plugin record. If you need "compile then
   run", chain two `start_run`s yourself (fragile) or run the build inside your own `run_command` and then
   start the config.
3. **No debugger** for plugin configs (`debug_adapter: None`); `debug: true` always fails.
4. **No stdin/file redirection, no exit-code capture except `RunExited.status`**, no way to read the run's
   stdout: `run-exited` carries only `id/config/status/killed`. Getting output requires
   `test-results.parse` (test tree), or `host::run_command` with its own child handle.
5. **No user-editable form.** Nothing in the WIT lets the plugin declare per-config form fields; the user
   edits JSON/TOML only by writing `.openidea/run.toml` under the same id (which then shadows your record).

**Capability / sandbox pitfalls**
6. `process:` grants are **per exact token**, absolute paths included; a plugin that wants `java` and `mvn`
   needs two `Risk::High` grants. A JDK upgrade changes the pinned file's hash ⇒ the run refuses
   (`pin.verify()`); the user must re-approve.
7. Your record is **silently dropped** (not offered at all) while the program has no pin — `detect()` filters
   dynamic entries through the pin check. There is no error surface; the Run dialog simply lacks the entry.
   Debugging aid: the pin-missing error string only appears if the config was already listed and the grant
   was revoked (`"run program is no longer granted"`).
8. Untrusted project ⇒ export never runs, configs never appear, and `start_run` would still hit the
   `project is not trusted` paths (`prepare_run_target`/`test_configuration`).
9. `plugin:` is a **reserved id space**: any user/detected config id starting with `plugin:` and
   `detected: true` goes through `external::approved()`; a plugin config in an untrusted/no-provider state
   errors with `"the plugin that detected this configuration is not loaded"`.
10. Changing the marker `files` after install, or listing a marker that does not exist, means the export
    never fires. Markers are project-root-relative (`check_relative`), 1–16, and are checked with
    `.is_file()` (a directory marker does not count).
11. 2 s budget per refresh, 32 concurrent operations, ≤1000 configs, ≤4 MiB serialized export,
    `name` ≤128, 256 args × 4096 B. No pagination.
12. `framework` values other than `"plugin"` give you no parser: for `framework = "junit"` the host's
    `test_configuration` returns `None` and `parse`/`select_tests` are never called.

**UNCONFIRMED (would need files beyond the 100 KB fetch cap)**
13. `crates/ide-host/src/plugins/api.rs` (162 KB) — `program_for_project`, `approved_program_in_project`,
    `ProjectPins` and the `run.start` import implementation. Everything in §4–§5 about pins is derived from
    the *call sites* in `run_detectors.rs` / `execution.rs` / `run_targets.rs` plus the capability parser;
    the exact UI flow that creates a **project** pin (whether the user picks `<project>/mvnw` and it is
    stored under `process:mvnw` for that root) is inferred, not quoted.
14. `crates/ide-host/src/plugins/mod.rs` (130 KB) and `crates/ide-host/src/plugins/files_changed.rs` (30 KB)
    were not read: **other refresh triggers** for `refresh_run_configurations` (project open,
    `files-changed` debounce, command) may exist in addition to the Run-dialog listing. `configurations()`
    being called on project open is plausible but unproven.
15. Whether a **bare** id (`"tests"`) is accepted by `start_run` (see the test-runner example) or whether
    the fully-qualified `plugin:<plugin-id>:<id>` is required. The host code splits only a `plugin:`-prefixed
    string (`strip_prefix(EXTERNAL_PREFIX).and_then(split_once(':'))`), and both `test_configuration` and
    `prepare_debug_run` return errors/`None` for unprefixed ids; a bare id would silently have no owner ⇒
    **pass the qualified id**.
16. Whether `contributes.runConfigurations` additionally requires the marker list to be declared in
    `[capabilities]` of some kind (it does not: capability is purely the implicit `process:` set).
17. `EventKind` activation form — the README tail (activation syntax `on-event:<kind>`) could not be read;
    the enum names (`run-exited`) and `activation = ["on-command:*"]` examples are confirmed.
18. Test-panel wiring for `test = true` without `framework = "plugin"`, and whether `select-tests` is also
    used for "Rerun Failed Tests" in the newer Test panel (only the test-runner README says so).

---

## 9. One-paragraph build recommendation (for the plugin team)

Ship a **dynamic configuration provider** (`[[contributes.runConfigurations]] files = ["pom.xml",
"build.gradle", "build.gradle.kts"]`, `engines.openidea = "^1.4"`, `wasm`), declare
`required = ["process:java"]` (plus `process:mvn`/`process:gradle` / `process:mvnw` if we shell out or let the
user pick a wrapper), `activation = ["on-command:*", "on-event:run-exited"]`, expose one command that calls
`host::start_run("plugin:<our-id>:<local>", &[], false)`; inside `configurations()` read our settings with
`host::setting_string` (JVM defaults), build `<program=java, args=[-cp|@argfile, <defaults…>, main]>`,
keep it under 2 s, and cache the classpath. Accept that the user's built-in Java/Spring configs stay
untouched, that Debug is unavailable, and that env vars are impossible.

---

# Addendum A — three follow-up questions (manifest declaration vs config-supplied `program`, `.cmd` spawn, `runConfigurations` fields)

All quotes below are from `crates/ide-host/src/plugins/{run_detectors,execution,api}.rs` and
`crates/ide-run/src/process.rs` (main branch, fetched 2026-11-09).

## A.1 Which capability string is tested for a config-supplied `program`? (verbatim token — never a manifest token)

`crates/ide-host/src/plugins/run_detectors.rs`, dynamic filter (the *only* place a provider record is turned
into a runnable):

```rust
for ((plugin, project), entries) in self.dynamic.read().iter().filter(|((_, project), _)| project == root) {
    for c in entries {
        let cap = Capability::Process(c.program.clone());
        if !self.project_pins.read().contains_key(&(plugin.clone(), cap.to_string(), project.clone()))
            && !self.pins.read().contains_key(&(plugin.clone(), cap.to_string()))
        {
            continue;
        }
```

* The tested string is `format!("process:{program}")` with **`program` exactly as returned by
  `configurations()`** — no PATH lookup, no PATHEXT expansion, no `root.join()`, no mapping back to a
  manifest-declared bare name.
* Lookup order: project pin `(plugin, "process:<token>", root)` first, then the global pin
  `(plugin, "process:<token>")`.
* On miss the record is **`continue`d**: it is not listed, not prompted, not logged, not an error. It is
  simply absent from the Run dialog.
* `validate_configurations` does not add path checks to `program` (only: non-empty, ≤256 bytes, no
  whitespace), so a `program` of `D:\proj\mvnw.cmd` passes validation and then matches nothing.

Launch time repeats the same lookup in `crates/ide-host/src/plugins/api.rs`:

```rust
pub(super) fn approved_program_in_project(&self, program: &str, root: Option<&Path>) -> Result<PathBuf, HostError> {
    if let Some(pin) = root.and_then(|root| {
        self.shared.project_pins.read().get(&(self.id().into(), format!("process:{program}"), root.into())).cloned()
    }) {
        return pin.verify().map(Path::to_path_buf).map_err(|e| HostError::Denied(e.to_string()));
    }
    let cap = format!("process:{program}");
    // Granted without a pinned file: the program wasn't found when it was
    // allowed, and installing it later approves nothing by itself.
    let pin = self.shared.pins.read().get(&(self.id().to_owned(), cap.clone())).cloned().ok_or_else(|| {
        HostError::Failed(format!(
            "{cap}: `{program}` was not found on PATH when this permission was allowed. Restarting doesn't approve a \
             program installed later: once `{program}` is on PATH, clear {cap} in Settings › Plugins and select it again"
        ))
    })?;
    pin.verify().map(Path::to_path_buf).map_err(|e| HostError::Denied(format!("{cap}: {e}")))
}
```

**This function never creates a pin — it only reads one.** Pins are only ever created by the grant UI
(plugin install / Settings › Plugins), and the grant UI only knows about the strings in the manifest's
`[capabilities] required`. Therefore:

> **A `program` that is not also declared verbatim in `[capabilities] required` can never become granted;
> such configurations are permanently invisible and can never be started (not even via `start_run`, which
> goes through `workspace().run_configuration(...)` → same pins).** There is no prompt for them anywhere.

### Consequence: the two legal ways to launch a project-local Maven/Gradle wrapper

| | Option A — bare wrapper token (portable) | Option B — shell as the declared program (no per-project step) |
|---|---|---|
| manifest | `required = ["process:mvnw"]` | `required = ['process:C:\Windows\System32\cmd.exe']` |
| record `program` | `"mvnw"` | `"C:\\Windows\\System32\\cmd.exe"` (no whitespace — passes `validate_configurations`) |
| pin that must exist | project pin `(plugin, "process:mvnw", <root>)` — the user approves *the file* in Settings › Plugins **once per project** | global pin `(plugin, "process:C:\…\cmd.exe")` — granted at install, machine-wide |
| args | `["-B","-Dmaven.repo.local=…","package"]` (the pinned `mvnw.cmd` receives them directly) | `["/c","mvnw.cmd","-B","package"]` |
| weaknesses | invisible until approved; `pin.verify()` hashes the file, so **editing/replacing `mvnw.cmd` invalidates the pin** until re-approved; every new project needs re-approval | Windows-only manifest; `cmd /c` adds a quoting layer; killing must kill the tree (the IDE does: `own_group`/`kill_tree`) |

There is no third option: a project-relative capability (`process:./mvnw`) is a hard parse error
(`"…: use a program name or an absolute path"`), and `covers()` is exact-match, so a `process:java` grant
does not cover `process:mvnw`.

## A.2 How a `.cmd` is spawned — no `SpawnMode` in the run path, and no shell

`crates/ide-run/src/process.rs::invocation` is the *only* decision point:

```rust
const SHELL_CHARS: &[char] = &['|','&',';','<','>','(',')','$','`','\\','"','\'','*','?','[',']','#','~','=','%','{','}','\n'];

let looks_like_shell = command.contains(char::is_whitespace) || command.contains(SHELL_CHARS);
if config.args.is_empty() && looks_like_shell && !cwd.join(command).is_file() {
    return shell_invocation(command, windows, std::env::var("COMSPEC").ok());   // cmd /D /S /C "…", raw_args
}
```

* Shell mode needs **`args.is_empty()`** — a plugin configuration always has args, and `program` may not
  contain whitespace, so **the `cmd.exe`/`/bin/sh` path is unreachable for plugin records**.
* Spawn is `Command::new(&inv.program)` + `cmd.args(&inv.args)` (no `raw_args`, no shell), with Windows
  `creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)`; a spawn failure surfaces as
  `IdeError::unavailable("command not found: {config.command}")`.
* `inv.program` at that moment is the **pinned absolute path** (`run_operation_sink` rewrites
  `config.command = binary.to_string_lossy()` and sets `config.detected = false` immediately before launch;
  `prepare_debug_run` does the same for Debug).
* **`SpawnMode` / `ShellScript` do not exist in `crates/ide-run/src/process.rs` nor in
  `crates/ide-core/src/process.rs`** (that file has `prepare`, `own_group`, `kill_tree*`, `host_env`,
  `executable_names`, `find_executable_in`, `DEFAULT_PATHEXT = ".COM;.EXE;.BAT;.CMD"`). The designer's
  `SpawnMode::ShellScript` sighting is from another subsystem (tools/terminal/deploy), **not** from the
  run-configuration launcher.
* Resolution of the *program token* is not done by the host at all for plugin records (only pins are
  consulted). For built-in configs `resolve_program` is the one that expands PATHEXT
  (`kotlinc` → `kotlinc.bat`) and prefers the run directory first.
* So a `.cmd` reaches `Command::new` as-is and **works**: Rust's `std` itself invokes batch files. The Rust
  Security Response WG advisory for CVE-2024-24576 (Rust 1.77.2, 2024-04-09) states it plainly — *"the Rust
  standard library did not properly escape arguments when invoking batch files (with the `bat` and `cmd`
  extensions) on Windows using the `Command` API"* — i.e. `Command` natively routes `.bat`/`.cmd` through
  `cmd.exe` and, since 1.77.2, escapes the arguments. This is also why `Invocation.raw_args` exists
  (*"Windows `cmd /S /C "…"`: cmd does not parse C runtime quoting"*) and why `SHELL_CHARS` includes `%`,
  `\`, `"`. **Verified from the advisory, 2026-11-09.**
  ⇒ `program = "mvnw"` pinned to `…\mvnw.cmd` with plain `args` is a supported launch; no `cmd /c`
  pre-wrapping needed. Keep Option B in reserve only if the wrapper must also work when the pinned file is
  a POSIX `mvnw` script (std refuses shell scripts: *"a POSIX `gradlew` script is not runnable"*).

## A.3 `[[contributes.runConfigurations]]` — exact accepted fields and the marker check

`crates/ide-host/src/plugins/execution.rs::configurations_declaration`, verbatim:

```rust
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Decl {
    files: Vec<String>,
}
pkg.manifest.needs_api("contributes.runConfigurations", 4)?;
let decl: Decl = ide_extension::decode("runConfigurations", value)?;
if pkg.manifest.wasm.is_none() || decl.files.is_empty() || decl.files.len() > 16 {
    return Err("runConfigurations needs code and 1-16 marker files".into());
}
for file in &decl.files {
    ide_extension::check_relative(std::path::Path::new(file), "files")?;
}
```

* Accepted fields: **`files` only** — 1..=16 relative paths. `deny_unknown_fields` ⇒ `id`, `command`,
  `args`, `name`, … are a hard error. (Contrast: the *static* `[[contributes.runDetectors]]` entry is
  `{id, name, files, command, args, test, framework}` with `deny_unknown_fields` as well.)
* Requirements: `needs_api(…, 4)` = API 1.4, and `pkg.manifest.wasm.is_some()`.
* Marker check, verbatim from `refresh_run_configurations` (per project, before the export is called):

```rust
let owners: std::collections::BTreeSet<_> = self.ui.items("runConfigurations", None).into_iter()
    .filter(|d| d.decl["files"].as_array()
        .is_some_and(|files| files.iter().filter_map(|f| f.as_str()).any(|f| root.join(f).is_file())))
    .map(|d| d.plugin_id).collect();
self.shared.run_configurations.write().retain(|(p, r), _| r != root || owners.contains(p));
```

  ⇒ literal `root.join(f).is_file()`: **a glob never matches** (and `check_relative` does no glob
  expansion), it must be a real file at that relative path; `is_file()` (not `exists()`, unlike the static
  detector path). If no marker matches, `configurations()` is **not called at all**; if a plugin's markers
  disappear, `retain` drops its cached set for that root.
* The export itself still requires a **trusted** project (`if !workspace().trusted(root) { return; }`).

## A.4 Install prompt and pin scope

* `[capabilities] required` is the sole source of the capability list. `capability.rs` produces
  `CapabilityInfo { id, risk, description, optional, granted }` for the install page; `Capability::Process`
  is `Risk::High`, described as `Run {bin} — equivalent to running arbitrary code on this machine`, so it
  gets its own consent dialog at install time; that is when the pin is resolved and stored under
  `(plugin, "process:<declared token>")`.
* Project pins are `Arc<RwLock<HashMap<(String, String, PathBuf), ProgramPin>>>` → keyed by
  **plugin + `process:<token>` + project root**, i.e. a project-scoped approval applies to **that project
  only**, never to every project.
* `pin.verify()` is executed on every listing filter, every `start_run`, every Debug and every IDE rerun, so
  a changed/re-signed/chmod-ed binary revokes itself until re-approved. For a Maven wrapper that is
  regenerated by `mvn wrapper:wrapper`, expect a re-approval cycle.
* Missing-grant behaviour at launch: `HostError::Failed` (surfaced as a PermissionDenied-style error in the
  Run console) with the `…was not found on PATH when this permission was allowed…` message, or
  `"run program is no longer granted"`, or
  `"allow process:{cmd} for plugin {plugin} in Settings › Plugins to run this configuration"`.

**UNCONFIRMED (would need `crates/ide-host/src/plugins/mod.rs`, 130 KB, over the fetch cap, and
`install.rs`, which failed to fetch):** the exact Settings › Plugins widget that creates a *project* pin
("select it again" flow), and whether the pin dialog is reachable for a capability declared but resolving to
nothing. Everything else above is quoted from code.
