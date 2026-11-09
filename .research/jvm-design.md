# JVM default parameters — domain design (implementable spec)

Status: **DRAFT v1** — owner `jvm-designer`, task `task-3`.
Audience: the Lead implementing `src/` of the OpenIDEA plugin `jvm-defaults`.
Companions: `.research/api-host.md` (host/SDK signatures), `.research/api-run.md` (run-configuration contribution
semantics, capability rules). Where those two disagree with this doc, **they win** — this doc owns the JVM/Maven/
Gradle/Spring Boot/IDE-file-format domain only.

Everything marked **[UNCONFIRMED]** depends on host behaviour or on a specific tool version we could not execute.
Shell here has no network and no cargo; nothing in this document was verified by running a build.

Grounding used (read on this machine, not from the internet):

- `D:\workspace\water\water-server` — real Maven multi-module Spring Boot 3.5.11 / Java 21 project, `mvnw`+`mvnw.cmd`,
  `.mvn/jvm.config`, `.idea/workspace.xml` with 20+ `SpringBootApplicationConfigurationType` and `JUnit` run configs,
  `.idea/jvm-parameter-settings.xml` (written by the real IntelliJ **JvmParameter** plugin), `bin/run-gateway.bat`,
  `bin/app_boot.sh`, `build/jars/*.jar`, `attach-gateway/target/classes`.
- `D:\workspace\jhs\server` — second Maven multi-module Spring Boot project (same shape).
- `D:\workspace\mes\detection-client-native\.vscode\launch.json` — real JSONC launch file (comments present).
- `D:\DevSoft\OpenIDEA\openidea.exe` exists (single exe, no readable SDK that we can inspect from here).

---

## 0. TL;DR for the implementer

1. **Default strategy = `bootJar`** when a runnable jar exists (`java <vm> -jar <jar> <args>`), because it is the only
   strategy that needs no shell, no `.cmd`, no build tool and no derived classpath. It is a *direct* `process:java`
   spawn with a plain argv — exactly the shape the `run-configurations` export can express.
2. **Fallback = `springBootMaven`** (`mvnw.cmd spring-boot:run -Dspring-boot.run.jvmArguments=… -Dspring-boot.run.arguments=…`)
   for projects that have no built jar. On Windows this needs `cmd.exe` because a `.cmd` is not an executable image
   (see §1.2, spawn modes).
3. **Maven Spring Boot canonical argv** (Windows, project root as cwd):

   ```text
   D:\workspace\water\water-server\mvnw.cmd
     -B -pl attach-gateway -am
     -Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev
     -Dspring-boot.run.arguments=--server.port=8081
     -Dspring-boot.run.fork=true
     spring-boot:run
   ```

   Each `-D…=…` is **one argv element** (no shell). `-Dspring-boot.run.arguments` only lands in the *app*, never in
   the JVM; VM options only take effect in a **forked** app JVM (hence the explicit `fork=true`).
4. Settings keys (camelCase): `enabled, vmOptions, programArgs, activeProfile, profiles, mergeMode, dedupe, strategies,
   preferStrategy, emitAllStrategies, maxConfigurations, inheritIdeConfigs, importJvmParameterFile, writeRunXml,
   previewOnly, printPreviewOnRefresh, javaExecutable, jrePath, cwdMode, customCwd, extraVmOptions, configOverrides,
   buildDeps, offline, jarSearchDirs, spawnMode` (§4).
5. Id scheme: `jvm-defaults:<strategy>:<module-slug>[.<jar-slug>]`, e.g.
   `jvm-defaults:bootjar:attach-gateway`, `jvm-defaults:spring-boot-maven:attach-gateway` (§5.3).
6. There is **no env field** in the record, so `JAVA_TOOL_OPTIONS`/`JDK_JAVA_OPTIONS`/`_JAVA_OPTIONS`/`MAVEN_OPTS`
   injection is impossible through contributed configs. File-based equivalents (`.mvn/jvm.config`) and
   `host::start(Command{env})` side paths are covered in §3.0 as opt-in escape hatches, with their traps.

---

## 1. Launch strategies

### 1.0 Shared model: `LaunchPlan`, spawn modes, quoting

The export only ever gives the host `{ program, args: list<string>, cwd }`. So every strategy compiles down to one
`LaunchPlan`:

```rust
// src/model.rs
pub enum SpawnMode {
    Direct,       // program is an executable image (java.exe, /bin/java, sh)
    ShellScript,  // program is a .cmd/.bat/.sh wrapper -> needs an interpreter (see §1.0.2)
}

pub struct LaunchPlan {
    pub program: String,       // absolute path preferred (capability is pinned per program)
    pub args: Vec<String>,     // ALREADY final argv; never shell-quoted in Direct mode
    pub cwd: Option<String>,   // absolute
    pub spawn: SpawnMode,
    pub strategy: Strategy,    // BootJar | SpringBootMaven | SpringBootGradle | MavenExec | PlainMain | JUnit
}
```

`Strategy` → display label:

| Strategy | Label (used in config `name`) |
|---|---|
| `BootJar` | `JAR` |
| `SpringBootMaven` | `Spring Boot (Maven)` |
| `SpringBootGradle` | `Spring Boot (Gradle)` |
| `MavenExec` | `exec:exec` |
| `PlainMain` | `main class` |
| `JUnit` | `test` |

#### 1.0.1 Platform detection without `std::env`

The component is WASM; we cannot read `cfg!(windows)` at runtime for the *host* platform and should not trust the
build target. Detect from the project root we are handed:

```rust
fn is_windows(root: &str) -> bool { root.contains('\\') || root.as_bytes().get(1) == Some(&b':') }
```

This is what picks `mvnw.cmd` vs `mvnw`, `java.exe` vs `java`, `gradlew.bat` vs `gradlew`, and `;` vs `:` classpath
separators.

#### 1.0.2 Spawn modes (this is the sharpest edge in the whole design)

- **Direct**: `program` is a PE/ELF image (`java`, `java.exe`, `/bin/sh`). Rust `Command::new(program).args(args)`
  passes argv verbatim; **no quoting of any kind must be applied**. This is the mode every `java …` strategy uses.
- **ShellScript**: `program` is `mvnw.cmd`, `gradlew.bat`, `mvnw` (a shell script without a shebang bit on Windows
  checkouts). Windows `CreateProcess` cannot start a `.cmd`/`.bat`, and Rust's `Command` does not shell out, so the
  only portable form is:

  ```text
  cmd.exe /d /s /c " <abs path to mvnw.cmd> <args...> "
  ```

  **[UNCONFIRMED]** whether the OpenIDEA host itself special-cases `.cmd`/`.bat` (i.e. spawns them via `cmd.exe`)
  before handing the argv to `CreateProcess`. If it does, we must NOT wrap in `cmd.exe` ourselves (double wrap breaks
  quoting). Therefore: **one settings flag** `spawnMode: "auto" | "direct" | "cmd"` (add to §4, default `auto`) and
  `auto` = wrap only if `program` ends with `.cmd`/`.bat` **and** the host reports a failure the first time
  (remember the verdict in `host::storage` under `jvm-defaults/spawnmode/<hash(program)>`).

  On POSIX, `ShellScript` = `["/bin/sh", "<mvnw>"]` so a missing exec bit (common when a repo was committed from
  Windows) does not break the launch.

**cmd.exe quoting rules we must implement when `spawnMode == cmd`** (`fn cmd_quote(s: &str) -> String`):

1. If the token is empty → `""`.
2. If it contains a space, tab, or one of `&|<>^()` → wrap in `"…"` (inside double quotes `cmd` treats `& | < > ( ) ^`
   literally, so wrapping is sufficient; do **not** `^`-escape inside quotes).
3. If the token ends with `\`, double that trailing backslash before the closing quote (otherwise the `\"` sequence is
   misparsed by the C runtime the child inherits).
4. `"` inside a token: **reject** (`V_E_QUOTE_IN_SHELL_MODE`). `cmd` has no reliable escaping for it (`""`, `\"` and
   `^"` all behave differently per context); silently mangling a user's `-D` value is worse than refusing it.
5. `%` inside a token: **reject** (`V_E_PERCENT_IN_SHELL_MODE`) — `cmd /c` performs `%VAR%` expansion inside quotes
   too. Rare in VM options; if unavoidable, tell the user to use the `bootJar`/`exec:exec` strategy.

**POSIX quoting** (`fn sh_quote(s: &str) -> String`) when a rendered preview must be pasted into a shell:
`'` → `'\''`, wrap the whole token in `'…'`; empty token → `''`. Single quotes make every other character literal.

Rendered previews are **display only** (§3.4) — they are never fed back into `program`/`args`.

#### 1.0.3 Program catalogue (capability surface)

| program (Windows) | program (POSIX) | Strategies | Notes |
|---|---|---|---|
| `java.exe` / `java` | `java` | BootJar, PlainMain, JUnit-console | Preferred: benign, tiny argv, Direct spawn |
| `…\mvnw.cmd` | `…/mvnw` | SpringBootMaven, MavenExec, JUnit | `.cmd` ⇒ ShellScript ⇒ needs `cmd.exe` capability |
| `…\gradlew.bat` | `…/gradlew` | SpringBootGradle | note: `.bat`, not `.cmd` |
| `C:\Windows\System32\cmd.exe` | — | wrapper interpreter | only if the host does not do it itself |
| `/bin/sh` | `/bin/sh` | wrapper interpreter (POSIX) | fallback for non-executable `mvnw` |

`java` absolute path resolution: setting `javaExecutable` (§4) → else `"java"` and rely on the host/capability to
resolve and pin it. **[UNCONFIRMED]** whether `program` may be a bare name resolved through `PATH`, or must be an
absolute path because the `process:<program>` capability is pinned by absolute path + content hash. If it must be
absolute, the plugin needs a `javaExecutable` setting (the project SDK path lives in `jdk.table.xml`, outside the
project, which we cannot read) — this is open question **Q1**.

---

### 1.1 Spring Boot via Maven Wrapper (`mvnw spring-boot:run`)

**When**: `pom.xml` mentions Spring Boot (`spring-boot-maven-plugin`, `spring-boot-starter-parent` or
`spring-boot-dependencies`) and a wrapper exists at the multi-module root. Works without any pre-built artifact.

**argv template**

```text
program : <ROOT>\mvnw.cmd                     (Windows, SpawnMode::ShellScript)
          <ROOT>/mvnw                          (POSIX,  SpawnMode::ShellScript via /bin/sh if not +x)
cwd     : <ROOT>                               (the multi-module root that owns the wrapper)

args    : [
            "-B",                                                     # batch mode: no progress-bar spam
            "-pl", "<MODULE_RELPATH>",                                # attach-gateway
            "-am",                                                    # also build required upstream modules
            ["-o"]?,                                                  # settings.offline
            ("-Dspring-boot.run.jvmArguments=" + join_tokens(vm_tokens)),
            ("-Dspring-boot.run.arguments="    + join_tokens(prog_tokens)),
            "-Dspring-boot.run.fork=true",                             # force fork so VM opts apply
            "spring-boot:run"
          ]
```

Concrete example (attach-gateway of `water-server`):

```text
program = D:\workspace\water\water-server\mvnw.cmd
cwd     = D:\workspace\water\water-server
args    = [
  "-B", "-pl", "attach-gateway", "-am",
  "-Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev",
  "-Dspring-boot.run.arguments=--server.port=8081",
  "-Dspring-boot.run.fork=true",
  "spring-boot:run"
]
```

**Semantics / caveats**

