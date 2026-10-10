//! 应用图标：在**进程内按需生成** PNG（不往仓库里放二进制）。
//!
//! 为什么要自己生成：
//! - 平板/手机"添加到主屏幕"要真图标，而 `apple-touch-icon` **不认 SVG**，必须是 PNG；
//! - 仓库里不放二进制图片（发布扫描对"不认识的二进制"一律保守，见 `docs/发布前扫描-*.md`）；
//! - 于是手写一个**最小 PNG 编码器**：zlib 只写"存储块"（不压缩）+ 标准 CRC32。
//!   零依赖、不动构建配置，测试里能逐项验证（签名 / IHDR / CRC / 像素落点）。
//!
//! 图案：品牌色圆角方块 + 白色开口圆环（"C"）。`maskable` 版本满幅出血、图案缩到安全区。
//!
//! 体积上的取舍：不压缩意味着**像素多少 = 字节多少**（比压缩大几十倍），所以这里只生成
//! 各平台真正需要的尺寸（页签 64 / 主屏与清单 192 / iOS 180）。浏览器把图标缓存一天，
//! 一次安装只拉一两百 KB。要更小就得写一个真的 deflate——不值得，先别做。
//!
//! 换成自己的图标：`cercis-app --logo 我的图标.png`（见 `IconSource`）。

use std::path::Path;

/// 生成图标 PNG（圆角方块，用于 favicon / apple-touch-icon / manifest 的 `any`）。
pub fn png(size: u32) -> Vec<u8> {
    render(size, true, 1.0)
}

/// 生成 `maskable` 图标（满幅出血，供 Android 自适应裁切；图案落在中心安全区内）。
pub fn png_maskable(size: u32) -> Vec<u8> {
    render(size, false, 0.85)
}

/// 图标来源：内置生成（默认）或作者自带的 PNG。
///
/// 为什么只收 PNG：一个文件要同时喂给**页签图标、iOS 主屏图标、Android 清单**，
/// 只有 PNG 是三处都认的格式（SVG 在 `apple-touch-icon` 上无效）。各尺寸共用同一张图，
/// 由浏览器自己缩放——想要逐尺寸精修的话，用内置生成的那套就够。
#[derive(Clone, Debug)]
pub enum IconSource {
    BuiltIn,
    Custom(Vec<u8>),
}

impl IconSource {
    /// 从文件读自定义图标（必须是 `.png`）。
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "png" {
            return Err(format!(
                "图标只支持 PNG（{} 是 .{}；先转成 PNG 再试）",
                path.display(),
                if ext.is_empty() { "无扩展名" } else { &ext }
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("读不了图标 {}：{e}", path.display()))?;
        if bytes.len() < 8 || bytes[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
            return Err(format!("{} 不是有效的 PNG 文件", path.display()));
        }
        Ok(Self::Custom(bytes))
    }

    /// 取某个尺寸的图标字节。
    pub fn bytes(&self, size: u32, maskable: bool) -> Vec<u8> {
        match self {
            Self::Custom(b) => b.clone(),
            Self::BuiltIn if maskable => png_maskable(size),
            Self::BuiltIn => png(size),
        }
    }
}

/// 打包脚本用：把一整套尺寸的图标写进目录（macOS 的 .icns、Windows 的 .ico、
/// Linux 的 hicolor 主题、以及网页那几张，都从这里取）。
pub fn write_all(dir: &Path, src: &IconSource) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for size in [16u32, 32, 64, 128, 180, 192, 256, 512, 1024] {
        let name = format!("icon-{size}.png");
        std::fs::write(dir.join(&name), src.bytes(size, false))?;
        written.push(name);
    }
    for size in [192u32, 512] {
        let name = format!("icon-maskable-{size}.png");
        std::fs::write(dir.join(&name), src.bytes(size, true))?;
        written.push(name);
    }
    Ok(written)
}

// ---------------------------------------------------------------- 画图

fn render(size: u32, rounded: bool, mark_scale: f32) -> Vec<u8> {
    let s = size.clamp(16, 1024);
    let sf = s as f32;
    // 每像素 3×3 超采样，边缘就不会有锯齿
    const SS: u32 = 3;
    let total = (SS * SS) as f32;

    let mut raw: Vec<u8> = Vec::with_capacity((s * (1 + s * 4)) as usize);
    for y in 0..s {
        raw.push(0); // PNG 每行的过滤器字节：0 = None
        for x in 0..s {
            let (mut bg_cov, mut mark_cov) = (0f32, 0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if !inside_shape(px, py, sf, rounded) {
                        continue;
                    }
                    bg_cov += 1.0;
                    if inside_mark(px, py, sf, mark_scale) {
                        mark_cov += 1.0;
                    }
                }
            }
            let alpha = (bg_cov / total * 255.0).round() as u8;
            let ink = if bg_cov > 0.0 { mark_cov / bg_cov } else { 0.0 };
            let bg = shade((x as f32 + y as f32) / (2.0 * (sf - 1.0).max(1.0)));
            let [r, g, b] = blend(bg, [0xff, 0xf8, 0xfc], ink);
            raw.extend_from_slice(&[r, g, b, alpha]);
        }
    }
    encode_png(s, s, &raw)
}

