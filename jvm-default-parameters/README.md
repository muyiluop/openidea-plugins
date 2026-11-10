# JVM Default Parameters · OpenIDEA 插件

给 **所有自动识别出来的启动配置** 统一追加 JVM 参数（`-Xmx2g`、`-Dxxx=yyy` …）和程序参数
（`--server.port=8081` …），不用再一个个点开编辑。多模块 / 微服务工程一次配置，全部生效。

插件 id：`muyiluop.jvm-default-parameters`　·　API：`^1.3`　·　License：MIT

---

## 它解决什么问题

OpenIDEA 左侧运行树里的 `AttachAiApplication`、`AttachDmaApplication`、`AccessGatewayApplication`
这类条目，是 IDE **自己扫出来**的：它扫 `pom.xml` / `build.gradle` 里的 Spring Boot 插件和
`@SpringBootApplication` 类，运行时生成 `java -cp ${classpath} <MainClass>`，**完全没有 VM 参数**，
也没有任何 UI 能一次给它们全部加上参数。于是：

* 每个服务都要手动编辑一遍运行配置；
* 少写一个 `-Dspring.cloud.nacos.config.namespace=dev`，应用起来就报
  `Failed to configure a DataSource: 'url' attribute is not specified`。

本插件把参数写进 IDE 自己读取的位置，让这些自动生成的配置**带上你的参数**。

## 快速开始

1. 装好插件（见「安装」），打开项目。
2. `Settings › Plugins › JVM Default Parameters` 填两项：
   * **VM options**：`-Xmx2g -Dspring.cloud.nacos.config.namespace=new_dev_local -Dspring.cloud.nacos.discovery.namespace=new_dev_local`
   * **Program arguments**（可选）：`--server.port=8081`
3. 命令面板执行 **`JVM Parameters: Apply to All Launch Configurations`**。
4. 打开运行对话框 / 左侧 Spring Boot 树 —— 每个服务的条目已经在列表里，参数已经带上，直接 Run。

想先看效果：执行 **`JVM Parameters: Preview`**，它只报告会写什么，不动任何文件。

## 它写了什么

| 文件 | 作用 |
| --- | --- |
| `.openidea/run.toml` | **主功能**：为每个自动识别的 Spring Boot 应用保存一条同名 id 的配置，IDE 用它覆盖自己生成的条目，参数由此生效 |
| `.idea/workspace.xml` | 补齐 IntelliJ 时代留下的 `<option name="VM_PARAMETERS">`（`.idea/` 常被 gitignore，按字面路径读写） |
| `.run/*.run.xml`、`.idea/runConfigurations/*.xml` | 同上，针对显式创建的 JetBrains 配置 |
| `.vscode/launch.json` | `type: "java"` 的 `vmArgs` / `args` |

`.openidea/run.toml` 里生成的一条长这样（和 IDE 自己的 JetBrains 导入器写法一致，
`src/features/run/jvmTypes.ts` 会把它解析回表单字段，`${classpath}` 由 IDE 在启动前解析）：

```toml
[[configurations]]
id = "application:attach-dma:com.attach.dma.AttachDmaApplication"
name = "AttachDmaApplication"
kind = "spring-boot"
module = "attach-dma"
command = "java"
debug-adapter = "java-debug"
args = ["-Dspring.cloud.nacos.config.namespace=new_dev_local", "-cp", "${classpath}", "com.attach.dma.AttachDmaApplication", "--server.port=8081"]
```

id 的格式 `application:<模块目录>:<主类>`（根构建只有 `application:<主类>`）来自
`crates/ide-run/src/detect.rs`，与 IDE 生成的完全一致 —— 这正是「覆盖而不是重复」的关键。

## 合并规则

* **append（默认）**：配置里已有的值一个不动，只补它没有的开关。`-Xmx4g` 不会被 `-Xmx2g` 覆盖，
  但缺的 `-Dnacos.namespace` 会补上。
* **override**：同名开关（`-Xmx`、`-Dkey`、`-XX:Name`、`-javaagent:path`）用你的值替换，位置不变。
* 两种模式都**幂等**：重复执行不会重复追加，也不会改写你没碰过的字段（`env`、`cwd`、
  `before-launch` 等原样保留）。
* `jvmParams.springProfiles` 会作为 `--spring.profiles.active=…` 放在主类之后、程序参数之前。

## 设置项

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `jvmParams.enabled` | `true` | 总开关 |
| `jvmParams.vmOptions` | 空 | 追加的 JVM 参数（一行，可带引号） |
| `jvmParams.programArgs` | 空 | 追加的程序参数（一行） |
| `jvmParams.springProfiles` | 空 | 逗号分隔，生成 `--spring.profiles.active=` |
| `jvmParams.mode` | `append` | `append` / `override` |

