# Changelog

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
