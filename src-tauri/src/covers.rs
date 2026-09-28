use base64::Engine;
use std::path::{Path, PathBuf};

pub fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn covers_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = crate::data_dir(app).join("covers");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// 默认封面底图路径（用户可在设置里更换；不存在则用内置渐变）。
pub fn default_cover_path(app: &tauri::AppHandle) -> PathBuf {
    crate::data_dir(app).join("default-cover.img")
}

pub fn serve_cover(base: &Path, name: &str) -> (String, Vec<u8>) {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return ("image/svg+xml".into(), placeholder_svg("?", "").into_bytes());
    }
    let full = base.join(name);
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    match std::fs::read(&full) {
        Ok(b) => (mime.into(), b),
        Err(_) => ("image/svg+xml".into(), placeholder_svg("📖", "").into_bytes()),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 显示宽度：全角计 2，半角计 1（用于按显示宽度而非字符数排版）。
pub fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) >= 0x1100 && is_wide(c) { 2 } else { 1 })
        .sum()
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF |
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA000..=0xA4CF |
        0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F |
        0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1FAFF)
}

/// 把「像素可用宽度」换算成「显示宽度单位」（全角计 2）。
/// 中文一个全角字 ≈ 1×字号 像素，即 2 显示单位/字号 像素。
/// 例：可用 276px、字号 51 → 276*2/51 ≈ 10.8 显示单位（约 5 个全角字/行）。
fn px_to_units(max_w_px: usize, font_size: usize) -> usize {
    ((max_w_px * 2) / font_size.max(1)).max(2)
}

/// 按显示宽度折行到多行；返回 (行数组, 是否被省略)。
/// - 每行不超过 `max_w`（显示宽度单位：全角计 2）；
/// - 最多 `max_lines` 行；超出则在最后一行尾部加省略号。
fn wrap_by_width(s: &str, max_w: usize, max_lines: usize) -> (Vec<String>, bool) {
    let chars: Vec<char> = s.chars().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let w = if is_wide(ch) { 2 } else { 1 };
        if cur_w + w > max_w && !cur.is_empty() {
            // 当前行已满：收行，准备下一行
            if lines.len() + 1 >= max_lines {
                // 已达行数上限：剩余字符全部视为溢出，收尾处理
                break;
            }
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
            continue;
        }
        cur.push(ch);
        cur_w += w;
        i += 1;
    }
    if lines.len() < max_lines && !cur.is_empty() {
        lines.push(cur);
    }
    let consumed: usize = lines.iter().map(|l| l.chars().count()).sum();
    let overflow = consumed < chars.len();
    if overflow {
        if let Some(last) = lines.last_mut() {
            let mut t: String = last
                .chars()
                .take(last.chars().count().saturating_sub(1))
                .collect();
            t.push('…');
            *last = t;
        }
    }
    (lines, overflow)
}

/// 封面排版结果（正向布局：字号随封面尺寸自适应，文字块填充下部区）。
pub struct CoverLayout {
    pub lines: Vec<String>,
    pub font_size: usize,
    pub line_h: usize,
    pub author: Option<String>,
    /// 文字区顶边（供渲染定位）。
    pub area_top: usize,
    pub area_h: usize,
}