设置是**项目级优先**的：给每个工程配各自的参数（`Settings` 右上角切 Project 作用域），
没有项目级值时用全局值。

## 能力

* `project.read`（必需）：读 `pom.xml` / `build.gradle` / 源码 / 运行配置文件。
* `project.write`（可选）：写 `.openidea/run.toml` 和上面那些配置文件。
  第一次执行命令时 IDE 会问一次，允许后方可写入。

没有 `process:*` 能力 —— 本插件不启动任何进程，只写文件。

## 已知限制

* 改完参数后**再执行一次命令**即可；新增模块 / 新增启动类同理（检测结果会随源码变化）。
* 如果你在 IDE 里手工编辑了某个运行配置，再次执行命令时它的 `args` 会被**合并**而不是还原：
  append 模式保留你写的值，只补缺的参数。
* IDE 运行中修改 `.idea/workspace.xml` 后需要 `File › Reload Project` 才会重新读取（`.openidea/run.toml`
  每次列出运行配置时都会读，不用重载）。
* 只处理 Java / Spring Boot 的运行配置；其它类型（shell、npm …）不动。
* 调试（Debug）走的是同一条命令行，`java-debug` 适配器保持原样。

## 安装

### 用现成的插件包

1. 下载仓库根目录的 `release/muyiluop.jvm-default-parameters-0.4.0.zip`。
2. 解压到插件目录，得到 `plugins/muyiluop.jvm-default-parameters/{plugin.toml,dist/plugin.wasm,…}`：

   ```text
   Windows: %APPDATA%\ai.l0ng.openidea\plugins\
   macOS:   ~/Library/Application Support/ai.l0ng.openidea/plugins/
   Linux:   ~/.local/share/ai.l0ng.openidea/plugins/
   ```

3. IDE 里 `Settings › Plugins` 点 **Reload**，允许 `project.read`（和 `project.write`）。
4. 需要分享时，把 zip 原样发出去即可，对方按同样步骤解压。

### 从源码构建

这个插件住在仓库的子目录 `jvm-default-parameters/`，构建都在这个目录里：

```bash
cargo install --locked --git https://github.com/l0ng-ai/openidea ide-host --bin openidea-plugin
rustup target add wasm32-wasip2

cd jvm-default-parameters        # 仓库里的插件目录
cargo test                       # 核心逻辑（crates/jvm-params-core，91 个测试，无需 SDK）
openidea-plugin build            # → dist/plugin.wasm（wasm 组件）
openidea-plugin link             # 把当前目录链进插件目录，然后在 Settings › Plugins › Reload
```

本机实测：SDK `tag = "v0.1.39"`，`wasm32-wasip2` 组件 368,521 字节；`crates/jvm-params-core`
`cargo test` 91 passed。打包成 zip（脚本在仓库根目录，对任意插件目录通用）：

```powershell
pwsh -File scripts/package.ps1 -Plugin jvm-default-parameters  # 构建 + 打包
pwsh -File scripts/package.ps1 -UseExistingWasm                 # 已有 dist/plugin.wasm 时只重打包
```

## 结构

```text
jvm-default-parameters/            插件目录（就是 zip 里的内容加源码）
  plugin.toml                      插件清单（API 1.3，两个命令，project.read/write）
  settings.schema.json             设置界面
  src/lib.rs                       SDK 适配层：读设置 → 扫描工程 → 写 run.toml / 补丁文件
  src/provider.rs                  （api14 特性）API 1.4 的 provider，暂不启用
  crates/jvm-params-core/          全部可测逻辑：设置、命令行解析与合并、JetBrains/VS Code 补丁、
                                   Maven/Gradle 检测、Spring Boot 应用扫描、run.toml 读写
  dist/plugin.wasm                 构建产物（忽略提交）
scripts/package.*                  仓库级打包脚本
release/                           仓库里提交的成品 zip
.research/                         宿主插件 API 摘录与设计笔记（多个插件共用）
```

## 和 IntelliJ 的对应关系

| IntelliJ | 本插件 |
| --- | --- |
| `JVM Default Options`（全局 VM 参数） | `jvmParams.vmOptions` + 一条命令写入所有运行配置 |
| JvmParameter 插件的项目级参数 | 项目作用域的 `vmOptions`（与 `.idea/jvm-parameter-settings.xml` 语义一致） |
| 每个配置的 `VM options` 输入框 | 不需要了 —— 生成/覆盖的配置自带参数 |

## License

MIT
