//! The few sentences the plugin writes itself, in the IDE's language.
//!
//! Text that comes from the manifest or the settings schema is translated by the
//! host from `locales/<tag>.json`; text the plugin builds (notifications, the
//! report a command returns) has to be translated here, so these strings follow
//! `host::locale()`: Chinese for `zh`, `zh-CN`, `zh-TW`, … and English otherwise.

use openidea_plugin::host;

/// The strings of one command run, in the IDE's language.
#[derive(Debug, Clone, Copy)]
pub struct Text {
    zh: bool,
}

impl Text {
    /// Reads the IDE's language once per command.
    pub fn new() -> Self {
        let locale = host::locale().to_ascii_lowercase();
        Self {
            zh: locale.starts_with("zh"),
        }
    }

    /// How many Spring Boot applications were found and how many tables changed.
    pub fn saved(&self, total: usize, changed: usize) -> String {
        if self.zh {
            format!(".openidea/run.toml：发现 {total} 个启动类，改写了 {changed} 条配置")
        } else {
            format!(".openidea/run.toml: {total} application(s) found, {changed} configuration(s) written")
        }
    }

    /// One configuration file that was patched.
    pub fn patched(&self, path: &str, count: usize) -> String {
        if self.zh {
            format!("已改写 {path}（{count} 条配置）")
        } else {
            format!("patched {path} ({count} configuration(s))")
        }
    }

    /// Values were pinned into the project's own settings file.
    pub fn pinned(&self) -> String {
        if self.zh {
            "已把这几个参数固定到本项目（.openidea/settings.toml），以后改全局设置不会影响本项目".to_string()
        } else {
            "pinned these values to this project (.openidea/settings.toml): later global edits no longer reach it"
                .to_string()
        }
    }

    /// The project pinning was skipped because the setting is off.
    pub fn pinning_off(&self) -> String {
        if self.zh {
            "未固定到本项目（`pinToProject` 已关闭），当前用的是全局配置".to_string()
        } else {
            "not pinned to this project (`pinToProject` is off): the global values are in effect".to_string()
        }
    }

    /// What to do after writing.
    pub fn next(&self) -> String {
        if self.zh {
            "下一步：打开运行对话框即可看到这些配置；若列表还是旧的，重新打开项目或重启 IDE".to_string()
        } else {
            "next: open the run dialog — the configurations are listed right away; reopen the project or restart the IDE if the list is stale"
                .to_string()
        }
    }

    /// A file that could not be written.
    pub fn failed(&self, what: &str) -> String {
        if self.zh {
            format!("失败：{what}")
        } else {
            format!("failed: {what}")
        }
    }

    /// A hint appended after the failures.
    pub fn hint(&self, what: &str) -> String {
        if self.zh {
            format!("提示：{what}")
        } else {
            format!("hint: {what}")
        }
    }

    /// The `project.write` capability is missing.
    pub fn need_write_capability(&self) -> String {
        if self.zh {
            "在 Settings › Plugins 里给本插件开启可选的 `project.write` 权限，才能写文件".to_string()
        } else {
            "allow the optional `project.write` capability for this plugin in Settings › Plugins to write files"
                .to_string()
        }
    }

    /// Nothing could be written at all.
    pub fn nothing_written(&self, failures: usize) -> String {
        if self.zh {
            format!("没能写入任何配置（{failures} 个错误）")
        } else {
            format!("could not write any configuration ({failures} failure(s))")
        }
    }

    /// The summary notification after a successful run.
    pub fn done(&self, saved: usize, updated: usize) -> String {
        if self.zh {
            format!("已写入 {saved} 条运行配置，改写了 {updated} 条已有配置")
        } else {
            format!("{saved} run configuration(s) written, {updated} existing one(s) patched")
        }
    }

    /// The preview notification.
    pub fn would_save(&self, count: usize) -> String {
        if self.zh {
            format!("会在 .openidea/run.toml 里写入 {count} 条运行配置")
        } else {
            format!("{count} application(s) would be saved in .openidea/run.toml")
        }
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}