/// 封面正向排版 layoutCover（F4 重做）：
/// - 字号 = 从 `W*0.16`（MAX，短书名会很大、占多行）往下试到 14（MIN）；
/// - 约束：文字块要塞进「下部文字区」（约占封面高 42%，底部对齐留边）；
/// - 选**能塞进的最大字号** → 短书名自然放大铺满，不再一行飘着；
/// - 降级：塞不下 → 丢作者 → 书名独占仍塞不下 → 尾部省略号（wrap_by_width 处理）；
/// - 宽度判定：全角计 2、半角计 1。
pub fn layout_cover(title: &str, author: &str, w: usize, h: usize) -> CoverLayout {
    const MARGIN_X: usize = 22;
    let max_w = w.saturating_sub(MARGIN_X * 2).max(40);
    // 文字区改到「顶部」：书名放封面上方，避开左下角进度环。
    // 顶部留白 62px：封面在网格里会缩放，而格式角标/角标是固定尺寸的 HTML 叠层，
    // 留足 62px 才能保证最小封面缩放后首行仍落在右上角格式角标下方、不被遮挡。
    let area_top = 62;
    let area_h = ((h as f64) * 0.42).round() as usize;

    let title = if title.trim().is_empty() { "未命名" } else { title.trim() };
    let author = author.trim();

    let max_fs = ((w as f64) * 0.16).round() as usize; // 320 → 51
    let max_fs = max_fs.clamp(16, 96);
    let min_fs = 14usize;

    // 从大往小试：优先带作者，其次丢作者。折行宽度按「当前字号下可容纳的显示单位」计算。
    let mut chosen: Option<CoverLayout> = None;
    let mut fs = max_fs;
    'search: while fs >= min_fs {
        let line_h = fs + fs / 5 + 6; // 行距 ≈ 1.2×字号 + 6
        let max_lines = (area_h / line_h).max(1);
        let units = px_to_units(max_w, fs);
        // 先试「书名 + 作者」
        if !author.is_empty() {
            let (lines, overflow) = wrap_by_width(title, units, max_lines.saturating_sub(1).max(1));
            let need = lines.len() + 1; // + 作者行
            if !overflow && need * line_h <= area_h {
                let a = wrap_by_width(author, units, 1).0.into_iter().next();
                chosen = Some(CoverLayout {
                    lines,
                    font_size: fs,
                    line_h,
                    author: a,
                    area_top,
                    area_h,
                });
                break 'search;
            }
        }
        // 再试「书名独占」
        let (lines, overflow) = wrap_by_width(title, units, max_lines);
        if !overflow && lines.len() * line_h <= area_h {
            chosen = Some(CoverLayout {
                lines,
                font_size: fs,
                line_h,
                author: None,
                area_top,
                area_h,
            });
            break 'search;
        }
        fs -= 2;
    }

    chosen.unwrap_or_else(|| {
        // 最小字号仍塞不下 → 尾部省略号（wrap_by_width 已处理），书名独占。
        let line_h = min_fs + min_fs / 5 + 6;
        let max_lines = (area_h / line_h).max(1);
        let units = px_to_units(max_w, min_fs);
        let (lines, _) = wrap_by_width(title, units, max_lines);
        CoverLayout { lines, font_size: min_fs, line_h, author: None, area_top, area_h }
    })
}

/// 解析 `#rgb` / `#rrggbb` 为 (r,g,b)；失败返回 None。
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().trim_start_matches('#');
    let full = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h.to_string(),
        _ => return None,
    };
    let r = u8::from_str_radix(&full[0..2], 16).ok()?;
    let g = u8::from_str_radix(&full[2..4], 16).ok()?;
    let b = u8::from_str_radix(&full[4..6], 16).ok()?;
    Some((r, g, b))
}

