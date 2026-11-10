# JVM Default Parameters — OpenIDEA 插件

在设置里填一次 **JVM 参数**（`-Xmx2g -Dxxx=yyy`）和 **程序参数**（`--server.port=8081`），
然后用一条命令把它们写进本项目的**所有**启动配置——包括 IDE 自己在 `.idea/workspace.xml`
里自动识别出来的那些 Spring Boot / Application 配置。不用再一个个点开 Edit Configurations 手动填。

这就是 IntelliJ 的 **JVM Default Options** 在 OpenIDEA 里的等价物。

## 为什么需要它

多模块 / 微服务的仓库里，IDE 会从各个 `*Application.java` 自动识别出几十个 Spring Boot 启动项。
每个都要单独加 `-Dspring.cloud.nacos.config.namespace=…`、`-Xmx2g` 之类的参数，改一次要重复几十遍。
本插件把这一步变成"填一次，一条命令，全项目生效"。

## 用法：两步

1. **填参数**：`Settings › Plugins › JVM Default Parameters`
   - **VM options**：`-Xmx2g -Xms512m -Dfile.encoding=UTF-8`
   - **Program arguments**（可选）：`--server.port=8081`
2. **应用**：命令面板（`Ctrl+Shift+P`）执行
   **`JVM Parameters: Apply to All Launch Configurations`**
   然后 `File › Reload Project`（或重启 IDE）让 IDE 重新读取配置文件。

执行完，每个被改的配置里会多出：

```xml
<option name="VM_PARAMETERS" value="-Xmx2g -Dspring.cloud.nacos.config.namespace=new_dev_local" />
<option name="PROGRAM_PARAMETERS" value="--server.port=8081" />
```

只想看不想改，用 **`JVM Parameters: Preview`**：它打印规划出的命令行和会被改的文件，不落盘。

## 它写哪些文件

| 文件 | 说明 |
| --- | --- |
| `.idea/workspace.xml` | **主要目标**：IDE 自动发现的 Spring Boot / Application / JUnit 配置都在这里。按字面路径读写——`.idea/` 通常在 `.gitignore` 里，模糊搜索根本找不到它 |
| `.run/*.run.xml` | JetBrains 风格的共享 run 配置 |
| `.idea/runConfigurations/*.xml` | 同上（目录形式） |
| `.vscode/launch.json` | 只改 `"type": "java"` 的 `vmArgs` |

合并规则（设置项 `Merge mode`）：

- **append**（默认）：配置里已有的参数**原样保留**，只补你新加的；同一个 JVM 开关（`-Xmx`、`-Dkey`、`-XX:Name`）已经在的话不覆盖。
- **override**：同一个开关以你设置的值为准，其余保留。

两种模式都是**幂等**的：重复执行不会重复追加，第二次执行会报告 0 处改动。

## 设置项

| 键 | 默认 | 作用 |
| --- | --- | --- |
| `jvmParams.enabled` | `true` | 总开关 |
| `jvmParams.vmOptions` | 空 | 追加到每个配置的 JVM 参数，一行，支持引号 |
| `jvmParams.programArgs` | 空 | 追加到每个配置的程序参数，一行；留空则只注入 JVM 参数 |
| `jvmParams.springProfiles` | 空 | 逗号分隔的 profile，追加为 `-Dspring.profiles.active=…` |
| `jvmParams.mode` | `append` | `append` / `override` |

## 首次使用：两个能力批准

插件是沙箱里的 wasm 组件，读写文件要显式批准（`Settings › Plugins`）：

- `project.read` — **必需**，读要改的配置文件。
- `project.write` — **可选**，但不给就只能预览、不能写入。插件会提示你缺这个。

## 构建

```bash
cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin
rustup target add wasm32-wasip2
openidea-plugin build          # → dist/plugin.wasm，并按引擎版本校验清单
openidea-plugin link           # 把本目录登记进 IDE 的插件目录
# 之后在 IDE 里 Settings › Plugins › Reload
```

