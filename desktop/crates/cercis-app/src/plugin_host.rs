//! 插件宿主：app 侧只做三件事——**发现、授权、调用**。
//!
//! app 自己不实现插件能力（那是工具包的事），也不替插件写数据：
//! 它把插件当子进程调用，因此插件崩溃、卡住都不影响主界面与数据完整性
//! （需求文档 §7.16 FR-P06 / FR-P18）。
//!
//! 调用契约（插件协议的一部分，写在需求文档 §14.7 接口 A）：
//!
//! ```text
//! <插件入口> <命令…> --project <项目目录> --json [--cercis <内核命令>] [--allow-write]
//! ```
//!
//! - `--json`：插件必须回一份带 `ok` 的 JSON（app 与 AI 都读它）
//! - `--allow-write`：**只有**插件清单声明了 `write` 且已被信任时才加上
//! - 未信任的项目内插件一律不运行（D27 / FR-P31）

use std::path::{Path, PathBuf};
use std::process::Command;

use cercis_core::plugin::DiscoveredPlugin;
use serde_json::Value;

/// 一次插件调用的结果。
#[derive(Debug)]
pub struct RunOutcome {
    pub code: i32,
    /// 插件返回的 JSON（解析失败时为 None，此时看 stdout/stderr）
    pub result: Option<Value>,
    pub stdout: String,
    pub stderr: String,
    /// 实际执行的命令行（便于用户复制复现，也便于排查）
    pub command_line: String,
}

impl RunOutcome {
    pub fn succeeded(&self) -> bool {
        self.code == 0
            && self
                .result
                .as_ref()
                .map(|v| v.get("ok").and_then(|b| b.as_bool()).unwrap_or(true))
                .unwrap_or(false)
    }
}

/// 插件是否可以运行：项目内插件必须先被信任；全局插件视为已安装。
pub fn is_runnable(plugin: &DiscoveredPlugin, trusted: bool) -> bool {
    match plugin.origin.as_str() {
        "project" => trusted,
        _ => true,
    }
}

/// 在发现结果里挑出要用的那个插件。
///
/// 同名插件同时存在于项目内与全局时，**项目内的优先**：
/// 项目自带的是这份数据专属的版本，且它必须被信任才运行。
pub fn select<'a>(
    found: &'a [DiscoveredPlugin],
    id: &str,
) -> Option<&'a DiscoveredPlugin> {
    found
        .iter()
        .find(|p| p.manifest.id == id && p.origin == "project")
        .or_else(|| found.iter().find(|p| p.manifest.id == id))
}

/// 找到插件的可执行入口。
///
/// 顺序：插件目录内的同名文件 → 插件目录的 bin/ → app 同级目录 → PATH。
/// 这样既能跑"随项目分发"的插件，也能跑本机安装好的插件。
pub fn resolve_entrypoint(plugin_dir: &Path, entrypoint: &str) -> Option<PathBuf> {
    let entrypoint = entrypoint.trim();
    if entrypoint.is_empty() {
        return None;
    }
    let name = exe_name(entrypoint);

    let candidates = [
        plugin_dir.join(&name),
        plugin_dir.join("bin").join(&name),
    ];
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(&name);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    find_on_path(&name)
}

/// 内核命令行：插件用它读写数据（app 只负责把路径告诉插件）。
pub fn kernel_cli() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CERCIS_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(exe_name("cercis"));
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    if let Some(p) = find_on_path(&exe_name("cercis")) {
        return Some(p);
    }
    // 开发工作区：apps/desktop/crates/cercis-app → Cercis/kernel/target/debug/cercis
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../kernel/target/debug")
        .join(exe_name("cercis"));
    dev.is_file().then_some(dev)
}

/// 全局插件目录（D26 / FR-P29：插件可以装在项目内，也可以全局安装）。
///
/// 顺序：`CERCIS_PLUGINS` 环境变量 → `~/.cercis/plugins`。
/// 全局插件是用户自己装的，视为已安装即可运行；项目内插件则必须先被信任。
pub fn global_plugins_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CERCIS_PLUGINS") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join(".cercis").join("plugins");
    dir.is_dir().then_some(dir)
}

