# openidea-plugins

我给自己写的 [OpenIDEA](https://github.com/l0ng-ai/openidea) 插件集合。**一个子目录就是一个插件**：
各自独立构建、独立打包、独立安装，互不影响。

| 插件 | 作用 | 下载 |
| --- | --- | --- |
| [jvm-default-parameters](jvm-default-parameters/) | 给 JVM / Spring Boot 启动配置追加 VM 参数和程序参数：写进项目 `.openidea/run.toml`，直接覆盖 IDE 自动识别出来的那些微服务启动项，不用一个个手改；界面中英双语，参数默认锁定到本项目 | [0.5.0 zip](release/muyiluop.jvm-default-parameters-0.5.0.zip) |

## 安装插件

1. 下载上面表格里的 zip。
2. 解压到 OpenIDEA 的插件目录，得到一个以插件 id 命名的文件夹：

   ```text
   Windows: %APPDATA%\ai.l0ng.openidea\plugins\
   macOS:   ~/Library/Application Support/ai.l0ng.openidea/plugins/
   Linux:   ~/.local/share/ai.l0ng.openidea/plugins/
   ```

3. IDE 里 `Settings › Plugins` 点 **Reload**，按提示允许插件声明的能力。
4. 分享给别人就是把 zip 原样发出去。

## 仓库结构

```text
jvm-default-parameters/   一个插件：plugin.toml、settings.schema.json、src/、crates/*-core/、dist/
release/                  打包好的 zip（按插件 id 命名，可直接从 GitHub 下载）
scripts/                  构建与打包脚本，对任意插件目录通用
.research/                宿主插件 API 的摘录与设计笔记（多个插件共用）
LICENSE                   MIT
```

每个插件目录里都有自己的 `Cargo.toml`（带一个空的 `[workspace]` 表把它和上层隔开）和
`crates/*-core`（不依赖 SDK、可以离线 `cargo test` 的核心逻辑）。**仓库根目录故意不放
`Cargo.toml`**，否则几个插件会被并进同一个 workspace 而互相牵连。

## 构建 & 打包

```powershell
pwsh -File scripts/package.ps1 -Plugin jvm-default-parameters   # 测试 → 构建 wasm → 打包 zip
pwsh -File scripts/package.ps1 -UseExistingWasm                 # 已有 dist/plugin.wasm 时只重打包
```

只有一个插件时 `-Plugin` 可以省略。POSIX 下等价的是 `./scripts/package.sh jvm-default-parameters`。

产物有两份：`<plugin>/dist/<id>-<version>.zip`（构建产物，忽略提交）和
`release/<id>-<version>.zip`（提交进仓库，方便从 GitHub 直接下载）。

## 新增一个插件

```powershell
mkdir my-plugin
cd my-plugin
openidea-plugin new          # 生成模板，或者手写 plugin.toml + Cargo.toml
```

要点：

* `plugin.toml` 的 `id` 用「作者名.插件名」命名空间（例如 `muyiluop.my-plugin`），这个 id 会被
  宿主写进 `plugin:<id>:<local>` 的运行配置 id 和设置存储里，改了会破坏用户已有的配置。
* `Cargo.toml` 里保留空的 `[workspace]` 表，`crate-type = ["cdylib", "rlib"]`。
* SDK 依赖写 `openidea-plugin = { git = "https://github.com/l0ng-ai/openidea", tag = "vX.Y.Z" }`，
  tag 要选和 IDE 插件 API 对得上的版本（宿主 API 1.3 对应 `v0.1.39`）。
* 能脱离 SDK 的逻辑放进 `crates/<name>-core`，这样 `cargo test --offline` 就能跑。

## 为什么每个插件自己一个 workspace

OpenIDEA 的插件 crate 依赖 `openidea-plugin`（git 依赖 + `wasm32-wasip2` 目标），官方仓库也是
`sdk/`、`plugins/` 各自独立成 workspace。放在一个 `[workspace]` 里会共享 `Cargo.lock`、
feature 解析和 target 目录，插件之间会互相拖累；分开之后每个目录都能单独 `check`、单独发版。
