//! 极简 HTTP/1.1 服务器（仅监听本机，仅供本地界面使用）。
//!
//! 设计取舍：不引入 Web 框架，避免额外依赖与升级负担；只实现界面所需的最小功能。
//!
//! 安全：
//! - 只绑定 127.0.0.1（不对外网开放）
//! - 校验 Host 头（防 DNS rebinding）
//! - 不提供静态文件服务（页面为内置字符串），避免目录穿越

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

/// 请求。
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub body: String,
    /// `X-Cercis-Token` 头（LAN 模式下 app 的界面会带上）
    pub header_token: Option<String>,
}

/// 响应。
pub struct Response {
    pub status: u16,
    pub content_type: String,
    /// 正文用字节：界面本身是文本，但"录入员"页要放原音/原图（二进制）
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "application/json; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn html(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
        }
    }

    /// 自定义类型（图标 PNG、webmanifest…）。
    pub fn bytes(content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: content_type.into(),
            body,
        }
    }

    pub fn error(status: u16, message: &str) -> Self {
        let body = serde_json::json!({
            "ok": false,
            "error": message,
        })
        .to_string();
        Self {
            status,
            content_type: "application/json; charset=utf-8".into(),
            body: body.into_bytes(),
        }
    }
}

/// 访问闸门（S14：平板/手机等**别的设备**通过局域网访问时用）。
///
/// - `local_only`（默认）：只绑本机 + 校验 Host，防 DNS rebinding；
/// - `lan(token)`：绑局域网，**每个请求都要带令牌**（启动时打印的 URL 里含令牌）。
///   局域网里没有令牌连不上，但**同网段的人能扫到端口**——所以只在可信网络里开。
#[derive(Clone, Debug)]
pub struct Guard {
    /// 只允许本机（默认）
    pub local_only: bool,
    /// LAN 模式的令牌（`None` = 不校验）
    pub token: Option<String>,
}

impl Guard {
    pub fn local_only() -> Self {
        Self {
            local_only: true,
            token: None,
        }
    }

    pub fn lan(token: impl Into<String>) -> Self {
        Self {
            local_only: false,
            token: Some(token.into()),
        }
    }

    /// 这个请求放行吗？（带原因，方便调试与测试）
    pub fn check(&self, req: &Request, host_header: Option<&str>) -> Result<(), (u16, String)> {
        // 1) 本机模式：只认本机 Host（防 DNS rebinding）
        if self.local_only {
            if let Some(host) = host_header {
                let host_only = host.split(':').next().unwrap_or("");
                if !matches!(host_only, "127.0.0.1" | "localhost" | "[::1]") {
                    return Err((403, "只允许本机访问（Host 校验失败）".to_string()));
                }
            }
        }
        // 2) 令牌：LAN 与"带令牌的本机模式"都要对
        if let Some(expected) = &self.token {
            // 例外：图标这类**不含任何数据**的静态资源。浏览器取 manifest 里列出的图标时
            // 既不带查询串也不带自定义头，如果也要求令牌，"添加到主屏幕"就会变成一个没图标的方块。
            // 令牌保护的始终是**页面与数据**，不是应用图标。
            if public_asset(&req.path) {
                return Ok(());
            }
            let given = req
                .query
                .get("token")
                .cloned()
                .or_else(|| req.header_token.clone())
                .unwrap_or_default();
            if given != *expected {
                return Err((
                    401,
                    "令牌不对（LAN 模式要带上启动时打印的那个 token）".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// 不需要令牌的静态资源（图标；理由见 `Guard::check`）。
pub fn public_asset(path: &str) -> bool {
    matches!(
        path,
        "/favicon.ico"
            | "/icon-64.png"
            | "/icon-192.png"
            | "/icon-maskable-192.png"
            | "/apple-touch-icon.png"
    )
}

/// 带闸门的启动。
pub fn serve_with<F>(addr: &str, guard: Guard, handler: F) -> std::io::Result<()>
where
    F: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let listener = TcpListener::bind(addr)?;
    println!("Cercis 界面已启动：http://{addr}");
    if guard.local_only {
        println!("（仅本机可访问；按 Ctrl+C 退出）");
    } else {
        println!("（局域网模式：要带 ?token=… 才能访问；只在你信任的网络里开）");
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream, &guard, &handler) {
                    eprintln!("请求处理失败: {e}");
                }
            }
            Err(e) => eprintln!("连接失败: {e}"),
        }
    }
    Ok(())
}

fn handle_connection<F>(stream: TcpStream, guard: &Guard, handler: &F) -> std::io::Result<()>
where
    F: Fn(&Request) -> Response,
{
    let mut stream = stream;
    let mut reader = BufReader::new(stream.try_clone()?);

    // 请求行
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    // 头
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        let h = h.trim();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    // 正文
    let mut body = String::new();
    if let Some(len) = headers.get("content-length") {
        if let Ok(n) = len.parse::<usize>() {
            let mut buf = vec![0u8; n.min(8 * 1024 * 1024)];
            reader.read_exact(&mut buf)?;
            body = String::from_utf8_lossy(&buf).to_string();
        }
    }

    // 拆分路径与查询串
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_query(q)),
        None => (target.clone(), BTreeMap::new()),
    };