- `spring-boot.run.jvmArguments` is a **single string** that the plugin tokenizes on whitespace, so the whole
  `-D…=…` must be **one argv element** whose *value* contains spaces. No shell is involved, so this is fine — but it
  means **a single VM token may not itself contain whitespace** (the plugin's splitter would break it). Validation rule
  `V_E_SPACE_IN_SERIALIZED` (§3.5) fires for strategies that serialize into a property.
- `fork=true` is passed explicitly: VM arguments only reach the app when the plugin forks, and the default of
  `spring-boot.run.fork` differs across Boot 2.x/3.x versions. **[UNCONFIRMED]** exact default per version; explicit
  `true` is version-independent and harmless. `fork=true` costs ~0.5–1 s extra startup.
- `spring-boot.run.arguments` reaches the app **as program arguments** (`--server.port=8081`, not `-D…`).
  For `-D` style app properties you must use JVM options (`-Dspring.profiles.active=dev` in `jvmArguments`).
- `-pl <module> -am`: builds upstream reactor modules in the same session. Cold starts are slow (whole reactor),
  warm starts are ok. Alternative `-pl <module>` only, when the modules were `mvn install`ed previously; make it a
  toggle `buildDeps` (default `true`).
- **Do not use `-q`.** `spring-boot:run` pipes the forked app's stdout through the Maven plugin's logger; `-q`
  (WARN level) usually swallows the app's INFO log, which is the main reason to run the app at all.
  **[UNCONFIRMED]** for the exact Boot versions where the forked output is logger-mediated rather than inherited.
  Use `-B` instead (no ANSI/progress noise, full logs).
- The `.mvn/maven.config` file, if present, is honoured automatically and may add flags we do not know about —
  don't fight it.
- If `<MODULE_RELPATH>` is empty (single-module project), omit `-pl`/`-am`.
- Spring Boot 3.x also accepts `-Dspring-boot.run.profiles=`; we **do not** use it, we pass
  `-Dspring.profiles.active=` via VM options so the same setting works for every strategy.

**Maven property/quoting rules for `-D…=…`** (used by S1, S2, S5): a Maven user property is split at the **first**
`=`, so the value may contain `=` freely: `-Dspring-boot.run.jvmArguments=-Dfoo=a=b` is legal. What it may not contain
is whitespace *inside one token*. If the user needs `-Dmsg=hello world`, the only portable spelling is
`-Dmsg='hello world'` **inside** the serialized property, i.e.

```text
-Dspring-boot.run.jvmArguments=-Xmx1g -Dmsg='hello world'
```

and the Maven plugin's own tokenizer must honour the quotes. **[UNCONFIRMED]** (spring-boot-maven-plugin uses a
`CommandLineUtils`-style splitter; quoted spans are believed to be preserved). We keep this as a *warning*
(`V_W_QUOTED_IN_SERIALIZED`) and document it rather than blocking it.

---

### 1.2 Spring Boot via Gradle Wrapper (`gradlew bootRun`)

**When**: `settings.gradle(.kts)` + `build.gradle(.kts)` containing `org.springframework.boot` /
`spring-boot-gradle-plugin`, and `gradlew`/`gradlew.bat` at the build root.

**Program arguments** — officially supported, Gradle ≥ 5:

```text
program : <ROOT>\gradlew.bat   (Windows, ShellScript)   |   <ROOT>/gradlew  (POSIX)
cwd     : <ROOT>
args    : ["bootRun", "--args=" + join_tokens(prog_tokens)]
```

**VM arguments — the trap.** There is **no Gradle CLI flag** that feeds `jvmArgs` into the `bootRun` task:

| Candidate | Reality |
|---|---|
| `gradlew bootRun -Dspring-boot.run.jvmArguments=…` | ❌ that is a **Maven** property; Gradle ignores it |
| `gradlew bootRun -Dorg.gradle.jvmargs=-Xmx1g` | ❌ sets the **Gradle daemon** JVM (and can even break the daemon); the app is a separate forked JVM |
| `gradlew bootRun -Dorg.gradle.jvmargs` inside `gradle.properties` | ❌ same trap, worse: persistent |
| `JAVA_TOOL_OPTIONS`/`MAVEN_OPTS` env | ❌ no env field in our record (§3.0) |
| init script `-I` (below) | ✅ works, no build-file edit |
| build-file edit (`bootRun { jvmArgs = … }`) | ✅ works, but mutates the user's source — out of scope |

**Recommended**: for Gradle projects prefer **`bootJar`** (§1.3) — `build/libs/*.jar` + `java -jar` needs nothing
from Gradle at run time. If no jar exists yet and the user insists, use the **init-script injection**:

Write (once, idempotently) `<ROOT>/.gradle/jvm-defaults.init.gradle`:

```groovy
// written by OpenIDEA plugin jvm-defaults -- safe to delete
gradle.projectsEvaluated {
    def vm = System.getProperty('jvmd.vmArgs')
    def pa = System.getProperty('jvmd.progArgs')
    allprojects { p ->
        p.tasks.matching { it.name == 'bootRun' }.configureEach { t ->
            if (vm) t.jvmArgs(vm.split(' '))
            if (pa) t.args(pa.split(' '))
        }
    }
}
```

argv:

```text
args = ["-I", "<ROOT>/.gradle/jvm-defaults.init.gradle",
        "-Djvmd.vmArgs="   + join_tokens(vm_tokens),
        "-Djvmd.progArgs=" + join_tokens(prog_tokens),
        "bootRun"]
```

Notes:
- Matching by task **name** (`bootRun`) avoids referencing `org.springframework.boot.gradle.tasks.run.BootRun`,
  which is not on the init script's compile classpath until the plugin is applied.
- CLI `-D` become **build-JVM system properties**; `System.getProperty` in the init script sees them.
  **[UNCONFIRMED]** under a warm daemon that Gradle forwards CLI `-D` per invocation (it is documented behaviour for
  the build JVM, but daemon reuse makes it worth a smoke test).
- `configureEach` needs Gradle ≥ 4.9; `--args` needs ≥ 5.0 (both fine for any Boot 2.2+/3.x project).
- `vm.split(' ')` cannot carry values with spaces — same `V_E_SPACE_IN_SERIALIZED` rule.
- The init script lives under `.gradle/` which is normally git-ignored; never write it to the repo root.
- If the plugin may not write that path, fall back to a generated file under the plugin's storage and pass its absolute
  path — but `storage` paths are host-managed and may not be spawnable-cwd-relevant; **[UNCONFIRMED]** that the host
  allows reading plugin-storage files as process inputs. `.gradle/` inside the project is the safe choice.

---

### 1.3 Executable / boot jar (`java -jar`) — **default strategy**

**When**: a runnable jar exists. This is the only strategy with zero shell, zero build tool, zero classpath
derivation. Prefer it whenever `target/*.jar` or `build/libs/*.jar` is present.

**argv template**

```text
program : <JAVA>                     (javaExecutable setting, else "java"; Direct)
cwd     : per cwdMode, default = the jar's parent directory
args    : [ vm_tokens..., "-jar", "<JAR ARG>", prog_tokens... ]
```

Order matters: **all VM options must precede `-jar`** (JVM treats everything after `-jar <file>` as app args — a
`-D` after `-jar` becomes an app argument and is silently ignored by the JVM).

Jar argument form: if `cwd == jar.parent` → bare filename (`attach-gateway.jar`), else absolute path. Bare filename is
nicer in the preview and matches the project's own launcher (`bin/run-gateway.bat` does `cd ../attach-gateway/target`
then `java … -jar attach-gateway.jar`).

**Jar discovery order** (first hit wins; **deterministic**):

1. `settings` override: `configOverrides[id].jarPath`, else the remembered pick in
   `host::storage["jvm-defaults/pick/<id>"]` (still exists + still parses as a jar name ⇒ use it; this is what keeps
   the id stable across version bumps).
2. `<module>/target/*.jar` (Maven; `finalName` may drop the version, see below).
3. `<module>/build/libs/*.jar` (Gradle `bootJar` output).
4. Custom dirs from settings `jarSearchDirs` (e.g. this repo's `build/jars/*.jar` deploy copy).
5. `<module>/*.jar` — last resort, warns.

Within a directory, filter **out** names matching (case-insensitive):

```text
*-sources.jar, *-javadoc.jar, *-tests.jar, *-test.jar, *-plain.jar,
*.jar.original, *-original.jar, *.jar.sha1, *.jar.md5, *-shaded.jar?, (leave shaded in: it is usually runnable)
```

then prefer, in order:
(a) a jar whose base name equals the module directory name or the Maven `finalName`/`artifactId`
(this repo sets `<finalName>${project.artifactId}</finalName>` in a profile ⇒ `attach-gateway.jar`, **no version**),
(b) otherwise the newest by `stat` mtime, tie-broken by name ascending.

Jar vs. class dir sanity: `target/classes/` next to `target/x.jar` is normal, ignore it. Never treat
`*.jar.original` as runnable (that is Spring Boot's pre-repackage backup; running it fails with a missing
`Main-Class`).

**Caveats**

- Boot 3.x repackages `x.jar` into a "fat" jar whose launcher `org.springframework.boot.loader.launch.JarLauncher`
  is embedded; `java -jar x.jar` just works. If the module is a *library* (`<packaging>jar</packaging>` but no boot
  plugin), the jar has no `Main-Class` ⇒ launch fails. Guard: only offer this strategy when the module is known to be
  an application (main class found — §5.1) **or** the jar name matches a module that has a `*Application` class.
- `-Dfile.encoding=UTF-8` is *not* injected by default even though this project's script sets `-Dfile.encoding=utf-8`;
  see presets in §4 (`-Dfile.encoding=UTF-8` is available as an opt-in preset).
- Spring Boot's `--spring.config.location` / `application.yml` resolution is relative to the **working directory**;
  this is why `cwdMode` defaults to the jar's parent (`…/target`) in this strategy: it reproduces what the project's
  own `bin/run-gateway.bat` does. **[UNCONFIRMED]** which cwd the project authors actually expect for external
  config; expose `cwdMode` so the user can pick.
- Requires a build first. We never build: a run configuration must not mutate the workspace, and the host's refresh can
  be frequent. The config **name** carries `(build first)` when no jar was found but the strategy is still offered.

---

### 1.4 Plain main class + derived classpath (`java -cp … Main`)

**When**: no wrapper, no jar, but a main class and a classpath can be derived. Lowest-confidence strategy; **disabled
by default** (`strategies.plainMain = false`).

**argv template**

```text
program : <JAVA>
cwd     : <module dir> (default)
args    : [ vm_tokens..., "-cp", "<CLASSPATH>", "<MAIN_CLASS>", prog_tokens... ]
```

**Classpath derivation strategies** (in preference order), all read-only:

| # | Source | Algorithm | Robustness / cost |
|---|---|---|---|
| CL-1 | `build/install/<name>/lib/*` (Gradle `installDist`) | `module/build/install/*/lib` exists ⇒ classpath = `<that>/lib/*` (Java expands a trailing `*` to every `.jar` in the dir — **one token, no enumeration**) | ✅ very robust; requires `installDist` to have run |
| CL-2 | Maven cached dependency list | `module/target/classpath.txt` (written by `mvn dependency:build-classpath -Dmdep.outputFile=target/classpath.txt`) ⇒ `module/target/classes` + `;` + file content. Also accept `target/dependency/*` (from `dependency:copy-dependencies`) ⇒ `target/classes;target/dependency/*` | ✅ robust when present; ❌ never generate it ourselves (needs a build) |
| CL-3 | `.idea` module metadata | parse `.idea/modules.xml` → `*.iml` → `<output url="file://$MODULE_DIR$/target/classes">` + lib entries; plus `.idea/libraries/*.xml` `<CLASSES><root url="jar://$MAVEN_REPOSITORY$/…!/"/>` | ⚠️ medium: `$MAVEN_REPOSITORY$` and `$MODULE_DIR$` macros must be expanded, and **this repo has no `.idea/libraries` or `modules.xml` at all** (verified: only `compiler.xml`, `misc.xml`, `workspace.xml`, …) ⇒ unusable here |
| CL-4 | Gradle `build/classes/java/main` + `build/resources/main` only | no dependency jars ⇒ usually `NoClassDefFoundError` | ⚠️ only for zero-dependency demos |
| CL-5 | IDE-provided classpath | **unavailable** — the export has no classpath field and no hook to ask the IDE | ❌ |
| CL-6 | `target\classes;target\dependency\*` | `mvn dependency:copy-dependencies` output | ✅ if present |

Selection: try CL-1 → CL-2/CL-6 → CL-3 → CL-4; require `target/classes` (or `build/classes/java/main`) to exist,
otherwise the strategy is not offered at all (a classpath of only jars cannot run an app class).

Path separator: `;` on Windows, `:` on POSIX. Classpath tokens containing `;`/`:`) are inherently ambiguous — reject
with a warning if any element contains the separator (`V_W_CP_SEPARATOR`).

**Caveats**: shell-free, so `-cp` value with spaces is fine *as one argv element*, but Java on Windows has a
command-line length limit (32767 chars) — for very large classpaths prefer CL-1's `lib/*` wildcard. Also: a *directory*
`classes` entry plus `-cp` ordering can shadow resources; keep module output first, dependencies after.

---

### 1.5 Maven `exec` plugin

**When**: no jar yet, module has a main class, and either (a) `exec-maven-plugin` is already configured, or (b) the
project uses Maven and we need VM options + a correct classpath **without deriving the classpath ourselves**.

**(a) `exec:exec` — recommended Maven fallback (forked JVM ⇒ VM options work)**

