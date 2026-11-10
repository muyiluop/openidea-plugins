# Changelog

## 0.3.0 — 简化：一条命令、一次配置、全项目生效

针对反馈「插件做得复杂了，需求就是给自动识别出来的启动服务统一加 JVM/程序参数」重做：

* **能改到 IDE 自己识别的配置了**。这些配置存在 `.idea/workspace.xml` 里，而 `.idea/` 通常在
  `.gitignore` 中，`find_files` 会跳过它——0.2.0 因此报告"没有可改的文件"。现在按**字面路径**
  读写 `workspace.xml`（宿主的 `read_file`/`write_file` 只在根目录内限制，不受 gitignore 影响）。
  用真实的 52 KB `workspace.xml`（20 个 Spring Boot 配置）验证：一次写入 19 个配置，第二次 0 改动。
* **插件瘦身**：命令只留两个——`JVM Parameters: Apply to All Launch Configurations`（主功能）和
  `JVM Parameters: Preview`。删掉 `jvm-params.run`（终端里另起一套启动）、所有 `process:*` 能力和
  `Launch` / `Run dialog` 两组设置项，能力只剩 `project.read`（必需）+ `project.write`（可选）。
* 设置项收敛为 5 个：`enabled` / `vmOptions` / `programArgs` / `springProfiles` / `mode`。
* 写入后提示 `File › Reload Project`，因为 IDE 运行时会把 `workspace.xml` 留在内存里。
* 插入的 `<option>` 自动对齐原文件的缩进与换行（CRLF/LF），补丁保持幂等。
* 打包：`release/muyiluop.jvm-default-parameters-0.3.0.zip`（wasm 组件 296,290 字节，SDK `tag = "v0.1.39"`，API 1.3）。

## 0.2.0

改为 **plugin API 1.3**（当前已发布的 IDE v0.1.39 报告的就是 1.3.0），因此可以真正安装加载。
0.1.0 用了只在 `main` 上存在的 API 1.4，安装时被 IDE 拒绝（
`engines.openidea "^1.4" does not match this IDE's plugin API 1.3.0`）。

* 新增命令 `jvm-params.run`：按启动计划（`bootJar` / `springBootMaven` / `springBootGradle`）在 IDE 终端里
  直接启动，命令行就是最终 argv，不经过 shell，不需要先改任何文件。
* `jvm-params.apply-run-files` 保留为主功能（相当于 JVM Default Options）：写回 `.run/*.run.xml`、
  `.idea/runConfigurations/*.xml`、`.vscode/launch.json`。
* `jvm-params.preview` 改为返回文本（API 1.3 没有输出面板写入接口），并附一条通知。
* 能力调整：`required = ["project.read"]`，其余（`project.write` 与各 `process:*`）改为**可选能力**，
  用到时 IDE 才提示批准。
* `runConfigurations` 提供者移入 `src/provider.rs`，由 cargo feature `api14` 控制；
  IDE 与 SDK 升到 API 1.4 后一个 feature 就能打开。
* 打包：`release/muyiluop.jvm-default-parameters-0.2.0.zip`（124,816 字节，wasm 组件 300,385 字节，
  用 SDK `tag = "v0.1.39"` 构建）。
* 插件 id 用作者自己的命名空间：`muyiluop.jvm-default-parameters`（`l0ng` 是 IDE 作者的手柄/包名，与本插件无关）。

## 0.1.0

首个版本（**不可用**，见 0.2.0 的说明）。

* 动态运行配置提供者（`contributes.runConfigurations`，API 1.4）：为 Maven / Gradle 项目生成
  `bootJar`、`springBootMaven`、`springBootGradle` 三种启动方式，注入设置里的 JVM 参数与程序参数。
* 命令 `jvm-params.preview`：预览解析后的参数、策略与完整命令行。
* 命令 `jvm-params.apply-run-files`：把参数就地补写到 `.run/*.run.xml`、
  `.idea/runConfigurations/*.xml`、`.vscode/launch.json`（`append` 只补缺失项，`override` 原位替换）。
* 设置面板：`jvmParams.*` 扁平键，含合并模式、JVM/程序参数、`--spring.profiles.active`、
  Java 可执行文件、jar 搜索目录、提供者开关与数量上限。
* Maven 多模块支持：模块级 `pom.xml` 逐一生成配置，`-pl <模块> -am` 精准构建。
* 打包：`scripts/package.ps1` / `package.sh` 产出运行时布局的 zip。
