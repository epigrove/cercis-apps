//! 插件市场的 **app 段**：看清单、下载、把结果交给内核那一段。
//!
//! 分工（需求文档 §14.8 / D41–D43）：
//! - ① 清单：git 仓库里的 `marketplace.json`（预置基础包与科研包，都是公开仓库）
//! - ② **下载 / 更新检查：app（本模块）**——内核不联网
//! - ③ 装卸与屏蔽的界面：本 app 的"插件市场"面板
//! - ④ 校验、安装落盘、来源与指纹记账：内核（`cercis_core::market`），app 只是调它
//!
//! 下载用 `git clone --depth 1 --branch <ref>`（公开仓库直接来；私有仓库走用户已配置的
//! 凭据助手，凭据不经过本程序）。没有 git 的环境走**手动路径**：把仓库下载下来，
//! 用"从本地目录安装"（对应 FR-P48 的"拖入插件包"）——那时的错误信息会直接告诉用户怎么做。

use std::path::{Path, PathBuf};
use std::process::Command;

use cercis_core::market::{InstallRecord, Marketplace};

/// 下载缓存目录：`~/.cercis/cache/markets/<市场 id>`。
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CERCIS_CACHE_DIR") {
        return Some(PathBuf::from(d).join("markets"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".cercis").join("cache").join("markets"))
}

/// git 可执行文件（默认 PATH 里的 `git`；可用 `CERCIS_GIT_BIN` 指定）。
fn git_bin() -> String {
    std::env::var("CERCIS_GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

/// 下载（或更新）一个市场到缓存目录，返回本地目录。
///
/// 已经克隆过的：`git fetch --depth 1 origin <ref>` + `git reset --hard FETCH_HEAD`，
/// 这样"刷新"不会重新下整个仓库历史。
pub fn download(repo: &str, git_ref: &str, market_id: &str) -> Result<PathBuf, String> {
    let dir = cache_dir()
        .ok_or_else(|| "取不到 HOME，无法定位下载缓存目录".to_string())?
        .join(sanitize(market_id));
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建缓存目录失败：{e}"))?;
    }
    let git_ref = if git_ref.trim().is_empty() {
        "main"
    } else {
        git_ref.trim()
    };

    if dir.join(".git").is_dir() {
        run_git(&dir, &["fetch", "--depth", "1", "origin", git_ref])?;
        run_git(&dir, &["reset", "--hard", "FETCH_HEAD"])?;
    } else {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("清理旧缓存失败：{e}"))?;
        }
        let dest = dir.display().to_string();
        run_git(
            Path::new("."),
            &["clone", "--depth", "1", "--branch", git_ref, repo, &dest],
        )?;
    }
    if !dir.join("marketplace.json").is_file() && !dir.join("marketplace.yaml").is_file() {
        return Err(format!(
            "仓库下载好了（{}），但里面没有 marketplace.json——它可能不是一个插件市场",
            dir.display()
        ));
    }
    Ok(dir)
}

/// 读缓存里的市场清单。
pub fn load_listing(dir: &Path) -> Result<Marketplace, String> {
    let path = if dir.join("marketplace.json").is_file() {
        dir.join("marketplace.json")
    } else {
        dir.join("marketplace.yaml")
    };
    Marketplace::load(&path).map_err(|e| e.to_string())
}

/// 缓存目录里某个市场的清单（没下载过就是 None）。
pub fn cached_listing(market_id: &str) -> Option<(PathBuf, Marketplace)> {
    let dir = cache_dir()?.join(sanitize(market_id));
    if !dir.is_dir() {
        return None;
    }
    load_listing(&dir).ok().map(|m| (dir, m))
}

