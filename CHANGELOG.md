# Changelog

## 0.1.0

首个版本。

* 动态运行配置提供者（`contributes.runConfigurations`，API 1.4）：为 Maven / Gradle 项目生成
  `bootJar`、`springBootMaven`、`springBootGradle` 三种启动方式，注入设置里的 JVM 参数与程序参数。
* 命令 `jvm-params.preview`：预览解析后的参数、策略与完整命令行。
* 命令 `jvm-params.apply-run-files`：把参数就地补写到 `.run/*.run.xml`、
  `.idea/runConfigurations/*.xml`、`.vscode/launch.json`（`append` 只补缺失项，`override` 原位替换）。
* 设置面板：`jvmParams.*` 扁平键，含合并模式、JVM/程序参数、`--spring.profiles.active`、
  Java 可执行文件、jar 搜索目录、提供者开关与数量上限。
* Maven 多模块支持：模块级 `pom.xml` 逐一生成配置，`-pl <模块> -am` 精准构建。
* 已知限制见 `README.md` 第 6 节（Gradle 无法从命令行注入 app 的 VM 参数、无 Debug、
  `.cmd` 包装器需按项目批准、`JAVA_TOOL_OPTIONS` 类环境变量无法通过运行记录注入）。