/// 一个点是否落在图标形状内：圆角方块（`rounded`）或满幅（maskable）。
fn inside_shape(x: f32, y: f32, s: f32, rounded: bool) -> bool {
    if !rounded {
        return true;
    }
    let r = 0.20 * s; // 圆角半径
    // 到"内缩矩形"的距离 ≤ r，就是圆角方块的内部
    let nx = x.clamp(r, s - r);
    let ny = y.clamp(r, s - r);
    let (dx, dy) = (x - nx, y - ny);
    dx * dx + dy * dy <= r * r
}

/// 一个点是否落在白色圆环上（右侧留开口，看起来是个 "C"）。
fn inside_mark(x: f32, y: f32, s: f32, scale: f32) -> bool {
    let cx = s / 2.0;
    let cy = s / 2.0;
    // 除以 scale 就是"把图案缩小"（maskable 要留安全边，免得被系统裁掉）
    let dx = (x - cx) / s / scale;
    let dy = (y - cy) / s / scale;
    let d = (dx * dx + dy * dy).sqrt();
    let outer = 0.300;
    let inner = 0.185;
    if d < inner || d > outer {
        return false;
    }
    // 开口：右侧 ±55° 不画
    let mut ang = dy.atan2(dx).to_degrees();
    if ang < 0.0 {
        ang += 360.0;
    }
    !(ang < 55.0 || ang > 305.0)
}

/// 底色：对角线方向的深浅渐变（#8a4a7a → #5b2a4e，与界面强调色同族）。
fn shade(t: f32) -> [u8; 3] {
    let from = [0x8au8, 0x4a, 0x7a];
    let to = [0x5bu8, 0x2a, 0x4e];
    let k = t.clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for i in 0..3 {
        out[i] = (from[i] as f32 + (to[i] as f32 - from[i] as f32) * k).round() as u8;
    }
    out
}

fn blend(bg: [u8; 3], ink: [u8; 3], a: f32) -> [u8; 3] {
    let a = a.clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for i in 0..3 {
        out[i] = (bg[i] as f32 * (1.0 - a) + ink[i] as f32 * a).round() as u8;
    }
    out
}

// ---------------------------------------------------------------- 极简 PNG 编码

fn encode_png(w: u32, h: u32, raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 位/通道，RGBA，无隔行
    chunk(b"IHDR", &ihdr, &mut out);
    chunk(b"IDAT", &zlib_store(raw), &mut out);
    chunk(b"IEND", &[], &mut out);
    out
}

