//! 随包自带的基础工具包：**首次启动时装进用户级插件目录**。
//!
//! 需求 D35：`Cercis app` = 内核 + 基础工具包；FR-P44–P50：所有插件都能装卸、屏蔽。
//! 这里解决的是"装完就没有插件"的问题——安装包里带着基础插件的可执行文件，
//! 但 app 自己不含插件代码，插件是**独立可执行文件**，要落到用户级插件目录才找得到。
//!
//! 为什么在 app 里装而不是让安装脚本装：
//! - 装机不需要管理员权限，也不用管用户是不是刚换过机器；
//! - **走内核自己的安装通道**（`market::install_from_dir`）：写安装台账、记内容指纹、
//!   标成 `bundled`（随包插件不可删除，只能停用/屏蔽），和从市场装插件的账一模一样；
//! - macOS 的 `.app` 里插件放在 `Contents/Resources/kits/basic`（不在用户主目录里），
//!   安装包解压即用，第一次打开就把基础工具包装好。

use std::path::{Path, PathBuf};

use cercis_core::market::{install_from_dir, Installs, Marketplace};
use cercis_core::plugin::PluginManifest;

/// 随包基础工具包在磁盘上的位置（找不到就返回 None，静默跳过——开发时用脚本装）。
///
/// 顺序：`CERCIS_BUNDLED_KITS` → 可执行文件同级 `kits/basic` → macOS `.app` 的
/// `Contents/Resources/kits/basic`。
pub fn bundled_kits_dir(exe_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CERCIS_BUNDLED_KITS") {
        let p = PathBuf::from(p);
        if p.join("marketplace.json").is_file() {
            return Some(p);
        }
    }
    let candidates = [
        exe_dir.join("kits").join("basic"),
        // macOS：Cercis.app/Contents/MacOS/cercis-app → ../Resources/kits/basic
        exe_dir.join("..").join("Resources").join("kits").join("basic"),
    ];
    candidates
        .into_iter()
        .find(|c| c.join("marketplace.json").is_file())
}

/// 装/更新随包的基础插件；返回（新装的, 跳过的）插件 id。
///
/// 版本一致就不动它（别每次启动都重写一遍用户目录）；版本不同则更新。
pub fn install_bundled_kits(
    kits_dir: &Path,
    global_dir: &Path,
) -> Result<(Vec<String>, Vec<String>), String> {
    let listing = Marketplace::load(kits_dir.join("marketplace.json"))
        .map_err(|e| format!("读不了随包市场清单：{e}"))?;
    // 首次装机时用户插件目录可能还不存在
    std::fs::create_dir_all(global_dir).map_err(|e| format!("建不了插件目录：{e}"))?;
    let installs = Installs::load(global_dir).map_err(|e| format!("读不了安装台账：{e}"))?;

    let mut installed = Vec::new();
    let mut skipped = Vec::new();
    for plugin in &listing.plugins {
        let source = kits_dir.join(&plugin.path);
        if !source.join("plugin.yaml").is_file() {
            // 打包时漏了某个插件：说清楚，但别拦着 app 启动
            eprintln!(
                "随包插件 {} 不完整（缺 {}），跳过",
                plugin.id,
                source.display()
            );
            continue;
        }
        // 比的是**包里这个插件自己的版本**（清单里的版本号是市场的说法，插件清单才是实情）
        let bundled_version = PluginManifest::load(source.join("plugin.yaml"))
            .map(|m| m.version)
            .unwrap_or_else(|_| plugin.version.clone());
        if let Some(rec) = installs.get(&plugin.id) {
            if rec.version == bundled_version && global_dir.join(&rec.path).is_dir() {
                skipped.push(plugin.id.clone());
                continue;
            }
        }
        install_from_dir(
            &source,
            global_dir,
            "basic",
            &listing.repository,
            "bundled",
            listing.bundled,
        )
        .map_err(|e| format!("装随包插件 {} 失败：{e}", plugin.id))?;
        installed.push(plugin.id.clone());
    }
    Ok((installed, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个"随包基础工具包"的目录形状：marketplace.json + plugins/<id>/（清单 + 可执行文件占位）。
    fn fake_kits(root: &Path) -> PathBuf {
        let kits = root.join("kits").join("basic");
        std::fs::create_dir_all(kits.join("plugins").join("demo")).unwrap();
        std::fs::write(
            kits.join("marketplace.json"),
            r#"{
              "schema": "cercis.marketplace/1",
              "name": "基础工具包",
              "bundled": true,
              "repository": "https://github.com/epigrove/cercis-kits-basic",
              "plugins": [
                { "id": "demo", "name": "演示", "path": "plugins/demo", "version": "0.1.0",
                  "tier": "basic", "requires_plugin_api": 1, "summary": "test" }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            kits.join("plugins").join("demo").join("plugin.yaml"),
            "id: demo\nname: Demo\nversion: 0.1.0\nentrypoint: cercis-plugin-demo\n\
             description: 测试用\n",
        )
        .unwrap();
        std::fs::write(
            kits.join("plugins").join("demo").join("cercis-plugin-demo"),
            b"#!/bin/sh\nexit 0\n",
        )
        .unwrap();
        kits
    }

    #[test]
    fn bundled_kits_are_installed_once_and_recorded() {
        let root = std::env::temp_dir().join("cercis-bundled-test");
        let _ = std::fs::remove_dir_all(&root);
        let kits = fake_kits(&root);
        let global = root.join("plugins-dir");

        // 第一次：装上，并且进台账（bundled → 不可删除）
        let (installed, skipped) = install_bundled_kits(&kits, &global).unwrap();
        assert_eq!(installed, vec!["demo"]);
        assert!(skipped.is_empty());
        assert!(global.join("demo").join("plugin.yaml").is_file());
        let rec = Installs::load(&global).unwrap().get("demo").cloned().unwrap();
        assert!(rec.bundled, "随包插件要标 bundled（不可删除）");
        assert_eq!(rec.market, "basic");
        assert_eq!(rec.version, "0.1.0");
        assert!(!rec.fingerprint.is_empty(), "要记内容指纹");

        // 第二次：版本没变 → 一个都不动（别每次启动重写用户目录）
        let (installed2, skipped2) = install_bundled_kits(&kits, &global).unwrap();
        assert!(installed2.is_empty());
        assert_eq!(skipped2, vec!["demo"]);

        // 版本变了 → 更新（随包插件就是这样随 app 升级的）
        let manifest = kits.join("plugins").join("demo").join("plugin.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap().replace("0.1.0", "0.2.0");
        std::fs::write(&manifest, text).unwrap();
        let (installed3, _) = install_bundled_kits(&kits, &global).unwrap();
        assert_eq!(installed3, vec!["demo"], "版本不同要更新");
        assert_eq!(
            Installs::load(&global).unwrap().get("demo").unwrap().version,
            "0.2.0"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_kits_are_skipped_not_fatal() {
        let root = std::env::temp_dir().join("cercis-bundled-missing-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // 没有 kits 目录 → 找不到（开发时的正常情形：用脚本装）
        assert!(bundled_kits_dir(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