```text
args = ["-B", "-q", "-pl", "<MODULE>", "-am", "exec:exec",
        "-Dexec.executable=" + JAVA,
        "-Dexec.args=" + vm_tokens_joined + " -classpath %classpath " + main_class + " " + prog_tokens_joined]
```

`%classpath` is expanded by the plugin to the module's runtime classpath (exec-maven-plugin 3.x) — this is the cheap
way to avoid CL-1…CL-4 entirely. `-q` is safe here because `exec:exec` inherits the child's stdio directly (the child
writes to the same handles), unlike `spring-boot:run`'s logger-mediated output. Requires the `exec-maven-plugin` to be
resolvable from the local repo (offline OK if already downloaded) — **[UNCONFIRMED]** for the exact plugin version that
first supported `%classpath` (3.0+ believed).

**(b) `exec:java` — same JVM as Maven ⇒ VM options do NOT apply**

```text
args = ["-B", "-pl", "<MODULE>", "exec:java",
        "-Dexec.mainClass=" + main_class,
        "-Dexec.args=" + join_tokens(prog_tokens)]
```

- `exec:java` runs the main class **inside Maven's own JVM**. `-Dexec.jvmArgs`/`jvmArguments` are not a thing for
  `exec:java`; the only ways to give that JVM VM options are `MAVEN_OPTS` (no env field ⇒ unavailable) or
  `.mvn/jvm.config` (file ⇒ §3.0 Mode D). Therefore `exec:java` is only offered as a **program-arguments-only**
  strategy, and the UI/preview must say so explicitly (`vm options NOT applied`).
- `exec:args` is a single string, split by the plugin's own tokenizer ⇒ same single-token-without-space rule.

**Offer policy**: emit `MavenExec` only when (i) `exec-maven-plugin` appears in the module/root pom **or** the user
enabled `strategies.mavenExec` (default `false`), and (ii) the module has a main class.

---

### 1.6 JUnit tests (`test: true`)

The record has `test: bool` + `framework: option<string>`, and the IDE already ships JUnit run configs
(`type="JUnit"`, verified in `water-server/.idea/workspace.xml`). VM options for tests are needed for the same reason
(profiles, `-D` props, agent flags).

**Maven / Surefire** — VM options go through `argLine`:

```text
program = <ROOT>\mvnw.cmd
cwd     = <ROOT>
args    = ["-B", "-pl", "<MODULE_RELPATH>",
           "-Dtest=<Class>[#<method>]", "-DfailIfNoTests=false",
           "-DargLine=" + join_tokens(vm_tokens),
           "test"]
```

- `-DargLine=…` overrides `maven-surefire-plugin`'s `argLine` **including any `${argLine}` the build sets for JaCoCo**
  ⇒ you can silently kill coverage instrumentation. Detection: if the pom mentions `jacoco`, append
  `-Djacoco.skip=true` and warn (`V_W_ARGline_CLOBBERS`), or better: do not enable JUnit strategy by default for
  JaCoCo projects.
- `-Dtest=Class#method` is Surefire 2.19+ syntax; `-Dtest=Class` for the whole class. `-Dsurefire.failIfNoSpecifiedTests=false`
  is the modern property; `-DfailIfNoTests` is the older one — pass both (harmless).

**Gradle** — `gradlew test --tests "<Class>"` with the VM args via the same init-script trick, matching task name
`test` instead of `bootRun` (§1.2).

**JUnit console launcher** (only if `junit-platform-console-standalone-*.jar` is found in `~/.m2`/`lib`): too many
unknowns (classpath + engine discovery) — **do not implement in v1**.

`framework`: use `"junit"` **[UNCONFIRMED]** value set accepted by the host; if it only accepts a free string, `"junit"`
is safe. `test: true` for every JUnit plan.

---

### 1.7 Strategy selection matrix

| Situation | Emit |
|---|---|
| jar found **and** module is an app | `BootJar` (preferred), plus others if `emitAllStrategies` |
| Spring Boot + `mvnw` + no jar | `SpringBootMaven`, `MavenExec` |
| Spring Boot + `gradlew` + no jar | `SpringBootGradle` (init-script VM injection), `BootJar` if `build/libs` exists |
| plain Maven + main class + `target/classes` | `MavenExec` (`exec:exec` if plugin resolvable) → `PlainMain` (CL-2/CL-1) |
| plain Gradle + `installDist` | `PlainMain` with CL-1 |
| `.idea` JUnit configs present | one `JUnit` plan per test config (inheriting its class/method/module) |
| nothing detectable | **emit nothing**, one notification, no error |

`preferStrategy` (settings): `auto` = the order above; otherwise the named strategy is placed first, and (with
`emitAllStrategies=false`) it is the only one emitted.

---

## 2. Reading the IDE's existing configuration files

Goal: **inherit the user's real launch settings** (main class, module, classpath hints, existing VM/program params,
JRE, envs) so our contributed configurations are recognisable duplicates of what they already run — and so
per-configuration values can win over our defaults (§3.2).

### 2.1 JetBrains: locations and shapes

| Location | Root element | Notes |
|---|---|---|
| `.run/*.run.xml` | `<component name="ProjectRunConfigurationManager">` | shared per-project configs, the modern format (one config per file) |
| `.idea/runConfigurations/*.xml` | `<component name="ProjectRunConfigurationManager">` | the older shared format |
| `.idea/workspace.xml` | `<project>` → `<component name="RunManager">` (may also carry `<component name="RunManager" selected="…">`) | **local** configs, includes IDE-generated `temporary="true"` ones; verified real sample in `water-server` |
| `.idea/workspace.xml` `<component name="RunManager">` → `<configuration … default="true">` | template | **skip** — these are type templates, not user configs |

Real sample (verified, `water-server/.idea/workspace.xml`):

```xml
<component name="RunManager" selected="JUnit.TestParseSampleData.parsePressure">
  <configuration name="AttachGatewayApplication" type="SpringBootApplicationConfigurationType"
                 factoryName="Spring Boot" nameIsGenerated="true">
    <module name="attach-gateway" />
    <option name="SPRING_BOOT_MAIN_CLASS" value="com.attach.gateway.AttachGatewayApplication" />
    <method v="2">
      <option name="Make" enabled="true" />
    </method>
  </configuration>

  <configuration name="TestParseSampleData.parsePressure" type="JUnit" factoryName="JUnit"
                 temporary="true" nameIsGenerated="true">
    <module name="access-processor" />
    <extension name="coverage">
      <pattern><option name="PATTERN" value="com.attach.device.access.processor.biz.*" />
               <option name="ENABLED" value="true" /></pattern>
    </extension>
    <option name="PACKAGE_NAME" value="com.attach.device.access.processor.biz" />
    <option name="MAIN_CLASS_NAME" value="com.attach.device.access.processor.biz.TestParseSampleData" />
    <option name="METHOD_NAME" value="parsePressure" />
    <option name="TEST_OBJECT" value="method" />
    <method v="2"><option name="Make" enabled="true" /></method>
  </configuration>
</component>
```

Attributes on `<configuration>`: `name` (display), `type` (factory id), `factoryName` (human label),
`default="true"` (template — skip), `temporary="true"` (IDE-generated — keep but deprioritise),
`nameIsGenerated="true"` (informational), `folderName` (grouping, ignore).

**Option keys by configuration type** (only these matter to us; everything else is ignored):

| Type (`type=`) | main class key | other keys we read |
|---|---|---|
| `SpringBootApplicationConfigurationType` (`factoryName="Spring Boot"`) | `SPRING_BOOT_MAIN_CLASS` | `VM_PARAMETERS`, `PROGRAM_PARAMETERS`, `WORKING_DIRECTORY`, `ALTERNATIVE_JRE_PATH`, `ALTERNATIVE_JRE_PATH_ENABLED`, `ACTIVE_PROFILES`, `ENABLE_LAUNCH_DEBUG` (+ legacy `MODULE_NAME` option, see note below) |
| `Application` (`factoryName="Application"`) | `MAIN_CLASS_NAME` | same as above |
| `JUnit` (`factoryName="JUnit"`) | `MAIN_CLASS_NAME` + `TEST_OBJECT` ∈ {`class`,`method`,`package`,`directory`,`pattern`} + `PACKAGE_NAME`/`CLASS_NAME`/`METHOD_NAME`/`PATTERN` | `VM_PARAMETERS`, `PROGRAM_PARAMETERS`, `WORKING_DIRECTORY`, `TEST_SEARCH_SCOPE` |
| `GradleRunConfiguration` | — | `<ExternalSystem>` subtree; only use it to prove Gradle is in play |
| anything else (`MavenRunConfiguration`, `Remote`, `Compound`, `OpenRewrite…`) | — | ignore |

Structural children (elements, not `<option>`s):

```xml
<module name="attach-gateway" />                          <!-- module name, NOT an option -->
<option name="VM_PARAMETERS" value="-Xmx1g" />
<option name="PROGRAM_PARAMETERS" value="--server.port=8081" />
<option name="WORKING_DIRECTORY" value="$PROJECT_DIR$/attach-gateway" />
<option name="ALTERNATIVE_JRE_PATH" value="17" />          <!-- may be a name OR a path -->
<envs><env name="SPRING_PROFILES_ACTIVE" value="dev" /></envs>
<method v="2"><option name="Make" enabled="true" /></method>   <!-- before-run steps: ignored -->
```

Path macros to expand: `$PROJECT_DIR$`, `$MODULE_WORKING_DIR$` (→ module content root), `$MAVEN_REPOSITORY$`,
`$APPLICATION_CONFIG_DIR$` (unreadable ⇒ treat as unresolvable), `$USER_HOME$`. Also `${…}`-style is *not* used by
JetBrains here.

### 2.2 Lenient JetBrains XML parser spec (hand-rolled; no crates available)

The component is WASM and the crate registry is unreachable in this workspace, so assume **std/core only** and a
hand-rolled scanner. Requirements:

1. Read the file as UTF-8 with lossy replacement (never fail on encoding); strip a leading BOM.
2. Reject if > 2 MiB (`workspace.xml` of `water-server` is 51 KB; 2 MiB is plenty).
3. Locate configuration blocks by scanning, not by a DOM:
   - find the byte index of `<component` whose attribute `name` equals `ProjectRunConfigurationManager` **or**
     `RunManager`; if absent → no configs from this file.
   - from there, repeatedly find `<configuration` and match the balanced `</configuration>`; nested
     `<configuration>` elements do not legally occur but tolerate them with a depth counter.
4. Within a block, a **tag scanner** yields `(name, attrs, self_closing)` in document order. Attribute parser:
   `name = "value"` or `name = 'value'`, whitespace-tolerant around `=`; **decode entities**: `&amp; &lt; &gt; &quot;
   &apos;` and numeric `&#NN;`/`&#xNN;`; keep `<![CDATA[…]]>` text as-is (rare in these files but legal). A `<…>`
   containing `>` inside a quoted attribute must not confuse the scanner — scan attributes respecting quotes.
5. Collect only:
   - the `<configuration>` attributes `name/type/factoryName/default/temporary`;
   - every direct `<option name="…" value="…"/>` child (first occurrence wins per name);
   - every `<module name="…"/>` child (first wins);
   - `<envs><env name=… value=…/></envs>` (all);
   - for `JUnit`, the `TEST_OBJECT`/`PACKAGE_NAME`/`CLASS_NAME`/`METHOD_NAME`/`PATTERN` options.
6. **Ignore silently**: `<extension>`, `<method>`, `<log_file>`, `<patterns>`, `<shortenClasspath>`, unknown options,
   unknown element types. Never error on unknown content. Never re-serialize.
7. Trim leading/trailing ASCII whitespace on every attribute value; `value=""` → empty string (**not** "absent").
8. On malformed XML: abandon that file, log once (diagnostic), continue with other files. Never panic, never fail
   the whole refresh.

Resulting internal model:

```rust
pub struct IdeRunConfig {
    pub source: IdeSource,          // DotRun | IdeaRunConfigurations | WorkspaceRunManager
    pub file: String,
    pub name: String,
    pub cfg_type: Option<String>,   // type= attribute
    pub factory: Option<String>,    // factoryName=
    pub temporary: bool,
    pub module: Option<String>,
    pub main_class: Option<String>, // from SPRING_BOOT_MAIN_CLASS or MAIN_CLASS_NAME
    pub vm: Vec<String>,            // tokenized VM_PARAMETERS
    pub program: Vec<String>,       // tokenized PROGRAM_PARAMETERS
    pub working_dir: Option<String>,
    pub jre_path: Option<String>,
    pub envs: Vec<(String, String)>,
    pub test_object: Option<String>, // JUnit only
}
```