fn chunk(kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_in = Vec::with_capacity(4 + data.len());
    crc_in.extend_from_slice(kind);
    crc_in.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

/// zlib 流 = 2 字节头 + 若干"存储块"（不压缩）+ Adler-32。
fn zlib_store(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // CM=8 / CINFO=7 / 无预设字典；0x7801 % 31 == 0
    let mut i = 0;
    loop {
        let n = std::cmp::min(65535, raw.len() - i);
        let last = i + n >= raw.len();
        out.push(if last { 1 } else { 0 });
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(!(n as u16)).to_le_bytes());
        out.extend_from_slice(&raw[i..i + n]);
        i += n;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &byte in data {
        c ^= byte as u32;
        for _ in 0..8 {
            let mask = (c & 1).wrapping_neg();
            c = (c >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// 把 IDAT 的内容拼起来（我们只写一个，但别假设只有一个）。
    fn idat_of(png: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 8;
        while i + 8 <= png.len() {
            let len = be32(png, i) as usize;
            let kind = &png[i + 4..i + 8];
            if kind == b"IDAT" {
                out.extend_from_slice(&png[i + 8..i + 8 + len]);
            }
            i += 12 + len;
        }
        out
    }

    /// 取出解压后的原始像素（跳过 zlib 头与存储块头）。
    fn raw_pixels(png: &[u8]) -> Vec<u8> {
        let idat = idat_of(png);
        // zlib 头 2 字节 → 存储块头（BFINAL/BTYPE 1 字节 + LEN 2 + NLEN 2）→ 数据 → Adler-32
        assert_eq!(&idat[0..2], &[0x78, 0x01]);
        assert_eq!(idat[2] & 0b111, 0b001, "应当是最后一个存储块（BFINAL=1, BTYPE=00）");
        let len = u16::from_le_bytes([idat[3], idat[4]]) as usize;
        let nlen = u16::from_le_bytes([idat[5], idat[6]]);
        assert_eq!(nlen, !(len as u16));
        let raw = &idat[7..7 + len];
        assert_eq!(be32(&idat, idat.len() - 4), adler32(raw), "Adler-32 对不上");
        raw.to_vec()
    }

    #[test]
    fn png_structure_is_valid() {
        let png = png(192);
        assert_eq!(&png[0..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(be32(&png, 16), 192); // 宽
        assert_eq!(be32(&png, 20), 192); // 高
        assert_eq!(&png[24..29], &[8, 6, 0, 0, 0]);
        // 每个 chunk 的 CRC 都要对得上
        let mut i = 8;
        let mut seen = Vec::new();
        while i + 8 <= png.len() {
            let len = be32(&png, i) as usize;
            let kind = &png[i + 4..i + 8];
            let mut buf = Vec::from(kind);
            buf.extend_from_slice(&png[i + 8..i + 8 + len]);
            assert_eq!(crc32(&buf), be32(&png, i + 8 + len), "CRC 对不上");
            seen.push(String::from_utf8_lossy(kind).to_string());
            i += 12 + len;
        }
        assert_eq!(seen, vec!["IHDR", "IDAT", "IEND"]);
    }

    #[test]
    fn pixels_are_laid_out_row_by_row() {
        let size = 64u32;
        let raw = raw_pixels(&png(size));
        assert_eq!(raw.len(), (size * (1 + size * 4)) as usize);
        for y in 0..size as usize {
            assert_eq!(raw[y * (1 + size as usize * 4)], 0, "每行要以过滤器 0 开头");
        }
    }

    #[test]
    fn shape_and_mark_land_where_expected() {
        let size = 64u32;
        let raw = raw_pixels(&png(size));
        let stride = 1 + size as usize * 4;
        let px = |x: usize, y: usize| -> [u8; 4] {
            let i = y * stride + 1 + x * 4;
            [raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]
        };
        // 圆角外（左上角）透明
        assert_eq!(px(0, 0)[3], 0);
        // 圆心落在开口里 → 底色、不透明
        let c = px(size as usize / 2, size as usize / 2);
        assert_eq!(c[3], 255);
        assert!(c[0] < 0xf0, "圆心不该是白色：{c:?}");
        // 环上（圆心左侧约 0.24×尺寸处）是白色
        let x = (size as f32 * (0.5 - 0.24)).round() as usize;
        let on_ring = px(x, size as usize / 2);
        assert_eq!(on_ring[3], 255);
        assert!(on_ring[0] > 0xf0 && on_ring[1] > 0xe8, "环上该是白色：{on_ring:?}");
    }

    #[test]
    fn maskable_is_full_bleed() {
        let size = 64u32;
        let raw = raw_pixels(&png_maskable(size));
        let stride = 1 + size as usize * 4;
        for (x, y) in [(0usize, 0usize), (size as usize - 1, 0), (0, size as usize - 1)] {
            assert_eq!(raw[y * stride + 1 + x * 4 + 3], 255, "maskable 四角要填满");
        }
    }

    #[test]
    fn custom_logo_is_used_verbatim_and_only_png_is_accepted() {
        let dir = std::env::temp_dir().join("cercis-icon-source-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let png_path = dir.join("logo.png");
        std::fs::write(&png_path, png(48)).unwrap();
        let src = IconSource::from_file(&png_path).unwrap();
        assert_eq!(src.bytes(512, true), png(48), "自定义图标应原样发出去，不重新绘制");

        // SVG 不能当图标（apple-touch-icon 不认），要给出能照着做的提示
        let svg_path = dir.join("logo.svg");
        std::fs::write(&svg_path, "<svg/>").unwrap();
        let err = IconSource::from_file(&svg_path).unwrap_err();
        assert!(err.contains("PNG"), "报错要说清要 PNG：{err}");

        // 扩展名对了但内容不是 PNG，也要拦住
        let fake = dir.join("fake.png");
        std::fs::write(&fake, b"not a png").unwrap();
        assert!(IconSource::from_file(&fake).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 打包脚本要一整套图标（.icns / .ico / hicolor）；写出来的每一个都得是能用的 PNG。
    #[test]
    fn write_all_emits_a_full_icon_set() {
        let dir = std::env::temp_dir().join("cercis-icon-set-test");
        let _ = std::fs::remove_dir_all(&dir);
        let names = write_all(&dir, &IconSource::BuiltIn).unwrap();
        assert_eq!(names.len(), 11);
        for name in &names {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a], "{name}");
        }
        // 尺寸要跟文件名对上（图标集最容易在这里出错）
        let png16 = std::fs::read(dir.join("icon-16.png")).unwrap();
        assert_eq!(be32(&png16, 16), 16);
        assert_eq!(be32(&png16, 20), 16);
        let big = std::fs::read(dir.join("icon-1024.png")).unwrap();
        assert_eq!(be32(&big, 16), 1024);
        let m = std::fs::read(dir.join("icon-maskable-512.png")).unwrap();
        assert_eq!(be32(&m, 16), 512);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