fn srgb_chan(c: u8) -> f64 {
    let s = c as f64 / 255.0;
    if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// WCAG 相对亮度（0=黑, 1=白）。
fn rel_lum(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * srgb_chan(r) + 0.7152 * srgb_chan(g) + 0.0722 * srgb_chan(b)
}

/// 十六进制色是否偏亮（用于决定叠字用深/浅、以及遮罩方向）。
pub fn hex_is_light(hex: &str) -> bool {
    parse_hex(hex).map(|(r, g, b)| rel_lum(r, g, b) > 0.45).unwrap_or(false)
}

/// 分析已缩放底图「书名显示区域」（底部 42%）的平均亮度，判断该区偏亮还是偏暗。
/// 返回 true=偏亮（应配深色字），false=偏暗（应配浅色字）。带 alpha 加权，透明像素不计。
pub fn region_is_light(img: &image::RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let y0 = ((h as f64) * 0.58) as u32; // 下部 42%（与文字区一致）
    let sx = (w / 80).max(1);
    let sy = ((h - y0) / 80).max(1);
    let mut sum = 0f64;
    let mut wsum = 0f64;
    let mut y = y0;
    while y < h {
        let mut x = 0u32;
        while x < w {
            let p = img.get_pixel(x, y);
            let a = p[3] as f64 / 255.0;
            sum += rel_lum(p[0], p[1], p[2]) * a;
            wsum += a;
            x += sx;
        }
        y += sy;
    }
    if wsum <= 0.0 {
        return false;
    }
    sum / wsum > 0.5
}

/// 默认封面的可主题化样式（外壳主题驱动 / 自定义底图自适应）。
pub struct CoverStyle {
    /// 用户自定义底图（`data:` URL）；None=用主题渐变。
    pub base_href: Option<String>,
    /// 渐变三档色（仅在无底图时使用）。
    pub g1: String,
    pub g2: String,
    pub g3: String,
    /// 左侧强调竖线颜色。
    pub accent: String,
    /// 书名/作者字色。
    pub text: String,
    /// 字色是否偏亮：决定底部遮罩方向（浅字→压暗，深字→提亮）与文字投影颜色。
    pub text_is_light: bool,
}

impl CoverStyle {
    /// 内置默认观感（深蓝渐变 + 暖白字 + 金线），兼容旧行为。
    pub fn builtin(base_href: Option<&str>) -> Self {
        CoverStyle {
            base_href: base_href.map(|s| s.to_string()),
            g1: "#36485c".into(),
            g2: "#212d3c".into(),
            g3: "#141c26".into(),
            accent: "#c8a86a".into(),
            text: "#f4ecdc".into(),
            text_is_light: true,
        }
    }
}

/// 生成默认封面 SVG：共享底图/主题渐变 + 自适应遮罩 + 叠字（渲染时实时叠，不落盘）。
///
/// 结构：
///   1) 底图按 2:3 居中裁切填满（`xMidYMid slice`），或主题三档渐变；
///   2) 底部线性遮罩——浅字压暗、深字提亮，保证书名区永远与字色拉开对比；
///   3) 文字带轻投影（feDropShadow）增强清晰度；字色/竖线色由样式给定。
pub fn default_cover_svg_styled(title: &str, author: &str, style: &CoverStyle) -> String {
    const W: usize = 320;
    const H: usize = 480;
    const MARGIN: usize = 22;
    const ACCENT_X: usize = MARGIN - 9;

    let lay = layout_cover(title, author, W, H);
    let n = lay.lines.len();
    let author_h = if lay.author.is_some() { lay.font_size + 8 } else { 0 };
    let block_h = n * lay.line_h + author_h;
    // 顶部对齐：书名块贴着文字区顶边排（放在封面上方）。
    let block_top = lay.area_top;

    let text_fill = escape(&style.text);
    let accent_fill = escape(&style.accent);

    let mut text = String::new();
    for (i, l) in lay.lines.iter().enumerate() {
        let y = block_top + i * lay.line_h + lay.font_size;
        text.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{y}\" font-size=\"{}\" font-weight=\"700\" fill=\"{text_fill}\" letter-spacing=\"0.6\" font-family=\"Microsoft YaHei, Segoe UI, sans-serif\">{}</text>",
            lay.font_size,
            escape(l)
        ));
    }
    if let Some(a) = &lay.author {
        let afs = (lay.font_size * 62 / 100).max(13);
        let y = block_top + n * lay.line_h + afs;
        text.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{y}\" font-size=\"{afs}\" fill=\"{text_fill}\" fill-opacity=\"0.62\" letter-spacing=\"0.6\" font-family=\"Microsoft YaHei, Segoe UI, sans-serif\">{}</text>",
            escape(a)
        ));
    }

    // 底图：用户底图（href）或主题渐变。
    let base = match &style.base_href {
        Some(href) => format!(
            "<image href=\"{}\" x=\"0\" y=\"0\" width=\"{W}\" height=\"{H}\" preserveAspectRatio=\"xMidYMid slice\"/>",
            escape(href)
        ),
        None => format!("<rect width=\"{W}\" height=\"{H}\" fill=\"url(#cvbg)\"/>"),
    };

    // 遮罩与投影随字色明暗切换：浅字→底部压黑 0.75 + 黑投影；深字→底部提白 0.72 + 白投影。
    let (shade_color, shade_op, shadow_color) = if style.text_is_light {
        ("#000000", "0.75", "#000000")
    } else {
        ("#ffffff", "0.72", "#ffffff")
    };

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{W}" height="{H}" viewBox="0 0 {W} {H}">
<defs>
<linearGradient id="cvbg" x1="0" y1="0" x2="0.85" y2="1"><stop offset="0" stop-color="{g1}"/><stop offset="0.55" stop-color="{g2}"/><stop offset="1" stop-color="{g3}"/></linearGradient>
<linearGradient id="shade" x1="0" y1="0" x2="0" y2="1">
<stop offset="0" stop-color="{shade_color}" stop-opacity="{shade_op}"/>
<stop offset="0.42" stop-color="{shade_color}" stop-opacity="0"/>
<stop offset="1" stop-color="{shade_color}" stop-opacity="0"/>
</linearGradient>
<filter id="tsh" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="1" stdDeviation="1.1" flood-color="{shadow_color}" flood-opacity="0.45"/></filter>
</defs>
{base}
<rect width="{W}" height="{H}" fill="url(#shade)"/>
<rect x="{ACCENT_X}" y="{block_top}" width="3" height="{block_h}" rx="1.5" fill="{accent_fill}" fill-opacity="0.85"/>
<g filter="url(#tsh)">
{text}
</g>
</svg>"##,
        g1 = escape(&style.g1),
        g2 = escape(&style.g2),
        g3 = escape(&style.g3),
    )
}

/// 兼容旧调用：内置观感 + 可选底图。
pub fn default_cover_svg(title: &str, author: &str, base_href: Option<&str>) -> String {
    default_cover_svg_styled(title, author, &CoverStyle::builtin(base_href))
}

/// 兼容旧调用：无底图版本的默认封面。
pub fn placeholder_svg(title: &str, author: &str) -> String {
    default_cover_svg(title, author, None)
}