/// 运行插件。
///
/// `command` 是插件清单里声明的命令（如 `entry append`），`extra` 是本次的额外参数。
pub fn run(
    project_root: &Path,
    plugin: &DiscoveredPlugin,
    trusted: bool,
    command: &str,
    extra: &[String],
) -> Result<RunOutcome, String> {
    if !is_runnable(plugin, trusted) {
        return Err(format!(
            "插件 {} 来自项目内且尚未被信任，按安全规则不会运行",
            plugin.manifest.id
        ));
    }

    let entry = resolve_entrypoint(&plugin.path, &plugin.manifest.entrypoint)
        .ok_or_else(|| {
            format!(
                "找不到插件入口 {}（可在插件目录内放置该可执行文件，或装到系统 PATH）",
                plugin.manifest.entrypoint
            )
        })?;

    let mut args: Vec<String> = command
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    // 调用方没给 --project 时，默认指当前项目（项目向导会自己给，指向新项目）
    if !extra.iter().any(|a| a == "--project") && !args.iter().any(|a| a == "--project") {
        args.push("--project".into());
        args.push(project_root.display().to_string());
    }

    for a in extra {
        // 这两个参数由宿主统一注入，避免调用方伪造
        let trimmed = a.trim();
        if trimmed.is_empty() || trimmed.starts_with("--cercis") || trimmed == "--json" {
            continue;
        }
        args.push(trimmed.to_string());
    }

    args.push("--json".into());
    if let Some(kernel) = kernel_cli() {
        args.push("--cercis".into());
        args.push(kernel.display().to_string());
    }
    // 权限：只有清单声明了 write、且插件被允许运行时，才授予写入
    if plugin.manifest.permissions.write {
        args.push("--allow-write".into());
    }

    let command_line = format!("{} {}", entry.display(), args.join(" "));
    let output = Command::new(&entry).args(&args).output().map_err(|e| {
        format!(
            "插件 {} 启动失败：{e}（入口 {}）",
            plugin.manifest.id,
            entry.display()
        )
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let result = serde_json::from_str::<Value>(stdout.trim()).ok();

    Ok(RunOutcome {
        code: output.status.code().unwrap_or(-1),
        result,
        stdout,
        stderr,
        command_line,
    })
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cercis_core::plugin::{Permissions, PluginManifest, PluginTier, Surfaces};

    fn plugin(origin: &str) -> DiscoveredPlugin {
        DiscoveredPlugin {
            manifest: PluginManifest {
                id: "health_check".into(),
                name: "Health Check".into(),
                label_cn: "通用体检".into(),
                version: "0.1.0".into(),
                tier: PluginTier::Basic,
                requires_plugin_api: 1,
                permissions: Permissions {
                    read: true,
                    write: false,
                    network: false,
                },
                surfaces: Surfaces::default(),
                ui: vec![],
                entrypoint: "cercis-plugin-health".into(),
                description: String::new(),
            },
            origin: origin.into(),
            path: PathBuf::from("/tmp/plugins/health_check"),
            fingerprint: "abc".into(),
            trusted: false,
        }
    }

    #[test]
    fn project_plugins_need_trust_before_running() {
        assert!(!is_runnable(&plugin("project"), false));
        assert!(is_runnable(&plugin("project"), true));
        // 全局插件是用户自己装的，视为已安装
        assert!(is_runnable(&plugin("global"), false));
    }

    #[test]
    fn untrusted_project_plugin_is_refused() {
        let err = run(Path::new("/tmp/proj"), &plugin("project"), false, "", &[]).unwrap_err();
        assert!(err.contains("未被信任"), "{err}");
    }

    #[test]
    fn entrypoint_must_exist() {
        let outcome = resolve_entrypoint(Path::new("/tmp/nonexistent-plugin-dir"), "nope");
        assert!(outcome.is_none());
    }
}