    let req = Request {
        method,
        path,
        query,
        body,
        header_token: headers.get("x-cercis-token").cloned(),
    };
    // 闸门：Host 校验（本机模式）+ 令牌（LAN 模式）
    if let Err((status, msg)) = guard.check(&req, headers.get("host").map(|s| s.as_str())) {
        let resp = Response::error(status, &msg);
        write_response(&mut stream, &resp)?;
        return Ok(());
    }
    let resp = handler(&req);
    write_response(&mut stream, &resp)?;
    Ok(())
}

fn parse_query(q: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        out.insert(url_decode(k), url_decode(v));
    }
    out
}

/// 读项目里的媒体文件（给界面的原音/原图用）。
///
/// 安全：只允许项目内的 `_labnote/scribe/` 与 `media/` 两个位置，且路径里不许有 `..`，
/// 免得这个本地端口变成"读任意文件"的通道。
pub fn read_project_media(root: &std::path::Path, file: &str) -> Result<(Vec<u8>, String), String> {
    let rel = file.trim_start_matches('/');
    if rel.contains("..") || rel.starts_with('/') {
        return Err("路径不合法".into());
    }
    let ok_prefix = rel.starts_with("_labnote/scribe/") || rel.starts_with("media/");
    if !ok_prefix {
        return Err("只允许读项目里的 _labnote/scribe 与 media".into());
    }
    let path = root.join(rel);
    let bytes = std::fs::read(&path).map_err(|e| format!("读不了 {}：{e}", path.display()))?;
    let ctype = match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    };
    Ok((bytes, ctype.to_string()))
}

/// 极简 URL 解码（处理 %XX 与 +）。
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn write_response(stream: &mut TcpStream, resp: &Response) -> std::io::Result<()> {
    // 图标可以放心缓存（内容固定、不含数据）；页面与接口一律不缓存（数据必须新鲜）。
    let cache = if resp.content_type.starts_with("image/png") {
        "public, max-age=86400"
    } else {
        "no-store"
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        status_text(resp.status),
        resp.content_type,
        resp.body.len(),
        cache
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&resp.body)?;
    stream.flush()
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with(path: &str, query: &[(&str, &str)], token: Option<&str>) -> Request {
        Request {
            method: "GET".into(),
            path: path.into(),
            query: query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: String::new(),
            header_token: token.map(|s| s.to_string()),
        }
    }

    #[test]
    fn local_mode_rejects_foreign_hosts() {
        let g = Guard::local_only();
        assert!(g
            .check(&req_with("/", &[], None), Some("127.0.0.1:8765"))
            .is_ok());
        assert!(g
            .check(&req_with("/", &[], None), Some("localhost:8765"))
            .is_ok());
        let err = g
            .check(&req_with("/", &[], None), Some("evil.example:8765"))
            .unwrap_err();
        assert_eq!(err.0, 403);
    }

    #[test]
    fn lan_mode_requires_the_token() {
        let g = Guard::lan("s3cret");
        // 局域网里 Host 是内网 IP，不拦（靠令牌）
        assert!(g
            .check(&req_with("/", &[], None), Some("192.168.1.5:8765"))
            .is_err());
        assert!(g
            .check(
                &req_with("/", &[("token", "wrong")], None),
                Some("192.168.1.5")
            )
            .is_err());
        assert!(g
            .check(
                &req_with("/", &[("token", "s3cret")], None),
                Some("192.168.1.5")
            )
            .is_ok());
        // 也可以走请求头（给脚本用）
        assert!(g
            .check(&req_with("/", &[], Some("s3cret")), Some("192.168.1.5"))
            .is_ok());
    }

    #[test]
    fn token_can_also_guard_local_mode() {
        let g = Guard {
            local_only: true,
            token: Some("t".into()),
        };
        assert!(g.check(&req_with("/", &[], None), Some("127.0.0.1")).is_err());
        assert!(g
            .check(&req_with("/", &[("token", "t")], None), Some("127.0.0.1"))
            .is_ok());
        // 本机模式下 Host 仍然要校验
        assert!(g
            .check(&req_with("/", &[("token", "t")], None), Some("evil.example"))
            .is_err());
    }

    #[test]
    fn icons_do_not_need_the_token_but_pages_and_api_do() {
        let g = Guard::lan("s3cret");
        let host = Some("192.168.1.5:8765");
        // 图标：浏览器取 manifest 里的图标时不会带令牌，必须放行
        for p in ["/favicon.ico", "/icon-64.png", "/icon-192.png", "/apple-touch-icon.png"] {
            assert!(g.check(&req_with(p, &[], None), host).is_ok(), "{p} 应当放行");
        }
        // 页面与数据接口：仍然要令牌
        for p in ["/", "/api/open", "/api/app-manifest"] {
            assert!(g.check(&req_with(p, &[], None), host).is_err(), "{p} 不该放行");
        }
        // 本机模式下 Host 校验对图标一样有效（防 DNS rebinding）
        assert!(Guard::local_only()
            .check(&req_with("/icon-192.png", &[], None), Some("evil.example"))
            .is_err());
    }
}