pub fn make_thumbnail(data: &[u8], out_png: &Path) -> Result<(), String> {
    let img = image::load_from_memory(data).map_err(|e| e.to_string())?;
    let thumb = img.thumbnail(320, 480);
    thumb.save(out_png).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_placeholder(out_svg: &Path, title: &str, author: &str) -> Result<(), String> {
    std::fs::write(out_svg, placeholder_svg(title, author)).map_err(|e| e.to_string())?;
    Ok(())
}

/// 写入默认封面（可带用户底图 data URL）。
pub fn write_default_cover(
    out_svg: &Path,
    title: &str,
    author: &str,
    base_href: Option<&str>,
) -> Result<(), String> {
    std::fs::write(out_svg, default_cover_svg(title, author, base_href)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_wide_as_two() {
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文abc"), 7);
    }

    #[test]
    fn wrap_short_title_one_line() {
        let (l, of) = wrap_by_width("短书名", 272, 4);
        assert_eq!(l, vec!["短书名".to_string()]);
        assert!(!of);
    }

    #[test]
    fn wrap_long_english_ellipsis() {
        // 宽 272、半角计 1：每行约 272 字符；4 行上限 → 未超 1088 不溢出。
        // 这里用 1300 字符保证溢出。
        let long: String = "A".repeat(1300);
        let (l, of) = wrap_by_width(&long, 272, 4);
        assert!(of);
        assert_eq!(l.len(), 4);
        assert!(l.last().unwrap().ends_with('…'));
    }

    #[test]
    fn long_cjk_triggers_overflow() {
        // 宽 272、全角计 2：每行约 136 字；4 行上限 → 需 >544 字才溢出。
        let long: String = "中".repeat(600);
        let (l, of) = wrap_by_width(&long, 272, 4);
        assert!(of);
        assert!(l.last().unwrap().ends_with('…'));
    }

    #[test]
    fn normal_title_no_overflow() {
        let (l, of) = wrap_by_width("普通书名", 272, 4);
        assert_eq!(l, vec!["普通书名".to_string()]);
        assert!(!of);
    }

    #[test]
    fn cover_has_gradient_shade_and_warm_text() {
        let svg = default_cover_svg("测试书名", "测试作者", None);
        // 底部渐变遮罩（非全幅纯黑）
        assert!(svg.contains("id=\"shade\""));
        assert!(svg.contains("stop-opacity=\"0.75\""));
        assert!(!svg.contains("fill-opacity=\"0.60\""));
        // 暖白文字（避免纯白刺眼）+ 左侧强调竖线
        assert!(svg.contains("fill=\"#f4ecdc\""));
        assert!(!svg.contains("fill=\"#ffffff\""));
        assert!(svg.contains("#c8a86a"));
        assert!(svg.contains("测试书名"));
    }

    #[test]
    fn cover_with_base_uses_slice() {
        let svg = default_cover_svg("书名", "作者", Some("data:image/png;base64,AAAA"));
        assert!(svg.contains("preserveAspectRatio=\"xMidYMid slice\""));
        assert!(svg.contains("id=\"shade\""));
        assert!(svg.contains("fill=\"#f4ecdc\""));
    }

    #[test]
    fn short_title_gets_large_font() {
        // 短书名应取到较大字号（接近 MAX = W*0.16 = 51）
        let l = layout_cover("围城", "钱钟书", 320, 480);
        assert!(l.font_size >= 40, "short title font too small: {}", l.font_size);
        assert_eq!(l.author.as_deref(), Some("钱钟书"));
        assert!(!l.lines.is_empty());
    }

    #[test]
    fn long_title_drops_author() {
        // 足够长：书名独占能放下、但加上作者行就放不下 → 丢作者。
        let long: String = "长".repeat(900);
        let l = layout_cover(&long, "作者", 320, 480);
        assert!(l.author.is_none(), "author should be dropped for {}", long.chars().count());
        assert!(l.font_size <= 51);
    }

    #[test]
    fn very_long_title_ellipsis() {
        let long: String = "长".repeat(5000);
        let l = layout_cover(&long, "作者", 320, 480);
        assert!(l.author.is_none());
        assert!(l.lines.last().unwrap().ends_with('…'));
    }

    #[test]
    fn block_fits_in_text_area() {
        let l = layout_cover("一本中等长度的书名测试", "作者", 320, 480);
        let author_h = if l.author.is_some() { l.font_size + 8 } else { 0 };
        let block_h = l.lines.len() * l.line_h + author_h;
        assert!(block_h <= l.area_h, "block {block_h} exceeds area {}", l.area_h);
    }

    #[test]
    fn empty_title_fallback() {
        let svg = default_cover_svg("", "", None);
        assert!(svg.contains("未命名"));
    }
}