### 2.3 VS Code `launch.json` (JSONC)

Locations: `.vscode/launch.json`, plus workspace-file variants `*.code-workspace` (skip in v1),
`<folder>/.vscode/launch.json` for multi-root workspaces (handle: glob `**/.vscode/launch.json`, cap 8 files).

Shape (official Java extension):

```jsonc
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "java",
      "name": "Water Gateway",
      "request": "launch",
      "mainClass": "com.attach.gateway.AttachGatewayApplication",
      "projectName": "attach-gateway",
      "args": ["--server.port=8081"],       // array of strings; a single string is also tolerated
      "vmArgs": "-Xmx1g -Dspring.profiles.active=dev",  // STRING (space separated) per spec; array tolerated
      "classPaths": ["target/classes"],      // array
      "modulePaths": [],                     // array
      "env": { "SPRING_PROFILES_ACTIVE": "dev" },
      "cwd": "${workspaceFolder}/attach-gateway",
      "console": "internalConsole",          // internalConsole | integratedTerminal | externalTerminal
      "shortenCommandLine": "none",          // none | jarmanifest | @argfile | argfile
      "preLaunchTask": "build"
    }
  ]
}
```

Also seen: `"type": "spring-boot"` (Spring Boot Dashboard extension, uses `mainClass`/`projectName` the same way) — map
it like `type:"java"`.

**JSONC leniency algorithm** (`fn strip_jsonc(src: &str) -> String`), single pass, string-aware:

1. Iterate chars with a state `InString{escape}`; inside a string, copy verbatim (handling `\\`).
2. Outside strings: `//` → skip to newline (keep the newline); `/*` → skip to `*/` (keep line count by emitting
   newlines encountered).
3. Then a **trailing-comma stripper**: outside strings, a `,` whose next non-whitespace char is `}` or `]` is dropped.
4. If a real JSON parser exists in the crate set, feed the cleaned text to it; otherwise hand-roll a minimal
   object/array/string/number/bool/null parser (~150 lines). Booleans must be tolerated unquoted in `default` fields
   that we do not read anyway.
5. On parse failure → skip the file, diagnostic, continue.

**Mapping into our model**:

| launch.json | internal | notes |
|---|---|---|
| `name` | `name` | display only |
| `type` ∈ {`java`,`spring-boot`} | filter | else skip |
| `mainClass` | `main_class` | may contain `$`-less FQN only |
| `projectName` | `module` (heuristic) | Gradle project name or Maven artifactId — resolve to a module dir by matching `artifactId`/dir name |
| `args` (array or string) | `program` tokens | string form is split with our tokenizer |
| `vmArgs` (string or array) | `vm` tokens | string form is split with our tokenizer |
| `classPaths` | `classpath_hint` (used by `PlainMain` CL-3-ish) | relative to `cwd`/workspace folder |
| `env` | `envs` | **cannot be applied** by us — surface as a warning `V_W_ENV_NOT_APPLIED` (§3.0) |
| `cwd` | `working_dir` | expand `${workspaceFolder}`, `${fileDirname}` (file-relative ⇒ unresolvable, warn), `${env:…}` (unresolvable) |
| `console` | ignored | informational |

### 2.4 Precedence and graceful degradation

1. **JetBrains first**: if `.idea/` exists and at least one parseable *non-template* configuration is found with a
   main class → use those as the authoritative "known configurations" (this is the case in `water-server`: 20+
   Spring Boot configs, each with module + main class already curated by the developer).
2. **VS Code second**: if `.vscode/launch.json` exists and JetBrains yielded nothing → use it.
3. **Filesystem heuristics third** (§5): detect modules/mains/wrappers ourselves.
4. **Nothing** → contribute zero configurations, and emit **one** informational notification per project per session:
   *"JVM defaults: no Java launch configuration found in this project (looked at .run/*.run.xml,
   .idea/runConfigurations, .idea/workspace.xml, .vscode/launch.json, pom.xml, build.gradle)."*
   Never a modal, never an error.

Dedup rule when both sources describe the same app: key = `(main_class, module_relpath)`; JetBrains wins; the VS Code
one is used only to fill gaps (`vmArgs` if JetBrains had none, `env` never).

If parsing partially fails (one file bad, others fine) → continue, one aggregated diagnostic, and (important) **do not
let inheritance change ids**: ids never depend on IDE files (§5.3).

### 2.5 Free win: import the real IntelliJ *JvmParameter* settings file

`water-server` contains `.idea/jvm-parameter-settings.xml`, written by the actual IntelliJ plugin this project mimics:

```xml
<project version="4">
  <component name="JvmParameterSetting">
    <option name="jvmParameterList">
      <list>
        <JvmParameter>
          <option name="enabled" value="true" />
          <option name="global"  value="false" />
          <option name="name"  value="spring.cloud.nacos.config.namespace" />
          <option name="value" value="new_dev_local" />
        </JvmParameter>
      </list>
    </option>
  </component>
</project>
```

