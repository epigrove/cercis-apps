//! 内置网页资源（编译进二进制；不读磁盘文件，避免路径问题）。

/// 主界面 HTML。
pub const INDEX_HTML: &str = include_str!("index.html");