只跑纯逻辑单测（不需要 SDK，也不需要网络）：

```bash
cd crates/jvm-params-core && cargo test
```

打包成可分享的 zip（Windows 用 **pwsh 7**，不要用 Windows PowerShell 5.1，它写出的 zip 用反斜杠）：

```bash
pwsh -File scripts/package.ps1                 # 单测 → 编译 → 组装 → zip
pwsh -File scripts/package.ps1 -UseExistingWasm # 已有 dist/plugin.wasm，只重新打包
```

产物：`dist/<id>-<version>.zip`，并在 `release/` 里留一份直接可下载的副本。

### 直接下载现成的插件包

[`release/muyiluop.jvm-default-parameters-0.3.0.zip`](https://github.com/muyiluop/openidea-plugins/raw/main/release/muyiluop.jvm-default-parameters-0.3.0.zip)
→ 解压到 `%APPDATA%\ai.l0ng.openidea\plugins\`（Windows）→ `Settings › Plugins › Reload`。

### 本机实测

- 宿主插件 API **1.3**（OpenIDEA v0.1.39，最新正式版），`engines.openidea = "^1.3"`。
- wasm 组件产物 296,290 字节，`00 61 73 6d 0d 00 01 00`（component model）。
- 纯逻辑单测 77 个全绿。
- 用真实的 52 KB `.idea/workspace.xml`（20 个 Spring Boot 配置）验证：一次写入 19 个配置，二次执行 0 改动。

## 已知限制

1. **写完要 Reload**。IDE 运行时把 `workspace.xml` 留在内存里，直接改盘上文件可能被它覆盖回写；报告和通知都会提示先 `Reload Project`（或重启）。
2. 插件 API 1.3 没有"贡献运行配置"的接口（那是未发布的 1.4），所以做不到 IntelliJ 那种实时生效——只能写文件，这也是为什么需要执行一条命令。
3. `JUnit` 配置同样会被加上这组 `VM_PARAMETERS`（对测试通常无害；介意的话用 `override` 或单独项目）。
4. 只注入 JVM/程序参数，不搬 classpath、不换 JDK、不改工作目录。
5. 写文件限制在项目根目录内（宿主沙箱），不碰 IDE 全局配置。

## 目录结构

```
plugin.toml                 插件清单（id / 引擎 / 命令 / 能力 / 设置）
settings.schema.json        设置项（JSON Schema，IDE 直接渲染成表单）
icon.svg                    图标
src/lib.rs                  API 1.3 适配层：两个命令 + 扫项目 + 调 core
src/provider.rs             1.4 的运行配置提供者，编译在 api14 feature 后面
crates/jvm-params-core/     全部可测逻辑（只依赖 serde_json，无 SDK）
  src/settings.rs             设置 → Defaults
  src/shell.rs                命令行切分/拼接（带引号）
  src/merge.rs                按 JVM 开关去重合并（append / override）
  src/jetbrains.rs            读写 JetBrains run 配置 XML（宽松解析，按字符串补丁）
  src/vscode.rs               读写 launch.json 的 vmArgs
  src/jsonc.rs                JSONC（带注释/尾逗号）清洗
  src/detect.rs               识别 Maven/Gradle、wrapper、多模块、可运行 jar
  src/plan.rs                 生成启动方案 + 预览文本
  src/patch.rs                把参数写回各类配置文件
scripts/                    打包脚本（PowerShell / bash）
release/                    可直接下载的 zip
.research/                  开发时整理的宿主 API 笔记（非运行必需）
```

## 与 IntelliJ 侧插件的对应关系

| IntelliJ | 本插件 |
| --- | --- |
| JVM Default Options（全局 VM 参数） | `jvmParams.vmOptions` + Apply 命令写进 `workspace.xml` |
| JvmParameter 插件（项目级 `-D` 参数，如 nacos namespace） | 同样填在 `jvmParams.vmOptions`，写进同一批配置 |
| 每个配置手点 Edit Configurations | 不需要，一次全量 |

## 许可

MIT
