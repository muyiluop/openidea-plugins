# JVM Default Parameters — OpenIDEA 插件

给 OpenIDEA 的 Java 启动注入命令行参数（JVM 选项 + 程序参数），相当于 IntelliJ IDEA 里
`JVM Default Options` 加上 [JvmParameter](https://plugins.jetbrains.com/plugin/18022-jvm-parameter) 这类插件的组合：
一处配置、每个项目/模块自动生效，也可以一键写回 IDE 本来就会读取的 run 配置文件。

- 插件 id：`muyiluop.jvm-default-parameters`（`<作者命名空间>.<名字>`；与 IDE 作者 `l0ng` 无关，
  官方自带插件用 `openidea.*`，第三方用自己的 handle）
- 引擎：`engines.openidea = "^1.3"`（**当前已发布的 IDE 报告的就是 plugin API 1.3.0**）
- 依赖：`openidea-plugin`（官方 Rust SDK，`tag = "v0.1.39"`）+ 本仓库内的 `crates/jvm-params-core`
  （纯逻辑，无 SDK 依赖，可离线单测）

> **为什么不是 1.4？** 官方的 `runConfigurations` 贡献点（让插件把自己的配置直接列进 Run 对话框）
> 只在仓库 `main` 上，**没有任何 release 带它**：最新 release `v0.1.39` 的 SDK 仍是 API 1.3，
> `wit/` 里没有 `run.wit`，而 IDE 安装插件时会校验 `<plugin>/plugin.toml: engines.openidea "^1.4"
> does not match this IDE's plugin API 1.3.0`，直接拒装。所以 0.2.0 用 API 1.3 能做的事来实现同样的目标：
> 写回 run 配置文件 + 在终端里按注入参数启动。等官方发布带 API 1.4 的版本后，把本仓库 `Cargo.toml`
> 打开 `api14` feature 即可启用 `src/provider.rs` 里的运行配置提供者（见第 2 节末尾）。

---

## 1. 它做什么

同一个设置、三种投放方式：

### A. 写回 run 配置文件（主功能，相当于 JVM Default Options）

命令 **`JVM Default Parameters: Apply Parameters to Run Configuration Files`** 就地修改 IDE 本来就会读取的文件：

| 文件 | 处理 |
| --- | --- |
| `.run/*.run.xml` | 合并 `VM_PARAMETERS`；`PROGRAM_PARAMETERS` 仅在你配置了程序参数时写入 |
| `.idea/runConfigurations/*.xml` | 同上 |
| `.vscode/launch.json` | 合并 `type: "java"` 配置的 `vmArgs` |

合并规则（`jvmParams.mode`）：

- `append`（默认）：保留配置里已有的每个值，只补充配置里没有的 JVM 开关。**不会覆盖**你为某个配置单独写死的值。
- `override`：同一个开关（`-Dkey`、`-Xmx`、`-XX:Name`、`-XX:+Flag`、`-javaagent:path`）用默认值替换，位置保持不变，新开关追加。

同一文件重复执行是幂等的：已经合并过就不会再改动。解析失败一律降级（跳过该配置并报告），不会损坏文件。

### B. 按注入参数在终端里运行

命令 **`JVM Default Parameters: Run with Injected Parameters`** 先按第 1.5 节的策略给当前项目算出一串启动命令——
直接就是最终 argv，不经过 shell——然后在 IDE 终端里启动它。多个候选时弹选择框（描述里带完整命令行）。
这样不改任何文件也能用上参数。

### C. 预览

命令 **`JVM Default Parameters: Preview Generated Configurations`** 把当前项目会生成的每一条命令行返回给你，
用来确认参数拼装是否正确。

### 1.5 启动策略（B 与未来的 A 共用）

对每个满足条件的模块（`pom.xml` / `build.gradle` / `build.gradle.kts`，且项目已信任）：

| 策略 | 生成的命令行 | 说明 |
| --- | --- | --- |
| `bootJar` | `java <VM 选项…> -jar <模块>/target/<app>.jar <程序参数…>` | 最忠实：VM 选项与程序参数都能完整注入 |
| `springBootMaven` | `mvnw.cmd -B [-pl <模块> -am] -Dspring-boot.run.jvmArguments=<VM 选项> -Dspring-boot.run.fork=true -Dspring-boot.run.arguments=<程序参数> spring-boot:run` | 无需先打包；`fork=true` 保证 VM 选项传给被 fork 的应用 JVM |
| `springBootGradle` | `gradlew.bat bootRun --args=<程序参数>` | Gradle 命令行无法传 `jvmArgs`，所以这类配置**只带程序参数**，名字里会写明 |

默认每个模块只出一套（`bootJar` 优先，看不到 jar 时退到 wrapper 策略），打开
`provider.emitAllStrategies` 可输出全部可用策略。

> 为什么 `program` 是 `mvnw.cmd` 这种裸 token 而不是绝对路径：宿主用配置记录里的 `program` 字符串
> **逐字**匹配 `process:<program>` 授权，绝对路径无法在清单里声明，也就永远批不下来（会被静默过滤）。

---

## 2. 构建与安装

需要 Rust 1.85+ 与 `wasm32-wasip2` 目标。

```bash
# 1) 官方工具链（提供 openidea-plugin 子命令）
cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin

# 2) 组件模型目标（插件产物是 wasm32-wasip2 组件）
rustup target add wasm32-wasip2

# 3) 构建 + 校验 + 链接进 IDE 的插件目录
openidea-plugin build        # → dist/plugin.wasm，随后自动 check
openidea-plugin link         # 把本目录写进 links.json（$OPENIDEA_PLUGINS_DIR，默认 %APPDATA%\ai.l0ng.openidea\plugins）
```

然后在 IDE 中 **Settings › Plugins › Reload**。

> 想在本地 checkout 的 SDK 上构建：`openidea-plugin new . --sdk <openidea 仓库>/sdk/rust/openidea-plugin`，
> 或把 `Cargo.toml` 里的 git 依赖换成 `path = "…/sdk/rust/openidea-plugin"`。

### 启用 API 1.4 的运行配置提供者（可选，现在用不了）

`src/provider.rs` 里已经写好了 `RunConfigurations` 实现，编译条件是一个 cargo feature：

```toml
# Cargo.toml：SDK 换成带 run.wit 的版本（main 上 4bc08a1 之后的任意 commit）
openidea-plugin = { git = "https://github.com/l0ng-ai/openidea", rev = "4bc08a1…" }
```

```bash
cargo build --release --target wasm32-wasip2 --features api14
```

同时还要把 `plugin.toml` 改成 `engines.openidea = "^1.4"` 并加回：

```toml
[[contributes.runConfigurations]]
files = ["pom.xml", "build.gradle", "build.gradle.kts"]
```

**只有 IDE 自己也升级到 plugin API ≥ 1.4 之后，这样装上的包才能被加载。**

### 只跑纯逻辑单测（不需要 SDK、不需要网络）

参数解析、合并、JetBrains/VS Code 文件解析与回写、Maven/Gradle 项目探测、启动计划生成全部在
`crates/jvm-params-core`，只依赖 `serde_json`：

```bash
cd crates/jvm-params-core
cargo test
```

被测行为包括：带引号的参数切分与重组、`append`/`override` 合并语义、`-D`/`-Xmx`/`-XX`/`-javaagent`
去重键、XML 属性转义与 span 回写、JSONC 注释与尾逗号、多模块 `-pl … -am` 顺序、jar 排除
（`-sources`/`-javadoc`/`-plain`/`.original`）、配置 id 稳定性、以及“计划产出的每个 `program` 都必须是清单里
声明过的能力 token”。

### 打包成可分享的插件包

```powershell
# Windows：跑单测 → 编译 wasm → 组装运行时目录 → 打 zip
pwsh -File scripts/package.ps1
#   → dist/muyiluop.jvm-default-parameters-0.2.0.zip
#   → release/muyiluop.jvm-default-parameters-0.2.0.zip（这一份进仓库，方便直接下载）

# 只打包源码（给要自己编译的人）
pwsh -File scripts/package.ps1 -SourceOnly

# 已有 dist/plugin.wasm 时只重新打包，不编译（改了清单/设置时用）
pwsh -File scripts/package.ps1 -UseExistingWasm
```

```bash
# Git Bash / Linux / macOS
bash scripts/package.sh
bash scripts/package.sh --source
bash scripts/package.sh --existing-wasm
```

> 用 **pwsh (PowerShell 7+)** 而不是 Windows PowerShell 5.1：5.1 的 .NET 写出的 zip 条目分隔符是 `\`，
> 在 Linux/macOS 上解压会得到一个名字里带反斜杠的文件。脚本用
> `[System.IO.Compression.ZipFile]::CreateFromDirectory` 生成 `/` 分隔的标准 zip。

插件包的结构就是运行时的目录结构，压缩包根目录是插件目录本身：

```
muyiluop.jvm-default-parameters/
├── plugin.toml
├── settings.schema.json
├── icon.svg
├── README.md
├── LICENSE
└── dist/plugin.wasm        # 组件（其它文件都是数据）
```

**收到插件包的人这样安装**：把压缩包解压到 `%APPDATA%\ai.l0ng.openidea\plugins\`（或
`$OPENIDEA_PLUGINS_DIR`）下任意一个子目录，然后 **Settings › Plugins › Reload**。脚本默认用
`openidea-plugin build` 产出 wasm，若该工具不在 PATH 上则退化为 `cargo build --release --target
wasm32-wasip2` 再复制到 `dist/plugin.wasm`。

### 直接下载现成的插件包

仓库里已经放了一份打好包的插件（含 wasm 组件），不需要自己编译：

<https://github.com/muyiluop/openidea-plugins/raw/main/release/muyiluop.jvm-default-parameters-0.2.0.zip>

下载后按上面的步骤解压安装即可。`release/` 是对外分发的目录（跟着版本走），`dist/` 是本地构建输出、不进仓库。

### 本机实测过的构建结果

- `crates/jvm-params-core` 单测：**77 passed, 0 failed**。
- 用官方 SDK `tag = "v0.1.39"`（= IDE 的 plugin API 1.3.0）编译到 `wasm32-wasip2`：**成功**，
  产物 `openidea_plugin_jvm_params.wasm` 300,385 字节，文件头 `00 61 73 6d 0d 00 01 00` = 组件模型（layer 1），
  二进制里带 `jvm-params.preview` / `jvm-params.apply-run-files` / `jvm-params.run` 与
  `openidea:plugin/settings`、`openidea:plugin/views` 导入。
- （0.1.0 那版是针对 `main` 上的 API 1.4 构建的，装不进 1.3 的 IDE，已废弃。）
- 安装到 IDE、点击命令、批准 `process:` 这几步需要你在自己的机器上确认。

### 发布到 GitHub

仓库：<https://github.com/muyiluop/openidea-plugins>（通过 SSH 推送；HTTP(S) 在受限网络里可能因证书被拦，SSH 更稳）。

```bash
git remote set-url origin git@github.com:muyiluop/openidea-plugins.git
git push -u origin main
git push origin --tags

# 之后每次发版
pwsh -File scripts/package.ps1    # 产出 dist/ 与 release/ 两份 zip
git add release/ && git commit -m "release: v<version>" && git push
git tag -a v<version> -m "<version>" && git push origin v<version>
```

也可以把 `release/*.zip` 作为 GitHub Release 的附件上传（网页上操作），二选一即可。

---

## 3. 首次使用：可选能力与进程批准

`project.read` 是必需能力（扫描构建文件、wrapper、jar，读取 run 配置文件）。其余都是**可选能力**，用到时才需要批准：

| 能力 | 何时需要 |
| --- | --- |
| `project.write` | 执行“写回 run 配置文件”命令时 |
| `process:java` | 用 `bootJar` 策略在终端里启动时 |
| `process:mvnw` / `process:mvnw.cmd` | 用 Maven wrapper 策略启动时（本项目内的脚本，**按项目批准**） |
| `process:gradlew` / `process:gradlew.bat` | 用 Gradle wrapper 策略启动时 |
| `process:mvn` / `process:gradle` | 只有在设置里把 `mavenProgram` / `gradleProgram` 改成这两个名字时才需要 |

- 在 **Settings › Plugins › JVM Default Parameters** 里批准；wrapper 属于项目内的文件，所以是**按项目**固定路径，
  宿主在每次启动前用内容哈希重新校验，**替换 wrapper 文件后需要重新批准**。
- 命令返回的错误里会写明缺哪个能力，照着批即可。
- `plugin.toml` 的 `[capabilities].optional` 与 `crates/jvm-params-core/src/lib.rs` 的 `PROGRAM_CAPABILITIES`
  必须保持一致（后者有单测守着）。

> 终端启动（`jvm-params.run`）使用的是宿主的 `open_terminal`，它**总是在项目根目录**里执行，不认模块子目录；
> `bootJar` 策略的 jar 路径是绝对的，所以不受影响。

---

## 4. 设置项

| 键 | 默认 | 作用 |
| --- | --- | --- |
| `jvmParams.enabled` | `true` | 总开关 |
| `jvmParams.vmOptions` | 空 | 注入的 JVM 选项（一行，可用引号，例如 `-Xmx2g -XX:+UseG1GC`） |
| `jvmParams.programArgs` | 空 | 注入的程序参数（例如 `--server.port=8081`） |
| `jvmParams.springProfiles` | 空 | 逗号分隔的 profile，前置为 `-Dspring.profiles.active=…` |
| `jvmParams.mode` | `append` | `append` 补缺 / `override` 同键替换 |
| `jvmParams.javaExecutable` | `java` | `-jar` 启动用的程序；保持 `java` 才能被批准 |
| `jvmParams.platform` | `auto` | `auto` 由项目根路径推断（WASM 插件拿不到 OS 信息） |
| `jvmParams.mavenProgram` | 空 | 覆盖 Maven 程序名（默认用项目 wrapper） |
| `jvmParams.gradleProgram` | 空 | 覆盖 Gradle 程序名 |
| `jvmParams.jarSearchDirs` | `[]` | 额外 jar 目录（相对模块，如 `build/jars`） |
| `jvmParams.provider.enabled` | `true` | 是否参与启动计划生成 |
| `jvmParams.provider.bootJar` | `true` | 是否出 `java -jar` 策略 |
| `jvmParams.provider.springBootMaven` | `true` | 是否出 Maven `spring-boot:run` 策略 |
| `jvmParams.provider.springBootGradle` | `true` | 是否出 Gradle `bootRun` 策略 |
| `jvmParams.provider.emitAllStrategies` | `false` | 每个模块输出全部可用策略 |
| `jvmParams.provider.maxConfigurations` | `24` | 单项目配置数量上限（1–256） |

设置读取失败从不报错：非法值退回默认并记录一条 warning（`activate` 时写进插件日志）。

---

## 5. 已知限制（都是宿主/沙箱约束，不是 bug）

1. **API 1.3 没有 `runConfigurations`**：插件不能把自己的配置列进 Run 对话框，所以改成“写回文件 + 终端启动”两条路。
   等 IDE 升到 1.4 才能用 `api14` feature 打开真正的 provider（见第 2 节）。
2. **Gradle 的 VM 选项**：`gradlew bootRun` 不能从命令行传 `jvmArgs`（`-Dorg.gradle.jvmargs` 只影响 daemon），
   所以 Gradle 项目的 VM 选项只能靠 `bootJar` 策略（`java -jar`）；文件写回路径不受此限。
3. **jar 发现**：`Project::find_files` 会跳过被忽略（`.gitignore`）的文件，`target/`、`build/` 常被忽略，
   因此 `bootJar` 策略在部分项目上不可用——此时自动退到 wrapper 策略。`jarSearchDirs` 可补充非忽略目录。
4. **环境变量**：启动记录里没有 `env` 字段，`JAVA_TOOL_OPTIONS` / `MAVEN_OPTS` 这类变量无法注入；
   需要的话写进 wrapper 或 `.mvn/jvm.config` / `gradle.properties`。
5. **终端启动在项目根目录**：宿主的 `open_terminal` 固定 cwd = 项目根；多模块的 `spring-boot:run` 用 `-pl <模块>`
   指定模块，不依赖 cwd。
6. **Windows/Linux**：插件产出的是平台对应的 wrapper token（`mvnw.cmd` vs `mvnw`），由项目根路径推断；
   POSIX 的 `mvnw` 脚本在 Windows 上不可直接执行，反之亦然。

---

## 6. 目录结构

```
.
├── plugin.toml              # 插件清单：id、能力、设置与会命令
├── settings.schema.json     # 设置 schema（扁平 jvmParams.* 键）
├── icon.svg                 # 插件图标（≤16 KiB）
├── src/lib.rs               # SDK 适配层（API 1.3）：设置读取、项目扫描、三个命令
├── src/provider.rs          # RunConfigurations 提供者（API 1.4，`--features api14` 时编译）
├── scripts/package.ps1      # 打包：单测 → wasm → dist/<id>-<version>.zip
├── scripts/package.sh       # 同上（POSIX shell）
├── release/                 # 对外分发的插件包（进仓库，可直接下载）
├── CHANGELOG.md
├── LICENSE
├── crates/jvm-params-core/  # 纯逻辑 + 单测（只依赖 serde_json）
│   ├── src/settings.rs      # host::setting → Defaults
│   ├── src/shell.rs         # 命令行切分/重组（带引号）
│   ├── src/merge.rs         # append/override 合并、去重键
│   ├── src/jetbrains.rs     # .run.xml / workspace.xml 解析与回写
│   ├── src/vscode.rs        # launch.json（JSONC）解析与 vmArgs 回写
│   ├── src/jsonc.rs         # 注释/尾逗号剥离
│   ├── src/detect.rs        # Maven/Gradle 多模块、wrapper、Spring Boot、jar 探测
│   ├── src/plan.rs          # Platform/策略/GeneratedConfig/预览
│   └── src/patch.rs         # 三类 run 配置文件的就地合并
├── dist/plugin.wasm         # 构建产物（git 忽略）
└── .research/               # 宿主 API 逆向笔记（可删）
```

## 7. 与 IntelliJ 侧插件的对应关系

| IntelliJ | 本插件 |
| --- | --- |
| Settings › JVM Default Options（新配置默认 VM 参数） | `jvmParams.vmOptions`；写回命令把它补进已有配置（`append` 不会覆盖显式值） |
| JvmParameter 的 `<project>/.idea/jvm-parameter-settings.xml` | 未自动导入；把同样的值填进设置即可（按项目/全局两级作用域） |
| Run Configuration 里的 `VM options` / `Program arguments` | 写回命令直接改 `VM_PARAMETERS` / `PROGRAM_PARAMETERS` / `vmArgs` |
| 直接在 IDE 里 Run | 命令 `Run with Injected Parameters`（终端启动，等价效果） |

## 8. 许可

MIT。
