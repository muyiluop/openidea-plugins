# JVM Default Parameters — OpenIDEA 插件

给 OpenIDEA 的 Java 启动配置注入命令行参数（JVM 选项 + 程序参数），相当于 IntelliJ IDEA 里
`JVM Default Options` 加上 [JvmParameter](https://plugins.jetbrains.com/plugin/18022-jvm-parameter) 这类插件的组合：
一处配置、每个项目/模块自动生效，也可一键写回已有的 run 配置文件。

- 插件 id：`muyiluop.jvm-default-parameters`（`<作者命名空间>.<名字>`；本插件与 IDE 作者 `l0ng` 无关，
  官方自带插件用的是 `openidea.*` 命名空间，第三方用自己的 handle）
- 引擎：`engines.openidea = "^1.4"`（用到 `runConfigurations` 贡献点，API 1.4 起提供）
- 依赖：`openidea-plugin`（官方 Rust SDK，git 依赖）+ 本仓库内的 `crates/jvm-params-core`（纯逻辑，可离线单测）

---

## 1. 它做什么

同一个设置、两套投放方式：

### A. 贡献运行配置（Run 对话框里直接出现）

对每个满足条件的项目（根目录有 `pom.xml` / `build.gradle` / `build.gradle.kts`，且已信任），插件通过
`run-configurations` 导出列出启动配置，参数全部来自你的设置：

| 策略 | 生成的命令行 | 说明 |
| --- | --- | --- |
| `bootJar` | `java <VM 选项…> -jar <模块>/target/<app>.jar <程序参数…>` | 最忠实：VM 选项与程序参数都能完整注入；前提是插件能看到 jar |
| `springBootMaven` | `mvnw.cmd -B [-pl <模块> -am] -Dspring-boot.run.jvmArguments=<VM 选项> -Dspring-boot.run.fork=true -Dspring-boot.run.arguments=<程序参数> spring-boot:run` | 无需先打包；`fork=true` 保证 VM 选项传给被 fork 的应用 JVM |
| `springBootGradle` | `gradlew.bat bootRun --args=<程序参数>` | Gradle 命令行无法传 `jvmArgs`，所以这类配置**只带程序参数**，名字里会写明 |

默认每个模块只出一套（`bootJar` 优先，看不到 jar 时退到 wrapper 策略），可在设置里打开
`provider.emitAllStrategies` 让所有可用策略都出现在 Run 对话框。

### B. 写回 run 配置文件（相当于 JVM Default Options）

命令 **`JVM Default Parameters: Apply Parameters to Run Configuration Files`** 会就地修改 IDE 本来就会读取的文件：

| 文件 | 处理 |
| --- | --- |
| `.run/*.run.xml` | 合并 `VM_PARAMETERS`；`PROGRAM_PARAMETERS` 仅在你配置了程序参数时写入 |
| `.idea/runConfigurations/*.xml` | 同上 |
| `.vscode/launch.json` | 合并 `type: "java"` 配置的 `vmArgs` |

合并规则（两种模式，见 `jvmParams.mode`）：

- `append`（默认）：保留配置里已有的每个值，只补充配置里没有的 JVM 开关。**不会覆盖**你为某个配置单独写死的值。
- `override`：同一个开关（`-Dkey`、`-Xmx`、`-XX:Name`、`-XX:+Flag`、`-javaagent:path`）用默认值替换，位置保持不变，新开关追加。

同一文件重复执行是幂等的：已经合并过就不会再改动。

命令 **`JVM Default Parameters: Preview Generated Configurations`** 把当前项目会被生成的每一条命令行打到
`JVM Default Parameters` 输出面板，便于确认参数拼装是否正确。

---

## 2. 构建与安装

需要 Rust 1.85+（官方 SDK 使用 edition 2024）与 `wasm32-wasip2` 目标。

```bash
# 1) 官方工具链（提供 openidea-plugin 子命令）
cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin

# 2) 组件模型目标（插件产物是 wasm32-wasip2 组件）
rustup target add wasm32-wasip2

# 3) 构建 + 校验 + 链接进 IDE 的插件目录
openidea-plugin build        # → dist/plugin.wasm，随后自动 check
openidea-plugin link         # 把本目录写进 links.json（$OPENIDEA_PLUGINS_DIR，默认 %APPDATA%\ai.l0ng.openidea\plugins）
```

然后在 IDE 中 **Settings › Plugins › Reload**。改代码后的循环同样只需 `openidea-plugin build` + Reload。

> **SDK 版本**：`Cargo.toml` 依赖固定到一个 commit
> `rev = "4bc08a1…"`（2026-11-09 的 `main`）。原因：目前**没有任何 release tag 带运行配置 API** ——
> `v0.1.39` 的 SDK 还是 API 1.3，`wit/` 里没有 `run.wit`，`register_plugin!(…, run_configurations)`
> 会直接编译失败（`no rules expected identifier run_configurations`）。等官方发布带 API 1.4 的 tag 后，
> 把 `rev` 换成 `tag = "vX.Y.Z"` 即可；也可以指向本地 checkout：`openidea-plugin new . --sdk <仓库>/sdk/rust/openidea-plugin`。

### 只跑纯逻辑单测（不需要 SDK、不需要网络）

参数解析、合并、JetBrains/VS Code 文件解析与回写、Maven/Gradle 项目探测、启动计划生成全部在
`crates/jvm-params-core`，只依赖 `serde_json`：

```bash
cd crates/jvm-params-core
cargo test
```

被测行为包括：带引号的参数切分与重组、`append`/`override` 合并语义、`-D`/`-Xmx`/`-XX`/`-javaagent`
去重键、XML 属性转义与 span 回写、JSONC 注释与尾逗号、多模块 `-pl … -am` 顺序、jar 排除
（`-sources`/`-javadoc`/`-plain`/`.original`）、配置 id 稳定性等。

### 打包成可分享的插件包

```powershell
# Windows：跑单测 → 编译 wasm → 组装运行时目录 → 打 zip
pwsh -File scripts/package.ps1
#   → dist/muyiluop.jvm-default-parameters-0.1.0.zip
#   → release/muyiluop.jvm-default-parameters-0.1.0.zip（这一份进仓库，方便直接下载）

# 只打包源码（给要自己编译的人）
pwsh -File scripts/package.ps1 -SourceOnly
#   → dist/muyiluop.jvm-default-parameters-0.1.0-source.zip

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
`$OPENIDEA_PLUGINS_DIR`）下任意一个子目录，然后 **Settings › Plugins › Reload**；首次使用时按第 3 节
批准 `process:` 程序。脚本默认用 `openidea-plugin build` 产出 wasm，若该工具不在 PATH 上则退化为
`cargo build --release --target wasm32-wasip2` 再复制到 `dist/plugin.wasm`。

### 直接下载现成的插件包

仓库里已经放了一份打好包的插件（含 wasm 组件），不需要自己编译：

<https://github.com/muyiluop/openidea-plugins/raw/main/release/muyiluop.jvm-default-parameters-0.1.0.zip>

下载后按上面的步骤解压安装即可。`release/` 是对外分发的目录（跟着版本走），`dist/` 是本地构建输出、不进仓库。

### 本机实测过的构建结果

- `crates/jvm-params-core` 单测：**77 passed, 0 failed**（`cargo test --offline`）。
- 用官方 SDK（`main` @ `4bc08a1`）编译到 `wasm32-wasip2`：**成功，无源码改动**；
  产物 `openidea_plugin_jvm_params.wasm` 302,842 字节，文件头 `00 61 73 6d 0d 00 01 00`
  = 组件模型（layer 1），并且二进制里确实带有 `openidea:plugin/run-configurations` 导出与
  `jvm-params.preview` / `jvm-params.apply-run-files` 命令。
- 安装到 IDE、点击运行、`process:` 批准这几步需要你在自己的机器上完成（无法在构建机上验证）。

### 发布到 GitHub

仓库已发布：<https://github.com/muyiluop/openidea-plugins>（通过 SSH 推送；
HTTP(S) 在受限网络里可能因证书被拦，SSH 更稳）。

```bash
git remote set-url origin git@github.com:muyiluop/openidea-plugins.git
git push -u origin main
git push origin --tags            # 推送 v0.1.0 标签

# 之后每次发版
pwsh -File scripts/package.ps1    # 产出 dist/ 与 release/ 两份 zip
git add release/ && git commit -m "release: v<version>" && git push
git tag -a v<version> -m "<version>" && git push origin v<version>
```

也可以把 `release/*.zip` 作为 GitHub Release 的附件上传（网页上操作），二选一即可。

---

## 3. 首次使用：批准进程（重要）

OpenIDEA 的插件运行在沙箱里，**贡献的运行配置里的 `program` 是一个 token，必须先在插件清单里声明，并且由你批准后
IDE 才会为它固定真实路径**（按项目固定，`pin.verify()` 在每次列出/启动前重新校验）。因此：

1. 第一次打开某个项目的 Run 对话框时，若配置没有出现，说明 `process:<program>` 还没批准；
2. 到 **Settings › Plugins › JVM Default Parameters** 里批准对应的程序（`java`、`mvnw.cmd`、`gradlew.bat` 等），
   并选择该项目里的具体文件（Maven/Gradle wrapper 是本项目内的脚本，所以是**按项目**的批准）；
3. 之后该项目的配置会稳定出现。**替换 wrapper 文件后需要重新批准**（内容哈希变了）。

> 这也是为什么插件只产出 `java` / `mvnw` / `mvnw.cmd` / `gradlew` / `gradlew.bat`（以及设置里可选的
> `mvn` / `gradle`）这类裸 token，而不产出 `D:\proj\mvnw.cmd` 这样的绝对路径：绝对路径无法被声明为能力，
> 也就永远无法被批准，配置会静默消失。`plugin.toml` 的 `[capabilities].required` 与
> `crates/jvm-params-core/src/lib.rs` 的 `PROGRAM_CAPABILITIES` 必须保持一致。

`project.read` 用于扫描构建文件、wrapper、jar 以及读取 run 配置文件；`project.write` 只在执行“写回 run 配置文件”
命令时使用，安装时选择可选能力即可。

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
| `jvmParams.provider.enabled` | `true` | 是否贡献运行配置 |
| `jvmParams.provider.bootJar` | `true` | 是否出 `java -jar` 配置 |
| `jvmParams.provider.springBootMaven` | `true` | 是否出 Maven `spring-boot:run` 配置 |
| `jvmParams.provider.springBootGradle` | `true` | 是否出 Gradle `bootRun` 配置 |
| `jvmParams.provider.emitAllStrategies` | `false` | 每个模块输出全部可用策略 |
| `jvmParams.provider.maxConfigurations` | `24` | 单项目配置数量上限（1–256） |

设置读取失败从不报错：非法值退回默认并记录一条 warning（`Preview`/`activate` 时写入输出面板）。

---

## 5. 已知限制（都是宿主/沙箱约束，不是 bug）

1. **Gradle 的 VM 选项**：`gradlew bootRun` 不能从命令行传 `jvmArgs`（`-Dorg.gradle.jvmargs` 只影响 daemon）。
   所以 Gradle 项目的 VM 选项只能靠 `bootJar` 策略（`java -jar`）；文件写回路径不受此限。
2. **jar 发现**：`Project::find_files` 会跳过被忽略（`.gitignore`）的文件，`target/`、`build/` 常被忽略，
   因此 `bootJar` 策略在部分项目上不可用——此时自动退到 wrapper 策略。`jarSearchDirs` 可补充非忽略目录。
3. **环境变量**：`run-configuration` 记录里没有 `env` 字段，所以 `JAVA_TOOL_OPTIONS`、`MAVEN_OPTS` 之类
   无法通过贡献配置注入（只能 `host::start(Command{env})`，但那绕开了 IDE 的运行模型）。
4. **调试**：插件贡献的配置 `debug_adapter = None`，Debug 按钮会提示 "this configuration has no debugger"。
5. **不能改 IDE 内置的 Java 运行配置**：只能新增自己的配置，或写回它读取的 run 配置文件（即功能 B）。
6. **Windows/Linux**：插件产出的是平台对应的 wrapper token（`mvnw.cmd` vs `mvnw`），由项目根路径推断；
   POSIX 的 `mvnw` 脚本在 Windows 上不可直接执行，反之亦然。

---

## 6. 目录结构

```
.
├── plugin.toml              # 插件清单：id、能力、runConfigurations 标记文件、设置与命令
├── settings.schema.json     # 设置 schema（扁平 jvmParams.* 键）
├── icon.svg                 # 插件图标（≤16 KiB）
├── src/lib.rs               # SDK 适配层：设置读取、项目扫描、命令实现、run-configurations 导出
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
| Settings › JVM Default Options（新配置默认 VM 参数） | `jvmParams.vmOptions`，对贡献配置与写回文件同时生效 |
| JvmParameter 的 `<project>/.idea/jvm-parameter-settings.xml` | 未自动导入；把同样的值填进设置即可（文件按项目/全局两级作用域） |
| Run Configuration 里的 `VM options` / `Program arguments` | 写回命令直接改 `VM_PARAMETERS` / `PROGRAM_PARAMETERS` / `vmArgs` |

## 8. 许可

MIT。
