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
mod web;

use std::process::Command;

fn main() {
    let mut port = 8765u16;
    let mut open_browser = true;

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
            "--help" | "-h" => {
                println!("用法: cercis-app [--port 8765] [--no-browser]");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let addr = format!("127.0.0.1:{port}");

    if open_browser {
        let url = format!("http://{addr}");
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

    if let Err(e) = http::serve(&addr, handler) {
        eprintln!("启动失败: {e}");
        std::process::exit(1);
    }
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