/// 从**已经下载好的**目录安装一个插件（内核那一段干活，app 只转交）。
pub fn install_from(
    dir: &Path,
    plugin: &str,
    market_id: &str,
    git_ref: &str,
) -> Result<(InstallRecord, PathBuf), String> {
    let listing = load_listing(dir)?;
    let entry = listing.plugin(plugin).ok_or_else(|| {
        format!(
            "市场 {} 里没有插件 {plugin}（可用：{}）",
            listing.summary(),
            listing
                .plugins
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        )
    })?;
    let src = resolve_plugin_dir(dir, &entry.path)?;
    let global = crate::plugin_host::global_plugins_dir()
        .ok_or_else(|| "取不到用户插件目录（~/.cercis/plugins）".to_string())?;
    let repo = if listing.repository.is_empty() {
        dir.display().to_string()
    } else {
        listing.repository.clone()
    };
    cercis_core::market::install_from_dir(&src, &global, market_id, &repo, git_ref, listing.bundled)
        .map_err(|e| e.to_string())
}

/// 删除插件（市场装的能删；随包的基础插件会被内核拒绝并给出停用/屏蔽的指引）。
pub fn uninstall(id: &str, note: &str) -> Result<String, String> {
    let global = crate::plugin_host::global_plugins_dir()
        .ok_or_else(|| "取不到用户插件目录（~/.cercis/plugins）".to_string())?;
    cercis_core::market::uninstall(&global, id, false, note)
        .map(|(_, msg)| msg)
        .map_err(|e| e.to_string())
}

