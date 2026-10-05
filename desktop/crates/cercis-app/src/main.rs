//! Cercis 桌面界面（本地网页界面）。
//!
//! 用法：
//! ```bash
//! cercis-app                 # 默认 http://127.0.0.1:8765
//! cercis-app --port 9000     # 指定端口
//! cercis-app --no-browser    # 不自动打开浏览器
//! ```
//!
//! 设计：只监听本机；所有数据操作经内核；页面为内置字符串（不读磁盘文件）。

mod api;
mod http;
mod market_host;
mod plugin_host;
mod web;

use std::process::Command;

fn main() {
    let mut port = 8765u16;
    let mut open_browser = true;
    let mut lan = false;
    let mut token: Option<String> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<u16>().ok()) {
                    port = v;
                    i += 1;
                }
            }
            "--no-browser" => open_browser = false,
            // 局域网模式：平板/手机在同一个网络里用浏览器打开（带令牌）
            "--lan" => lan = true,
            "--token" => {
                if let Some(v) = args.get(i + 1) {
                    token = Some(v.clone());
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!("用法: cercis-app [--port 8765] [--no-browser] [--lan] [--token <令牌>]");
                println!();
                println!("  默认只绑本机（127.0.0.1）。");
                println!("  --lan：绑局域网，平板/手机也能用浏览器打开（必须带令牌；");
                println!("         URL 会在启动时打印，例如 http://192.168.1.5:8765/?token=abcd）。");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let addr = if lan {
        format!("0.0.0.0:{port}")
    } else {
        format!("127.0.0.1:{port}")
    };
    // LAN 模式必须有令牌：没给就随机生成一个（16 位十六进制）
    let guard = if lan {
        let t = token.clone().unwrap_or_else(|| {
            let bytes = cercis_core::crypto::random_bytes(8);
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        });
        token = Some(t.clone());
        http::Guard::lan(t)
    } else if let Some(t) = token.clone() {
        // 本机模式也可以要令牌（脚本/自动化里有用）
        http::Guard {
            local_only: true,
            token: Some(t),
        }
    } else {
        http::Guard::local_only()
    };

    let local_ip = if lan { lan_ip() } else { None };
    let url = match (&local_ip, &token) {
        (Some(ip), Some(t)) => format!("http://{ip}:{port}/?token={t}"),
        _ => format!("http://127.0.0.1:{port}"),
    };

    if open_browser {
        let url = url.clone();
        // 尽力而为地打开浏览器（失败不影响服务）
        let _ = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(350));
            let _ = opener(&url);
        });
    }

    println!(
        "Cercis v{}（内核 v{}）",
        env!("CARGO_PKG_VERSION"),
        cercis_core::CORE_VERSION
    );
    println!("愿瑟希斯捍卫你的思想。 / May Cerces safeguard your thoughts.");
    if lan {
        println!();
        println!("局域网模式（平板/手机用这个地址打开）：");
        println!("  {url}");
        println!("  令牌：{}", token.clone().unwrap_or_default());
        println!("  提醒：只在可信网络里开；别把带令牌的链接发给别人。");
    }
    println!();

    let handler = |req: &http::Request| -> http::Response {
        if req.path == "/" || req.path == "/index.html" {
            http::Response::html(web::INDEX_HTML)
        } else if req.path.starts_with("/api/") {
            api::handle(req)
        } else {
            http::Response::error(404, "not found")
        }
    };

    if let Err(e) = http::serve_with(&addr, guard, handler) {
        eprintln!("启动失败: {e}");
        std::process::exit(1);
    }
}

/// 猜一个局域网 IP（给平板/手机用的地址）：挑第一个非回环的私网地址。
fn lan_ip() -> Option<String> {
    let out = Command::new("sh")
        .args(["-c", "ifconfig 2>/dev/null || ip -4 addr 2>/dev/null"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    for line in text.lines() {
        let line = line.trim();
        // macOS/Linux: inet 192.168.x.x / inet 10.x.x.x
        if let Some(rest) = line.split_whitespace().collect::<Vec<_>>().windows(2).find_map(|w| {
            if w[0] == "inet" {
                Some(w[1])
            } else {
                None
            }
        }) {
            let ip = rest.split('/').next().unwrap_or(rest);
            if ip.starts_with("127.") {
                continue;
            }
            if ip.starts_with("192.168.")
                || ip.starts_with("10.")
                || ip.starts_with("172.16.")
                || ip.starts_with("172.17.")
                || ip.starts_with("172.18.")
                || ip.starts_with("172.19.")
                || ip.starts_with("172.2")
                || ip.starts_with("172.30.")
                || ip.starts_with("172.31.")
            {
                return Some(ip.to_string());
            }
        }
    }
    None
}

/// 打开系统默认浏览器（macOS / Linux / Windows）。
fn opener(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", url]);
        c
    };
    cmd.status().map(|_| ())
}