Setting `importJvmParameterFile = true` (default **false**, opt-in) converts it to
`-D<name>=<value>` tokens, in list order, for entries with `enabled="true"`; `global="true"` entries become
`-D<name>=<value>` too (the distinction is the IntelliJ plugin's, we flatten it). This is a strong migration story
(users of the IntelliJ plugin keep working) at the cost of one file read.
Note: `global="true"` in the original plugin means "application-level" — our flattening is a deliberate
simplification; document it in the UI text.

---

## 3. Injection and merge semantics

### 3.0 Where injection is physically possible (and the env-var story)

The `run-configurations` record is `{ id, name, program, args, cwd, test, framework }`: **plain argv, no shell, no env,
no JVM/IDE config object.** Consequences, in order of importance:

1. **`JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, `_JAVA_OPTIONS` cannot be used through contributed run configurations.**
   There is no env field, and we must not assume the host lets a config carry env. (Even if it did: all three print a
   `Picked up …` banner to stderr — `JAVA_TOOL_OPTIONS` and `_JAVA_OPTIONS` — which pollutes app logs and breaks tools
   that parse stderr; `JDK_JAVA_OPTIONS` requires JDK 9+ and is *not* read by `mvn`/`gradle` themselves.)
2. **The same three variables, plus `MAVEN_OPTS`/`GRADLE_OPTS`, *are* usable from side paths** that take an explicit
   env: `host::start(Command { env })` and `open_terminal_session`-style APIs (see `.research/api-host.md` for exact
   signatures). Caveats: (a) only processes *we* start inherit them, so the IDE's own Run buttons are unaffected —
   that is the whole point of the original plugin and it is out of reach; (b) the variables are inherited by every
   child in the tree, including the build tool's JVM, which is exactly the "wrong JVM" trap
   (`MAVEN_OPTS` does affect `spring-boot:run` **only** when it does not fork); (c) they leak into subprocesses the app
   itself starts (Maven/Gradle/a second JVM), which can double-apply options; (d) banner noise (above).
   **Decision: do not use env vars in v1.** Document them in the README as "not supported, use the config strategies".
3. **File-based injection does work for the *native* IDE run configs we cannot mutate** — see Mode D below. This is
   the closest honest analogue of the IntelliJ plugin's behaviour.

So the plugin offers four injection **modes**, each a distinct surface:

| Mode | Surface | Works for | Risk |
|---|---|---|---|
| **A** (default) | contribute our own `run-configurations` | launches started from our configs | none; user must pick our config |
| **B** | copy of the existing IDE config, emitted by us (inherited values + our defaults) | same as A, but recognisable names | none |
| **C** (opt-in) | **materialize `.run/*.run.xml`** files the IDE's own runner reads | the IDE's Run popup, on project (re)load | writes project files; IDE may cache (Q5) |
| **D** (opt-in, dangerous) | patch `.mvn/jvm.config` | *any* Maven-launched JVM (incl. the IDE's Maven run configs), and `exec:java`/non-forked runs | mutates a shared, often-committed file; affects builds; not revertible if the user edits it |

Mode C writer spec (`.run/<safe-name> (JVM defaults).run.xml`, never touching user files):

```xml
<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="attach-gateway · JVM defaults (Spring Boot)"
                 type="SpringBootApplicationConfigurationType" factoryName="Spring Boot">
    <module name="attach-gateway" />
    <option name="SPRING_BOOT_MAIN_CLASS" value="com.attach.gateway.AttachGatewayApplication" />
    <option name="VM_PARAMETERS" value="-Xmx1g -Dspring.profiles.active=dev" />
    <option name="PROGRAM_PARAMETERS" value="--server.port=8081" />
    <option name="WORKING_DIRECTORY" value="$PROJECT_DIR$/attach-gateway" />
    <method v="2"><option name="Make" enabled="true" /></method>
  </configuration>
</component>
```

Ownership/revert: keep a storage record `jvm-defaults/runxml/<id>` → `{relative_path, sha256_written}`. On refresh or
on `writeRunXml: "off"`, delete files **only** when the on-disk hash still equals what we wrote (the user may have
edited them — then leave them alone and forget them). File names must be stable (derived from the config id slug) so
repeated runs do not create duplicates.

Mode D writer spec (`.mvn/jvm.config`, managed block):

```text
-Dfile.encoding=UTF-8
# >>> jvm-defaults (managed, do not edit inside this block) >>>
-Xmx1g
-Dspring.profiles.active=dev
# <<< jvm-defaults <<<
```

Same ownership rule: only rewrite/remove the block between markers; refuse (warn) if the markers are unbalanced.
Caveats to state in the UI: applies to **every** Maven invocation in that project (builds too — `-Xmx` on a build JVM
can make builds fail); does not reach a *forked* app (`spring-boot:run` with `fork=true`), so it only helps
`exec:java`/non-forked runs; `.mvn/jvm.config` is frequently committed. **Not enabled by default.**

### 3.1 Tokenization (the one primitive everything depends on)

`fn tokenize(text: &str) -> Result<Vec<String>, TokenizeError>` — used for settings text (`vmOptions`, `programArgs`)
**and** for inherited `VM_PARAMETERS`/`PROGRAM_PARAMETERS`/`vmArgs` strings.

State machine (`Outside | InDouble | InSingle`), iterating `char`s:

- `Outside`: ASCII whitespace and `\n`/`\r` and `\t` ⇒ separator. `"` ⇒ enter `InDouble`. `'` ⇒ enter `InSingle`
  (only if a quote starts a token; a `'` in the middle of a token is literal). `#` at the **start of a line (after
  trimming leading whitespace)** ⇒ comment to end of line (settings text only). `\` ⇒ literal (Windows paths!).
- `InDouble`: `\"` ⇒ `"` (the only escape honoured). Every other `\` stays literal — critical so
  `-Dpath="C:\Program Files\x"` does not become `C:Program Filesx`. `"` ⇒ `Outside`.
- `InSingle`: `'` ⇒ `Outside`; everything else literal (including `\`).
- EOF while `InDouble`/`InSingle` ⇒ `TokenizeError::UnbalancedQuote(pos)` (`V_E_UNBALANCED`).
- Quotes are **removed** from the token (they only group). `-Dmsg="a b"` ⇒ one token `-Dmsg=a b`.
- Limits: ≤ 512 tokens, each ≤ 8 KiB, total input ≤ 64 KiB; over ⇒ `TokenizeError::TooLarge`.
- A token that becomes empty (`""`) is dropped.
- Result is **already shell-free argv**: no quote characters survive, so the merge/serialize stages never need to
  re-parse.

Round-trip guarantee for the preview: `tokenize(join_for_preview(tokens)) == tokens` must hold for every token without
whitespace/quote/`#`/newline; enforced by a property test (§8).

### 3.2 Layer model and precedence

Layers, **lowest → highest**:

| # | Layer | Source | Notes |
|---|---|---|---|
| L0 | `preset` | `profiles[activeProfile]` (§4) | blank when `activeProfile` is `""` |
| L1 | `global` | global-scope `vmOptions` / `programArgs` | |
| L2 | `project` | project-scope `vmOptions` / `programArgs` | overrides global **per whole field**, not per token — write a note in the UI ("project value replaces the global one; use presets/extras to compose") |
| L3 | `imported` | `.idea/jvm-parameter-settings.xml` when `importJvmParameterFile` | `-Dk=v` tokens |
| L4 | `inherited` (per-configuration) | the IDE config we cloned: its `VM_PARAMETERS` / `PROGRAM_PARAMETERS` | only when `inheritIdeConfigs`; matched by `(main_class, module)` |
| L5 | `perConfig` | `configOverrides["<id>"].vmOptions/programArgs` | explicit user override for one generated config |
| L6 | `extra` | `extraVmOptions` (string) | appended to every config, last |

Ordering rationale: **user's existing per-configuration values must beat our defaults**, so `L4` sits above `L1/L2`.
Because the JVM applies *later* duplicates (`-Xmx1g -Xmx2g` ⇒ 2g), emitting layers in ascending order with last-wins
dedupe gives exactly that precedence without any rewrite.

`mergeMode`:

- `"merge"` (default): all layers, in order, dedupe (§3.3) across layers with **last occurrence wins**.
- `"override"`: `L4` (inherited) and `L6` are dropped; `L5` still applies. (Use when the plugin's values must be
  authoritative, e.g. forced `-Dspring.profiles.active`.)

`injectionMode` per layer is *not* needed as a separate setting: "append vs replace" is expressed by `mergeMode` +
`extraVmOptions` + `configOverrides`. Document that in the UI text; do not add a third mode.

Program arguments merge with the same layering (L1/L2/L4/L5), same dedupe rules, same last-wins.

### 3.3 Dedupe rules (`dedupe = true`, default)

Dedupe happens **within the merged list**, keeping the **last** occurrence of a key (so the highest layer survives) and
preserving the position of that last occurrence. Keys:

| Token shape | Key | Example |
|---|---|---|
| `-Dk=v`, `-Dk` | `prop:<k>` (case-sensitive; `-Dk` and `-Dk=v` are the same key) | `-Dserver.port=1` vs `-Dserver.port=2` ⇒ 2 |
| `-Xmx…`, `-Xms…`, `-Xss…`, `-Xmn…` | `x:mx`, `x:ms`, `x:ss`, `x:mn` | last wins |
| `-XX:+Name`, `-XX:-Name`, `-XX:Name=v` | `xx:<Name>` | the `+`/`-`/`=v` is the *value*, so `-XX:+UseG1GC` then `-XX:-UseG1GC` ⇒ the `-` wins |
| `-javaagent:<path>[=opts]` | `agent:<normalised path>` (lowercased on Windows, `\`→`/`) | last wins per agent jar |
| `-agentlib:jdwp=…`, `-agentpath:…` | `agentlib:jdwp` / `agentpath:<path>` | last wins |
| `-cp`, `-classpath`, `--class-path` | `cp` | **forbidden** in VM options by validation anyway |
| `--add-opens`, `--add-exports`, `--add-reads`, `--add-modules` | exact token (no keying — repeats are meaningful) | exact-duplicate removal only |
| `--enable-preview`, `--enable-native-access=…` | exact token | |
| anything else | the whole token | exact-duplicate removal only |

Rules:

- Never reorder tokens *within* a layer (users order `-javaagent` and `-XX:` deliberately).
- Removing a duplicate must not orphan a `-Dk` (i.e. you cannot remove `-Dk=v` because `-Dk` came later — treat them
  as the same key and keep the later one; JVM would keep the later one too).
- `dedupe = false` ⇒ plain concatenation (useful to prove exactly what is passed).
- Reporting: when a token is dropped, produce a `MergeNote { dropped, kept, reason: LayerShadowed{from_layer,to_layer} }`
  list shown in the preview (§3.4). This is what makes the plugin trustworthy versus "it silently ate my `-Xmx`".

### 3.4 Serialization and preview

#### 3.4.1 Serializing for `args` (always the truth)

- VM tokens ⇒ **one argv element each**, e.g. `["-Xmx1g", "-Dspring.profiles.active=dev", "-jar", "app.jar"]`.
- Program tokens ⇒ one element each, **after** the strategy's fixed prefix (`-jar <jar>` / `-cp <cp> <Main>` / the
  goal line).
- Never join tokens into a string for `args` — except inside a strategy property that *requires* a string
  (`-Dspring-boot.run.jvmArguments=…`, `-Dspring-boot.run.arguments=…`, `-Dexec.args=…`, `--args=…`), where the join is
  `join_tokens(tokens) = tokens.join(" ")` and a token containing whitespace is a validation error
  (`V_E_SPACE_IN_SERIALIZED`) for those strategies.

#### 3.4.2 Rendering a preview (display only)

Two renderers, both pure functions over `LaunchPlan`:

```rust
pub fn render_posix(p: &LaunchPlan) -> String   // sh_quote every token, join with " "
pub fn render_cmd(p: &LaunchPlan)   -> String   // cmd_quote every token, join with " "
```

`render_posix` example (same plan as §6):

```sh
cd '/d/workspace/water/water-server/attach-gateway/target' && \
'/usr/bin/java' '-Xmx1g' '-Dspring.profiles.active=dev' -jar 'attach-gateway.jar' '--server.port=8081'
```

`render_cmd` (what the user pastes into `cmd`):

```bat
cd /d D:\workspace\water\water-server\attach-gateway\target
java.exe -Xmx1g -Dspring.profiles.active=dev -jar attach-gateway.jar --server.port=8081
```

(Tokens without spaces/metachars are emitted unquoted for readability; quoting is applied by `cmd_quote` only when
needed. The renderer is strictly a display artifact — never `tokenize(render)` it.)

Preview surfaces:

- **Notification** on demand (command `jvm-defaults.preview`): first 3 lines + "…" — `notify(Info, …)` is short.
- **Multi-line dialog** (`input_multiline` or an output channel, whichever the host offers — see api-host): full
  argv, one line per element, plus the dedupe notes and the "which JVM/app sees what" split:

  ```text
  program : D:\workspace\water\water-server\mvnw.cmd    (shell: cmd.exe)
  cwd     : D:\workspace\water\water-server
  argv[0] : -B
  argv[1] : -pl
  argv[2] : attach-gateway
  ...
  VM options the JVM will see : -Xmx1g -Dspring.profiles.active=dev
  App args                    : --server.port=8081
  dropped                     : -Xmx512m (shadowed by project "-Xmx1g", layer L1→L2)
  ```

- `printPreviewOnRefresh` (default false): after each `configurations()` call, write the whole preview of all emitted
  plans to the output/diagnostic channel. Useful while bootstrapping; off by default (noise).
- `previewOnly` (dry run, default false): **contribute zero run configurations**; instead register the preview command
  and emit one notification listing the plans that *would* be emitted. This is the safe "see what it does before it
  does it" toggle; it must be visible in the UI text of the settings page.

### 3.5 Validation

Validation runs on the tokenized settings fields (per field) and on each assembled plan; results are
`Vec<Diag { severity, code, field, token: Option<String>, message: String }>`. `Error` blocks the config from being
emitted (and shows once in a notification with the field name); `Warning` is shown in the preview only.

| Code | Sev | Rule |
|---|---|---|
| `V_E_CP` | err | VM options contain `-cp`, `-classpath`, `--class-path`, `-p`, `--module-path`, `-jar`, `-m`, `--module`, `-Xbootclasspath`, `@argfile` — classpath/jar/main selection belongs to the strategy, not the user's VM field |
| `V_E_UNBALANCED` | err | `tokenize` hit an unbalanced `"`/`'` (report char offset) |
| `V_E_NEWLINE_IN_QUOTE` | err | a quoted span crosses a `\n` — almost always a typo; reject instead of guessing |
| `V_E_TOKEN_NO_DASH` | err | a VM-options token does not start with `-` (e.g. a main class pasted by mistake) |
| `V_E_SPACE_IN_SERIALIZED` | err | strategy serializes into a property AND a token contains whitespace (§1.1) |
| `V_E_QUOTE_IN_SHELL_MODE` | err | token contains `"` and mode is `cmd` (§1.0.2) |
| `V_E_PERCENT_IN_SHELL_MODE` | err | token contains `%` and mode is `cmd` |
| `V_W_DUP_XMX` | warn | the same key (`x:mx`, `x:ms`, `prop:k`) appears **within one field** — later wins, probably not intended |
| `V_W_D_VS_APPARG` | warn | a **program-arguments** token starts with `-D`/`-X`/`-XX:` — it looks like a VM option; offer "move to VM options" |
| `V_W_D_NO_EQ` | warn | `-Dkey` without `=`: legal (sets an empty property) but usually a typo |
| `V_W_AGENT_NO_JDWP` | warn | `-agentlib`/`-javaagent` present without an address/`jdwp` — debug will not attach |
| `V_W_ARGline_CLOBBERS` | warn | JUnit strategy + pom mentions `jacoco` (§1.6) |
| `V_W_ENV_NOT_APPLIED` | warn | inherited `.vscode` config has `env` — we cannot pass env (§3.0) |
| `V_W_CP_SEPARATOR` | warn | a classpath element contains the platform path separator |
| `V_W_PROFILE_TWICE` | warn | `activeProfile` preset also sets `-Dspring.profiles.active` explicitly elsewhere |

Hard limits enforced before anything is emitted: ≤ 64 KiB of settings text, ≤ 512 tokens, ≤ 16 KiB per argv element,
≤ 100 argv elements per plan, ≤ 64 configurations (settings `maxConfigurations`, default 20). `program` ≤ 32 KiB
(Windows command-line limit is 32767 chars total — warn at > 30000).

---

## 4. Settings model

All keys are declared once in the JSON schema; scoping is the host's project-over-global (§2 of the brief). Where the
table says `both`, the project value wins wholesale. Where it says `global`, the key is ignored at project scope
(host behaviour permitting — **[UNCONFIRMED]** whether the host can express "global only"; if not, treat them as
`both` and reconcile in code by always reading the global value).

| Key | Type | Default | Scope | Consumed by |
|---|---|---|---|---|
| `enabled` | bool | `true` | global | master switch; `false` ⇒ emit nothing |
| `vmOptions` | multiline string | `""` | both | L1/L2 |
| `programArgs` | multiline string | `""` | both | L1/L2 |
| `activeProfile` | string | `""` | both | picks `profiles[...]`; also exposed as a quick pick |
| `profiles` | object → `{vmOptions, programArgs}` | `{"dev":{"vmOptions":"-Dspring.profiles.active=dev"},"test":{"vmOptions":"-Dspring.profiles.active=test"},"prod":{"vmOptions":"-Dspring.profiles.active=prod"}}` | both (map-merged, project keys replace global keys) | L0 |
| `mergeMode` | enum `merge\|override` | `merge` | both | §3.2 |
| `dedupe` | bool | `true` | both | §3.3 |
| `strategies` | object of bools: `bootJar,springBootMaven,springBootGradle,mavenExec,plainMain,junit` | `{"bootJar":true,"springBootMaven":true,"springBootGradle":true,"mavenExec":false,"plainMain":false,"junit":true}` | both | §1.7 |
| `preferStrategy` | enum `auto\|bootJar\|springBootMaven\|springBootGradle\|mavenExec\|plainMain` | `auto` | both | ordering/only |
| `emitAllStrategies` | bool | `true` | both | one config per applicable strategy vs only preferred |
| `maxConfigurations` | number | `20` (0–64) | global | total cap |
| `inheritIdeConfigs` | bool | `true` | both | L4, §2 |
| `importJvmParameterFile` | bool | `false` | both | §2.5 → L3 |
| `writeRunXml` | enum `off\|dotRun\|runConfigurations` | `off` | both | §3.0 Mode C |
| `previewOnly` | bool | `false` | both | §3.4.2 dry run |
| `printPreviewOnRefresh` | bool | `false` | global | diagnostic |
| `javaExecutable` | string | `""` | both | absolute path to `java`; empty ⇒ `"java"` (Q1) |
| `jrePath` | string | `""` | both | `ALTERNATIVE_JRE_PATH` inheritance only (we cannot set a JRE for our own plans other than via `javaExecutable`) |
| `cwdMode` | enum `auto\|moduleDir\|projectRoot\|jarDir\|custom` | `auto` | both | cwd table below |
| `customCwd` | string | `""` | both | |
| `extraVmOptions` | string | `""` | both | L6 |
| `configOverrides` | **JSON string** | `"{}"` | both | L5, e.g. `{"jvm-defaults:bootjar:attach-gateway":{"vmOptions":"-Xmx2g"}}` |
| `buildDeps` | bool | `true` | both | `-am` for Maven strategies |
| `offline` | bool | `false` | both | `-o` for Maven strategies |
| `jarSearchDirs` | string | `""` | both | extra jar dirs, newline-separated, relative to project root (e.g. `build/jars`) |
| `spawnMode` | enum `auto\|direct\|cmd` | `auto` | both | §1.0.2 **[UNCONFIRMED]** default resolution |

`cwdMode = auto` resolution:

| Strategy | auto cwd |
|---|---|
| `BootJar` | the jar's parent directory (`…/target` or `…/build/libs`) |
| `SpringBootMaven` / `MavenExec` / `JUnit` | the multi-module root that owns the wrapper |
| `SpringBootGradle` | the Gradle build root (dir with `gradlew`) |
| `PlainMain` | module dir |
| `custom` | `customCwd`, expanded (`${projectRoot}`, `${moduleDir}`, `${jarDir}` supported) |

### 4.1 JSON schema (`settings/jvm-defaults.schema.json`)

**[UNCONFIRMED]** the exact dialect/widgets the host supports (`[[contributes.settings]] schema = path`). The file
below is draft-07-compatible and intentionally conservative (only `boolean`, `string` with `multiline`, `number`,
`enum`, nested `object` with `additionalProperties`). `multiline` is expressed as `"format": "multiline"` — verify
against `.research/api-host.md`; if the host has no such hint, use `"title"` text that says "one or more per line".

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "JVM Default Parameters",
  "description": "Injects VM options and program arguments into generated Java launch configurations.",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "enabled": { "type": "boolean", "default": true, "title": "Enable JVM defaults" },
    "vmOptions": {
      "type": "string", "default": "", "title": "VM options", "format": "multiline",
      "description": "One or more per line, e.g. -Xmx1g -Dspring.profiles.active=dev. Comments start with #."
    },
    "programArgs": {
      "type": "string", "default": "", "title": "Program arguments", "format": "multiline",
      "description": "Application arguments (after the main class / -jar), e.g. --server.port=8081."
    },
    "activeProfile": { "type": "string", "default": "", "title": "Active profile preset",
                       "enum": ["", "dev", "test", "prod"] },
    "profiles": {
      "type": "object", "title": "Profile presets",
      "additionalProperties": {
        "type": "object", "additionalProperties": false,
        "properties": {
          "vmOptions":    { "type": "string", "default": "", "format": "multiline" },
          "programArgs":  { "type": "string", "default": "", "format": "multiline" }
        }
      },
      "default": {
        "dev":  { "vmOptions": "-Dspring.profiles.active=dev",  "programArgs": "" },
        "test": { "vmOptions": "-Dspring.profiles.active=test", "programArgs": "" },
        "prod": { "vmOptions": "-Dspring.profiles.active=prod", "programArgs": "" }
      }
    },
    "mergeMode": { "type": "string", "default": "merge", "enum": ["merge", "override"],
                   "title": "Merge mode",
                   "description": "merge: our defaults are added before the configuration's own values (its values win). override: only plugin values are used." },
    "dedupe": { "type": "boolean", "default": true, "title": "Deduplicate repeated VM options",
                "description": "The last occurrence of -Dkey, -Xmx, -XX:… wins; dropped ones are shown in the preview." },
    "strategies": {
      "type": "object", "title": "Launch strategies", "additionalProperties": false,
      "properties": {
        "bootJar":           { "type": "boolean", "default": true,  "title": "Runnable / boot JAR (java -jar)" },
        "springBootMaven":   { "type": "boolean", "default": true,  "title": "Spring Boot via Maven wrapper (spring-boot:run)" },
        "springBootGradle":  { "type": "boolean", "default": true,  "title": "Spring Boot via Gradle wrapper (bootRun)" },
        "mavenExec":         { "type": "boolean", "default": false, "title": "Maven exec (exec:exec)" },
        "plainMain":         { "type": "boolean", "default": false, "title": "Main class + derived classpath" },
        "junit":             { "type": "boolean", "default": true,  "title": "JUnit test configurations" }
      },
      "default": { "bootJar": true, "springBootMaven": true, "springBootGradle": true,
                   "mavenExec": false, "plainMain": false, "junit": true }
    },
    "preferStrategy": { "type": "string", "default": "auto",
      "enum": ["auto", "bootJar", "springBootMaven", "springBootGradle", "mavenExec", "plainMain"],
      "title": "Preferred strategy" },
    "emitAllStrategies": { "type": "boolean", "default": true, "title": "Emit one configuration per applicable strategy" },
    "maxConfigurations": { "type": "number", "default": 20, "minimum": 0, "maximum": 64,
                           "title": "Maximum number of contributed configurations" },
    "inheritIdeConfigs": { "type": "boolean", "default": true,
      "title": "Inherit existing IDE run configurations",
      "description": "Reads .run/*.run.xml, .idea/runConfigurations/*.xml, .idea/workspace.xml, .vscode/launch.json." },
    "importJvmParameterFile": { "type": "boolean", "default": false,
      "title": "Import .idea/jvm-parameter-settings.xml",
      "description": "Migrates settings written by the IntelliJ JVM Parameter plugin." },
    "writeRunXml": { "type": "string", "default": "off", "enum": ["off", "dotRun", "runConfigurations"],
                     "title": "Also write .run/*.run.xml files for the IDE's own runner" },
    "previewOnly": { "type": "boolean", "default": false,
                     "title": "Dry run (preview only, contribute nothing)" },
    "printPreviewOnRefresh": { "type": "boolean", "default": false, "title": "Print the command preview on every refresh" },
    "javaExecutable": { "type": "string", "default": "", "title": "java executable",
                        "description": "Absolute path to java/java.exe. Empty = resolve 'java' from PATH." },
    "jrePath": { "type": "string", "default": "", "title": "JRE path (inheritance only)" },
    "cwdMode": { "type": "string", "default": "auto", "enum": ["auto", "moduleDir", "projectRoot", "jarDir", "custom"],
                 "title": "Working directory" },
    "customCwd": { "type": "string", "default": "", "title": "Custom working directory",
                   "description": "Supports ${projectRoot}, ${moduleDir}, ${jarDir}." },
    "extraVmOptions": { "type": "string", "default": "", "format": "multiline",
                        "title": "Extra VM options (appended to every configuration, highest priority)" },
    "configOverrides": { "type": "string", "default": "{}",
      "title": "Per-configuration overrides (JSON)",
      "description": "{\"<configuration id>\": {\"vmOptions\": \"-Xmx2g\", \"programArgs\": \"\"}}" },
    "buildDeps": { "type": "boolean", "default": true, "title": "Build required modules (-am)" },
    "offline": { "type": "boolean", "default": false, "title": "Maven offline (-o)" },
    "jarSearchDirs": { "type": "string", "default": "", "format": "multiline",
                       "title": "Additional JAR directories (one per line, relative to the project root)" },
    "spawnMode": { "type": "string", "default": "auto", "enum": ["auto", "direct", "cmd"],
                   "title": "Wrapper spawn mode",
                   "description": "auto: wrap .cmd/.bat in cmd.exe. direct: pass the wrapper path verbatim. cmd: always wrap." }
  }
}
```

Manifest side (**[UNCONFIRMED]**, api-run owner pins the exact TOML):

```toml
[[contributes.settings]]
schema = "settings/jvm-defaults.schema.json"
```

### 4.2 Validation of setting *values* (beyond per-field token validation)

- `activeProfile` not in `profiles` ⇒ warning, treat as empty.
- `configOverrides` unparseable ⇒ error diag on that key, ignore overrides (never fail the refresh).
- `javaExecutable` non-empty but missing/not a file (`stat`) ⇒ error diag, fall back to `"java"`.
- `customCwd` with an unknown `${…}` macro ⇒ error diag on that plan.
- `maxConfigurations = 0` ⇒ behaves like `previewOnly` (nothing emitted) — allowed, documented.
- `strategies` all false ⇒ nothing emitted + one notification pointing at the settings.

---

## 5. Detection and stable ids

### 5.1 Marker rules

Cheap probes first (`exists`/`stat`), content reads only when needed. Normalise all paths to `/`.

**Project kinds**

| Kind | Markers (all relative to project root) | Evidence to read |
|---|---|---|
| `maven` | `pom.xml` | `<modules>`, `<artifactId>`, `spring-boot-*` strings |
| `maven-wrapper` | `mvnw.cmd`, `mvnw`, `.mvn/wrapper/maven-wrapper.properties` | presence only |
| `gradle` | `settings.gradle`, `settings.gradle.kts`, `build.gradle`, `build.gradle.kts` | `org.springframework.boot`, `spring-boot-gradle-plugin`, `bootJar` |
| `gradle-wrapper` | `gradlew.bat`, `gradlew` | presence |
| `spring-boot` | any pom/build file mentioning `spring-boot`; or a `@SpringBootApplication` main class; or `.idea` config with `SpringBootApplicationConfigurationType` | |
| `boot-jar` | `*/target/*.jar`, `build/libs/*.jar`, plus `jarSearchDirs` | filename filter (§1.3) |
| `junit-configs` | IDE configs with `type="JUnit"` | class/method/module |

**Module enumeration (Maven, multi-module)** — the shape of both sample projects:

1. read root `pom.xml`; if `<modules>` present, each `<module>text</module>` is a relative dir
   (may be nested: `attach-device/device-server`), in **document order**;
2. if a module's own `pom.xml` has `<modules>`, recurse (depth ≤ 3, total ≤ 64 modules);
3. a module is an *application* if (a) it contains a `*Application.java` under `src/main/java` **or** (b) an IDE
   config names it with a main class. Both sample projects use `*Application.java`; `water-server` also has 20+
   curated IDE configs — use (b) as the primary source when available, because the user already decided what is
   runnable.
4. `spring-boot` evidence for a module: module pom **or** root pom contains `spring-boot-maven-plugin`,
   `spring-boot-starter-parent`, or `spring-boot-dependencies`. (In `water-server` the plugin appears in
   `attach-gateway/pom.xml` **only inside `<profiles>`** — so a plain "is the plugin in `<build><plugins>`" check
   would miss it. Substring search on the whole pom text is the correct, deliberately dumb heuristic.)
5. module dir = directory of the pom, relative to the project root, forward slashes (`attach-gateway`,
   `attach-device/device-server`).

**Main class discovery** (only when IDE configs did not give one):

```text
find_files("**/src/main/java/**/*Application.java", 16)      # glob dialect UNCONFIRMED
find_files("**/*Application.java", 16)
```

then read the file (≤ 64 KiB) and require `@SpringBootApplication` (or `public static void main`) in the text; derive
the FQN from the `package …;` line + file stem. Fallback: a main class from `.idea` configs is authoritative.

**Wrapper root discovery** (for `-pl <module>`): walk **up** from the module dir to the project root, pick the
shallowest ancestor dir that contains both `pom.xml` with `<modules>` and `mvnw.cmd`/`mvnw`. In `water-server` that is
the project root. If the project root itself is a single-module project, wrapper root = project root and `-pl`/`-am`
are omitted.

### 5.2 Deterministic ordering

Total order for emitted configurations (also the `maxConfigurations` truncation order):

1. modules by relpath, `strcmp` (byte order, case-sensitive) — *not* by mtime, not "detected order";
2. within a module: `preferStrategy` first, then the fixed strategy priority
   `BootJar(0) < SpringBootMaven(1) < SpringBootGradle(2) < MavenExec(3) < PlainMain(4) < JUnit(5)`;
3. within one strategy with several candidates (e.g. two jars): jar name `strcmp`;
4. JUnit configs last, ordered by `(module, class, method)`.

Rationale: a stable order means the Run popup order does not shuffle between refreshes, and truncation is
deterministic.

### 5.3 Stable id scheme

```text
id = "jvm-defaults:" + strategy_tag + ":" + slug(module_relpath) [ + "." + slug(jar_file_stem) ]
```

| Strategy | `strategy_tag` |
|---|---|
| `BootJar` | `bootjar` |
| `SpringBootMaven` | `spring-boot-maven` |
| `SpringBootGradle` | `spring-boot-gradle` |
| `MavenExec` | `maven-exec` |
| `PlainMain` | `main` |
| `JUnit` | `junit` |

Examples: `jvm-defaults:bootjar:attach-gateway`, `jvm-defaults:spring-boot-maven:attach-gateway`,
`jvm-defaults:junit:access-processor.TestParseSampleData.parsePressure`.

`slug(s)`:

1. lowercase ASCII;
2. `[^a-z0-9._/-]` → `-`; then `/` → `.`, collapse runs of `-`, trim `.-` from both ends;
3. if the result is empty → `root`;
4. if `len > 48`: `&s[..40]` + `-` + 6 lowercase hex of `fnv1a32(original)`;
5. charset is `[a-z0-9._-]` only — no spaces, no `:`, no `/`, so it is safe for any host that treats ids as opaque
   strings or as DOM-ish keys.

**Collisions**: append `-2`, `-3`, … in the deterministic order of §5.2 to the **later** ids. (With the strategy tag plus
module path this is nearly impossible; it can only happen with pathological module names that slug identically, e.g.
`a-b` vs `a_b`.) Collision resolution must be **stable**, therefore it is computed from the full detected list every
refresh — never from a counter in storage.

**Stability when files change**

- id depends **only** on `(strategy_tag, module_relpath, jar_file_stem_for_bootjar)` — never on mtimes, config names,
  counts, or ordering.
- Version bumps: `attach-gateway-1.0.0.jar` → `attach-gateway-1.1.0.jar` would change a `jar_file_stem`-based id, so for
  `BootJar` we **do not** put the version in the id: use the **module slug only**, and remember the concrete jar path in
  `host::storage["jvm-defaults/pick/<id>"]`. Change the jar's *name* (not just version) and the id still resolves
  through the remembered pick until the file disappears.
- `attach-gateway.jar` (no version, this repo's `finalName`) ⇒ nothing to do.
- Renames (`attach-gateway` → `gateway`): id changes (unavoidable). Mitigation: `jvm-defaults/idmap` in storage keeps
  `old_id → new_id` so per-configuration overrides (`configOverrides`) and the IDE's "recently used" feeling survive;
  when a new id appears and has no override, look it up by `(main_class, module)` in the idmap and re-attach the
  override. Bounded to 128 entries, LRU.
- **Never** derive ids from the discovered IDE config *name* (`AttachGatewayApplication`) — names are user-editable and
  duplicated in `workspace.xml` (`(1)`, `(2)` suffixes).

**Display name** (separate from id, human-facing):

```text
"<module-display> · <strategy-label>"                 e.g. "attach-gateway · JAR"
"<module-display> · <strategy-label> (<main-simple>)" e.g. "attach-gateway · Spring Boot (Maven)"
"<class>#<method> · test"                             e.g. "TestParseSampleData#parsePressure · test"
```

Prefix nothing with "JVM defaults" in the name (it makes the Run popup unreadable); the id namespace already
identifies us. Optionally suffix `· dev` when a profile preset is active (helps disambiguate three otherwise identical
entries) — controlled by `activeProfile`.

### 5.4 Refresh budget and caching

`configurations(root)` may run often (§ api-run: refresh triggers). Never do I/O proportional to repo size.

| Phase | Allowed I/O (default) |
|---|---|
| settings load | 1 host call |
| IDE config scan | ≤ 4 `exists` + ≤ 8 file reads (`*.run.xml`, `runConfigurations/*.xml`, `workspace.xml`, `launch.json`) |
| module enumeration | 1 glob or 1 root-pom read + ≤ 32 module pom reads, each **prefix-limited** (read ≤ 16 KiB, substring search) |
| main class scan | ≤ 16 file reads of ≤ 64 KiB, only if IDE configs gave no main class |
| jar discovery | `find_files` per module (≤ 64 results) or targeted `stat` of `target/*.jar`; no reads |
| total wall budget | aim < 150 ms warm, < 800 ms cold |

**Cache**: `host::storage["jvm-defaults/detect/<project_hash>"]` → `{ fingerprint, detected_at_unix, modules: [...], ide: [...] }`
where `fingerprint = fnv1a32(settings_json || root_pom_mtime_size || gradle_mtime_size || per-marker (path,mtime,size) for ≤ 16 markers)`.
On refresh: recompute the fingerprint cheaply; if equal, reuse the cached detection (no file reads at all). Invalidate
on any settings change. Never cache in memory only — the component may be re-instantiated per call. **[UNCONFIRMED]**
whether `host::storage` survives that, and whether it is per-project.

---

## 6. Worked example

Real project: `D:\workspace\water\water-server` (Maven multi-module, Spring Boot 3.5.11, Java 21,
`finalName = ${project.artifactId}` in a profile ⇒ `attach-gateway.jar` without a version).

Facts used (all verified on disk):

- root `pom.xml` `<modules>` includes `attach-gateway`; wrapper `mvnw.cmd` + `mvnw` + `.mvn/jvm.config`
  (`-Dfile.encoding=UTF-8`);
- `.idea/workspace.xml` has
  `<configuration name="AttachGatewayApplication" type="SpringBootApplicationConfigurationType" factoryName="Spring Boot">`
  with `<module name="attach-gateway"/>` and `SPRING_BOOT_MAIN_CLASS = com.attach.gateway.AttachGatewayApplication`;
- `attach-gateway/src/main/java/com/attach/gateway/AttachGatewayApplication.java` exists;
- `attach-gateway/target/classes` exists but **`attach-gateway/target/*.jar` does not** (no package build has run);
  the runnable artifact present on this machine is the deploy copy `build/jars/attach-gateway.jar` (298 KB) ⇒ this is
  exactly the case for `jarSearchDirs = "build/jars"` (§1.3 discovery step 4). Without that setting the `BootJar`
  strategy is not offered and `SpringBootMaven` becomes the preferred plan — see §6.3;
- `bin/run-gateway.bat` runs `java -Dfile.encoding=utf-8 -Xms512m -Xmx1024m -XX:MetaspaceSize=128m … -jar attach-gateway.jar`
  from `attach-gateway/target`;
- `.idea/jvm-parameter-settings.xml` contains the two Nacos namespace `-D` params (import off by default);
- `.idea` has **no** `libraries/` and **no** `modules.xml` ⇒ classpath via IDE metadata is unavailable here.

User settings:

```text
vmOptions    = -Xmx1g -Dspring.profiles.active=dev
programArgs  = --server.port=8081
```

### 6.1 Emitted configurations (5)

**1. `jvm-defaults:bootjar:attach-gateway` — "attach-gateway · JAR" (preferred default)**

```text
program : java                                   (javaExecutable="" -> "java")
cwd     : D:\workspace\water\water-server\attach-gateway\target        (cwdMode=auto, jar's parent)
args    : [
  "-Xmx1g",
  "-Dspring.profiles.active=dev",
  "-jar",
  "attach-gateway.jar",
  "--server.port=8081"
]
test    : false
framework: null
```

Final command the user sees (cmd rendering):

```bat
cd /d D:\workspace\water\water-server\attach-gateway\target
java.exe -Xmx1g -Dspring.profiles.active=dev -jar attach-gateway.jar --server.port=8081
```

Equivalent POSIX rendering:

```sh
cd '/d/workspace/water/water-server/attach-gateway/target' && \
java '-Xmx1g' '-Dspring.profiles.active=dev' -jar 'attach-gateway.jar' '--server.port=8081'
```

Variant actually available on this machine **today** (no `target/*.jar`), with
`jarSearchDirs = "build/jars"`:

```text
program : java
cwd     : D:\workspace\water\water-server\build\jars
args    : ["-Xmx1g", "-Dspring.profiles.active=dev", "-jar", "attach-gateway.jar", "--server.port=8081"]
```

(The id is unchanged — `jvm-defaults:bootjar:attach-gateway` — because the id never contains the jar directory or
version, §5.3. Moving from `build/jars` to `target` after a package build therefore does not orphan user overrides.)

**2. `jvm-defaults:spring-boot-maven:attach-gateway` — "attach-gateway · Spring Boot (Maven)"**

```text
program : D:\workspace\water\water-server\mvnw.cmd      (SpawnMode::ShellScript)
cwd     : D:\workspace\water\water-server
args    : [
  "-B",
  "-pl", "attach-gateway",
  "-am",
  "-Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev",
  "-Dspring-boot.run.arguments=--server.port=8081",
  "-Dspring-boot.run.fork=true",
  "spring-boot:run"
]
```

Final command (cmd rendering, i.e. the actual `cmd.exe /d /s /c` payload):

```bat
cd /d D:\workspace\water\water-server
mvnw.cmd -B -pl attach-gateway -am "-Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev" "-Dspring-boot.run.arguments=--server.port=8081" -Dspring-boot.run.fork=true spring-boot:run
```

**3. `jvm-defaults:maven-exec:attach-gateway` — "attach-gateway · exec:exec"** (only when
`strategies.mavenExec = true`; needs `exec-maven-plugin` resolvable)

```text
program : D:\workspace\water\water-server\mvnw.cmd
cwd     : D:\workspace\water\water-server
args    : [
  "-B", "-q", "-pl", "attach-gateway", "-am",
  "exec:exec",
  "-Dexec.executable=java",
  "-Dexec.args=-Xmx1g -Dspring.profiles.active=dev -classpath %classpath com.attach.gateway.AttachGatewayApplication --server.port=8081"
]
```

**4. `jvm-defaults:main:attach-gateway` — "attach-gateway · main class"** (default **off**; here it would also be
unavailable, because `strategies.plainMain = false`, no `target/classpath.txt`, no `installDist`, and no
`.idea/libraries` to derive a dependency classpath from). For the record, if `mvn dependency:copy-dependencies` had
been run:

```text
program : java
cwd     : D:\workspace\water\water-server\attach-gateway
args    : [
  "-Xmx1g", "-Dspring.profiles.active=dev",
  "-cp", "target/classes;target/dependency/*",
  "com.attach.gateway.AttachGatewayApplication",
  "--server.port=8081"
]
```

**5. `jvm-defaults:junit:access-processor.TestParseSampleData.parsePressure` — "TestParseSampleData#parsePressure · test"**

```text
program : D:\workspace\water\water-server\mvnw.cmd
cwd     : D:\workspace\water\water-server
args    : [
  "-B",
  "-pl", "attach-device/device-access/access-processor",
  "-Dtest=TestParseSampleData#parsePressure",
  "-DfailIfNoTests=false",
  "-Dsurefire.failIfNoSpecifiedTests=false",
  "-DargLine=-Xmx1g -Dspring.profiles.active=dev",
  "test"
]
test    : true
framework: "junit"            // value UNCONFIRMED
```

Note: the module relpath for that config (`access-processor`) has to be resolved from the module *name* → pom dir; if
unresolvable, the JUnit plan is emitted with `-pl` omitted and a warning, or skipped when `test` runs would hit the
whole reactor (default: skip + diag).

### 6.2 What `configurations()` returns (Rust shape)

```rust
vec![
  RunConfiguration {
    id: "jvm-defaults:bootjar:attach-gateway".into(),
    name: "attach-gateway · JAR".into(),
    program: java_program(),                       // "java" or javaExecutable
    args: vec!["-Xmx1g".into(), "-Dspring.profiles.active=dev".into(),
               "-jar".into(), "attach-gateway.jar".into(), "--server.port=8081".into()],
    cwd: Some(r"D:\workspace\water\water-server\attach-gateway\target".into()),
    test: false,
    framework: None,
  },
  RunConfiguration {
    id: "jvm-defaults:spring-boot-maven:attach-gateway".into(),
    name: "attach-gateway · Spring Boot (Maven)".into(),
    program: r"D:\workspace\water\water-server\mvnw.cmd".into(),
    args: vec!["-B".into(), "-pl".into(), "attach-gateway".into(), "-am".into(),
               "-Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev".into(),
               "-Dspring-boot.run.arguments=--server.port=8081".into(),
               "-Dspring-boot.run.fork=true".into(), "spring-boot:run".into()],
    cwd: Some(r"D:\workspace\water\water-server".into()),
    test: false, framework: None,
  },
  // ... maven-exec (if enabled), junit configs
]
```

`maxConfigurations = 20` is irrelevant here (5 plans); it matters for repos with 40+ `*Application` modules (this repo
has 20+ Spring Boot entry points, so the cap and the deterministic order both get exercised).

### 6.3 Same project, no jar built (the state on disk right now)

With `strategies.plainMain = false`, `strategies.mavenExec = false`, `jarSearchDirs = ""`:

```text
detected modules (application, ordered)  : attach-gateway, attach-ai, attach-auth, attach-dma, attach-knowledge,
                                           attach-revenue, attach-standalone, attach-work, ... (from
                                           .idea/workspace.xml SPRING_BOOT_MAIN_CLASS + *Application.java)
boot-jar candidates                      : none under */target, none under build/libs
=> emitted                               : 8 × SpringBootMaven  (plus 1 × JUnit per parsed JUnit config)
   1st plan: jvm-defaults:spring-boot-maven:attach-gateway
```

The first entry of the Run popup is therefore:

```text
program : D:\workspace\water\water-server\mvnw.cmd
cwd     : D:\workspace\water\water-server
args    : -B -pl attach-gateway -am
          "-Dspring-boot.run.jvmArguments=-Xmx1g -Dspring.profiles.active=dev"
          "-Dspring-boot.run.arguments=--server.port=8081"
          -Dspring-boot.run.fork=true
          spring-boot:run
```

Two consequences worth spelling out to the user (and to the Lead as UX text):

- first launch is slow (reactor build with `-am`); setting `buildDeps = false` after a successful `mvnw install` makes
  it fast;
- the moment `mvnw -pl attach-gateway package` (or the `microservice` profile build) produces
  `attach-gateway/target/attach-gateway.jar`, the next refresh emits the `JAR` plan and it sorts **before** the Maven
  plan — the user's choice in the popup changes without any action. If that is undesirable, set
  `preferStrategy = "springBootMaven"`.

---

## 7. Suggested module layout and core signatures

No crates (offline); `std` only, plus whatever the SDK provides. Keep every pure function testable against
`mock::MockHost` (see api-host) with **zero host calls**.

```text
src/lib.rs                 // plugin registration: register_plugin!(JvmDefaults, run_configurations)
src/settings.rs            // Settings struct + from_json(serde-free or serde if vendored) + scoping + defaults
src/tokens.rs              // tokenize(), join_tokens(), dedupe key, validator (pure)
src/merge.rs               // layers -> EffectiveParams { vm: Vec<String>, prog: Vec<String>, notes: Vec<MergeNote> }
src/detect/mod.rs          // detect(root, &Settings) -> Facts { modules, ide_configs, wrappers }  (+ cache)
src/detect/maven.rs        // pom parsing: <modules>, artifactId, spring-boot substring, finalName
src/detect/gradle.rs       // settings.gradle/build.gradle probing, bootJar output dirs
src/detect/ide_jetbrains.rs// lenient XML scanner -> Vec<IdeRunConfig>
src/detect/ide_vscode.rs   // strip_jsonc + minimal JSON parser -> Vec<IdeRunConfig>
src/detect/jars.rs         // jar discovery + filtering + pick memory
src/plan/mod.rs            // Strategy, LaunchPlan, plan_for(module, strategy, &Params, &Settings) -> LaunchPlan
src/plan/maven.rs          // springBootMaven + mavenExec argv builders
src/plan/gradle.rs         // springBootGradle argv + init-script writer
src/plan/java.rs           // bootJar + plainMain argv builders
src/plan/junit.rs          // surefire/gradle test argv builders
src/render.rs              // render_posix(), render_cmd(), cmd_quote(), sh_quote(), preview blocks
src/ids.rs                 // slug(), make_id(), collision resolution
src/runxml.rs              // Mode C writer (+ ownership/revert)
src/mvnconfig.rs           // Mode D managed-block writer (+ ownership/revert)
src/actions.rs             // preview command, "insert into VM options" quick fixes, notifications
```

Core signatures (implement these names so the tests map 1:1):

```rust
pub fn configurations(root: &str) -> Vec<RunConfiguration>;           // the export body
pub fn detect(root: &str, s: &Settings, h: &dyn Host) -> Facts;
pub fn plan_for(m: &Module, st: Strategy, p: &EffectiveParams, s: &Settings, facts: &Facts) -> Option<LaunchPlan>;
pub fn effective_params(m: &Module, facts: &Facts, s: &Settings, id: &str) -> EffectiveParams;
pub fn tokenize(text: &str) -> Result<Vec<String>, TokenizeError>;
pub fn dedupe(tokens: Vec<Tagged>) -> (Vec<String>, Vec<MergeNote>);
pub fn render_posix(p: &LaunchPlan) -> String;
pub fn render_cmd(p: &LaunchPlan) -> String;
pub fn validate_vm(tokens: &[String], mode: SerializationMode, spawn: SpawnMode) -> Vec<Diag>;
```

`configurations()` pipeline (no early returns except `enabled=false`):

```text
1  s   = Settings::load(root)              // global <- project
2  if !s.enabled || s.maxConfigurations == 0 { return vec![] }
3  facts = detect(root, &s, host)          // cached (§5.4); never panics, degrades to empty
4  if s.previewOnly { actions::show_preview(&facts, &s); return vec![] }
5  plans = for m in facts.modules_ordered() {      // §5.2 order
              for st in strategies_for(m, &s) {
                  if let Some(p) = plan_for(m, st, &effective_params(m, &facts, &s, id), &s, &facts) { yield p }
              } }
6  plans.truncate(s.maxConfigurations)
7  if s.writeRunXml != off { runxml::sync(&plans, &facts, &s) }      // idempotent, ownership-checked
8  if s.printPreviewOnRefresh { actions::print_previews(&plans) }
9  plans.map(|p| RunConfiguration { id, name, program, args, cwd, test, framework }).collect()
```

Hard rules for the implementation:

- `configurations()` must never return an `Err`-like empty/panic state; worst case = `vec![]` + one notification.
- No filesystem writes in the default configuration (`writeRunXml = off`, Mode D absent).
- Every string that reaches `program`/`args` must be absolute-or-PATH-resolvable and already shell-free.
- All ordering must come from the deterministic comparator, never from hash iteration order (`BTreeMap`/sorted vec).

---

## 8. Test plan (native `cargo test` with `mock::MockHost`; no network needed)

Unit (pure, no host):

1. `tokenize` table: `-Dmsg="a b"` ⇒ 1 token `-Dmsg=a b`; `-Dpath="C:\Program Files\x"` ⇒ backslashes preserved;
   `'single quoted'`; `#comment` line skipped; `-Durl=http://x#y` keeps `#`; unbalanced quote ⇒ error with offset;
   newline inside quotes ⇒ `V_E_NEWLINE_IN_QUOTE`; 513 tokens ⇒ `TooLarge`.
2. `dedupe`: `-Xmx1g,-Xmx2g` ⇒ `-Xmx2g` + note; `-Dk=1,-Dk` ⇒ `-Dk`; `-XX:+UseG1GC,-XX:-UseG1GC` ⇒ `-XX:-UseG1GC`;
   `-javaagent:a.jar,-javaagent:b.jar` ⇒ both kept; `--add-opens` duplicates kept.
3. `slug`/`make_id`: `attach-device/device-access/access-processor` ⇒ `attach-device.device-access.access-processor`;
   long path ⇒ 40 chars + `-` + 6 hex; stability: same input ⇒ same id; collision ⇒ `-2`.
4. `validate_vm`: each diag code fires exactly once for one fixture.
5. `strip_jsonc`: comments, trailing commas, `//` inside a URL string preserved, block comment spanning lines.
6. JetBrains parser: fixture = the real `<configuration name="AttachGatewayApplication" …>` block from §2.1 ⇒
   asserts `main_class == "com.attach.gateway.AttachGatewayApplication"`, `module == "attach-gateway"`,
   `vm.is_empty()`; a fixture with `<envs>`, `VM_PARAMETERS`, `$PROJECT_DIR$` macros; a `default="true"` template is
   skipped; malformed XML ⇒ empty result, no panic.
7. `render_cmd`/`render_posix` round-trip property: for random tokens without whitespace/quotes/`#`, the renderer's
   output tokenizes back to the same tokens.
8. Plan golden tests: the five §6.1 vectors, built from a fixture `Facts` (no filesystem).

Host-level (mock): `configurations(root)` with a fake `Host` that serves a fixture tree (paths + tiny pom/workspace.xml
strings); assert ids/names/args exactly, assert zero writes when `writeRunXml=off`, assert `previewOnly` contributes
nothing, assert a broken `workspace.xml` still yields the heuristic plans.

Optional integration (documented, not run here): build a throwaway Maven Spring Boot app with `mvnw`, run
`mvnw.cmd spring-boot:run -Dspring-boot.run.jvmArguments=-Dx=1 -Dspring-boot.run.arguments=--a=b` and check
`/actuator/env` or the startup log to confirm the `-D` reached the app JVM; same for a boot jar.

---

## 9. Open questions / UNCONFIRMED

| # | Question | Blocks | Default we ship until answered |
|---|---|---|---|
| Q1 | May `program` be a bare name (`java`) resolved via PATH, or must it be absolute (capability pinned by absolute path + hash)? | BootJar/PlainMain program string | `javaExecutable=""` ⇒ emit `"java"`; document the setting |
| Q2 | Does the host special-case `.cmd`/`.bat` (spawns via `cmd.exe` itself)? | all wrapper strategies on Windows | `spawnMode="auto"` + remember the verdict |
| Q3 | Can a coded provider declare `process:java`, `process:cmd.exe`? Or must we add `runDetectors` to auto-grant? | capability granting for `mvnw.cmd` | api-run owner to confirm; we ship `bootJar` (java only) as preferred |
| Q4 | Does the host's `find_files(glob, limit)` support `**` and `**/*.jar`? | module/jar discovery | `exists`/`stat` first, globs only as fallback |
| Q5 | Does the IDE re-read `.run/*.run.xml` while running, or only on project load? | Mode C usefulness | `writeRunXml` default `off`; tell the user to reload/restart |
| Q6 | Exact `[[contributes.settings]]` dialect (nested objects? `format: multiline`? per-project scope syntax) | schema file | conservative draft-07; fall back to flat string settings for `profiles`/`strategies` |
| Q7 | Is `configuration.id` charset/length constrained? Is it deduped against the IDE's own ids? | id scheme | `[a-z0-9._-]`, ≤ 96 chars |
| Q8 | Is there a clipboard/output-channel API for the preview? | preview UX | `notify` + (if available) `input_multiline` read-only |
| Q9 | `framework` accepted values for `test: true` | JUnit plans | `"junit"` |
| Q10 | `spring-boot.run.fork` default per Boot version; does `-q` hide the forked app's log? | S1 flags | pass `fork=true`, never `-q`; **[UNCONFIRMED]** |
| Q11 | Does `spring-boot-maven-plugin`'s splitter honour quotes inside `jvmArguments`? | values with spaces | warn (`V_W_QUOTED_IN_SERIALIZED`), do not block |
| Q12 | Does `exec-maven-plugin` `%classpath` exist in the version present in this machine's repo? | MavenExec strategy | `mavenExec` default `false` |
| Q13 | Does `host::storage` persist across component re-instantiation, and is it per project? | detection cache, jar picks | treat as best-effort; correctness must not depend on it |
| Q14 | Does the IDE's Maven support read `.mvn/jvm.config` too (Mode D side effects)? | Mode D safety | default off, warn loudly |

---

## Appendix A — quick reference: what reaches what

| Mechanism | JVM of the app | App arguments | Works for the IDE's own run buttons |
|---|---|---|---|
| `vmOptions` → BootJar `java <vm> -jar` | ✅ | ✅ (`args` after the jar) | ❌ (our config only) |
| `-Dspring-boot.run.jvmArguments` | ✅ (forked only) | — | ❌ |
| `-Dspring-boot.run.arguments` | ❌ | ✅ | ❌ |
| Gradle `-Dorg.gradle.jvmargs` / `gradle.properties` | ❌ (daemon) | — | ❌ |
| Gradle init script `jvmArgs` | ✅ | ✅ (`args`) | ❌ |
| `-Dexec.args` (`exec:exec`, `%classpath`) | ✅ (forked) | ✅ | ❌ |
| `exec:java` + `-Dexec.args` | ❌ (Maven's JVM) | ✅ | ❌ |
| `.mvn/jvm.config` (Mode D) | ✅ only for non-forked Maven runs | — | ✅ for Maven-launched JVMs |
| `JAVA_TOOL_OPTIONS` / `_JAVA_OPTIONS` / `JDK_JAVA_OPTIONS` | ✅ | — | ❌ (no env field; only our own `host::start`) |
| `MAVEN_OPTS` / `GRADLE_OPTS` | only non-forked build JVMs | — | ❌ same reason |
| `.run/*.run.xml` `VM_PARAMETERS` (Mode C) | ✅ (when the IDE (re)reads it) | ✅ | ✅ (subject to Q5) |

## Appendix B — argument-ordering invariants (do not "clean up")

1. `java`: every VM option **before** `-jar`/`-cp`/`-m`; main class or jar immediately after `-jar`/`-cp`; app args after.
2. `mvnw`: `-B` `-pl` `-am` `-o?` `-D…` `goal` (Maven accepts interleaving, but this order is stable and readable).
3. `-Dspring-boot.run.jvmArguments` value tokens keep the user's order; the last duplicate wins inside the forked JVM.
4. Never reorder tokens inside a VM layer (§3.3) — `-javaagent` order, `-XX:` order and `--add-opens` sets are
   semantically sensitive.
5. `--args=` / `-Dexec.args=` / `-Dspring-boot.run.arguments=` are **single argv elements**; the inner string is
   space-joined, never shell-quoted.