/// 找回被删掉的插件：`from` 给了本地目录就顺手装回来。
pub fn restore(id: &str, from: Option<&Path>) -> Result<String, String> {
    let global = crate::plugin_host::global_plugins_dir()
        .ok_or_else(|| "取不到用户插件目录（~/.cercis/plugins）".to_string())?;
    let (resolved, bundled, repo, market, git_ref) = match from {
        Some(dir) => {
            let listing = load_listing(dir)?;
            let entry = listing.plugin(id).ok_or_else(|| {
                format!(
                    "市场 {} 里没有插件 {id}（可用：{}）",
                    listing.summary(),
                    listing
                        .plugins
                        .iter()
                        .map(|p| p.id.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            })?;
            let src = resolve_plugin_dir(dir, &entry.path)?;
            (
                Some(src),
                listing.bundled,
                listing.repository.clone(),
                String::new(),
                String::new(),
            )
        }
        None => (None, false, String::new(), String::new(), String::new()),
    };
    cercis_core::market::restore(
        &global,
        id,
        resolved.as_deref(),
        &repo,
        &market,
        &git_ref,
        bundled,
    )
    .map(|(_, msg, _)| msg)
    .map_err(|e| e.to_string())
}

/// 安装台账（"从哪装的、装的哪一版"）。
pub fn installed() -> Vec<InstallRecord> {
    crate::plugin_host::global_plugins_dir()
        .and_then(|d| cercis_core::Installs::load(&d).ok())
        .map(|i| i.records)
        .unwrap_or_default()
}

/// 删除名单（"用户删过、升级不许复活"）。
pub fn removed() -> Vec<cercis_core::market::Removal> {
    crate::plugin_host::global_plugins_dir()
        .and_then(|d| cercis_core::market::Removals::load(&d).ok())
        .map(|r| r.removed)
        .unwrap_or_default()
}

/// 这台机器上有没有 git（界面据此决定"下载"按钮是否可用）。
pub fn has_git() -> bool {
    Command::new(git_bin())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 跑一条 git 命令；失败时把 stderr 带出来（含"没装 git"的指路）。
fn run_git(cwd: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new(git_bin())
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| {
            format!(
                "跑不起来 git（{e}）——这个环境没有 git。两条替代路：\
                 ① 手动把仓库下载下来（网页上的 Download ZIP 也行），再用「从本地目录安装」；\
                 ② 命令行 `cercis market install --from <目录> --plugin <id>`"
            )
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!(
            "git {} 失败：{}（私有仓库请先在系统里配好凭据；本程序不保存任何凭据）",
            args.first().copied().unwrap_or(""),
            if err.is_empty() { "原因不明" } else { &err }
        ));
    }
    Ok(())
}

/// 按清单里写的路径找插件目录；找不到时把"仓库里实际有什么"列出来（手写市场很容易写错）。
fn resolve_plugin_dir(repo_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let candidate = repo_dir.join(rel);
    if candidate.is_dir() {
        return Ok(candidate);
    }
    // 容错：清单写 `plugins/x`，但仓库根就是 x（或反过来）
    let base = rel.rsplit('/').next().unwrap_or(rel);
    let alt = repo_dir.join(base);
    if alt.is_dir() {
        return Ok(alt);
    }
    let listing: Vec<String> = std::fs::read_dir(repo_dir)
        .map(|it| {
            it.flatten()
                .filter(|e| e.path().is_dir() && e.file_name() != ".git")
                .map(|e| format!("{}/", e.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();
    Err(format!(
        "清单里写的插件路径是 {rel}，但仓库里没有这个目录（仓库根下有：{}）。         手写 marketplace.json 时最容易错的就是这一项——改成实际路径即可",
        if listing.is_empty() {
            "（空）".to_string()
        } else {
            listing.join(" ")
        }
    ))
}

/// 市场 id 用于目录名时的清洗（不信任外部输入）。
fn sanitize(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if cleaned.is_empty() {
        "market".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cercis-app-market-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 造一个"已经下载好的"市场目录（含清单与插件目录）。
    fn fake_repo(dir: &Path) -> PathBuf {
        let p = dir.join("plugins/demo_io");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("plugin.yaml"),
            "id: demo_io\nnamespace: demo_io\nname: Demo\nversion: 0.1.0\ntier: third-party\nrequires_plugin_api: 1\npermissions:\n  read: true\nui:\n  - kind: command\n    id: demo_io.ping\n    label_cn: 打招呼\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("marketplace.json"),
            r#"{"schema": "cercis.marketplace/1", "name": "演示市场",
                 "repository": "https://github.com/example/demo",
                 "plugins": [{"id": "demo_io", "name": "Demo", "path": "plugins/demo_io",
                               "version": "0.1.0", "tier": "third-party", "requires_plugin_api": 1}]}"#,
        )
        .unwrap();
        dir.to_path_buf()
    }

    #[test]
    fn loads_listing_from_a_downloaded_dir() {
        let dir = tmp("listing");
        fake_repo(&dir);
        let m = load_listing(&dir).unwrap();
        assert_eq!(m.plugins.len(), 1);
        assert!(m.plugin("demo_io").is_some());
    }

    #[test]
    fn install_from_local_dir_lands_in_global_dir_and_records_source() {
        let dir = tmp("install");
        fake_repo(&dir);
        let global = dir.join("global");
        std::fs::create_dir_all(&global).unwrap();
        std::env::set_var("CERCIS_GLOBAL_PLUGINS", &global);

        let (record, dest) = install_from(&dir, "demo_io", "演示市场", "main").unwrap();
        assert_eq!(record.id, "demo_io");
        assert!(dest.join("plugin.yaml").is_file());
        assert_eq!(record.repository, "https://github.com/example/demo");
        let listed = installed();
        assert!(listed.iter().any(|r| r.id == "demo_io"));

        // 市场里没有的插件：错误要说清
        let err = install_from(&dir, "不存在", "演示市场", "main").unwrap_err();
        assert!(err.contains("demo_io"), "{err}");
        std::env::remove_var("CERCIS_GLOBAL_PLUGINS");
    }

    #[test]
    fn missing_git_is_reported_with_a_manual_path() {
        // 指向一个不存在的 git（不动 PATH，避免影响并行跑的其它测试）
        let dir = tmp("no-git");
        std::env::set_var("CERCIS_GIT_BIN", "/nonexistent/git");
        std::env::set_var("CERCIS_CACHE_DIR", dir.join("cache"));
        let err = download("https://github.com/example/demo", "main", "demo").unwrap_err();
        assert!(
            err.contains("没有 git") && err.contains("从本地目录安装"),
            "{err}"
        );
        std::env::remove_var("CERCIS_GIT_BIN");
        std::env::remove_var("CERCIS_CACHE_DIR");
    }

    #[test]
    fn market_id_is_sanitized_for_directory_names() {
        assert_eq!(sanitize("team/market"), "team-market");
        assert_eq!(sanitize("../../etc"), "------etc");
        assert_eq!(sanitize(""), "market");
    }
}
