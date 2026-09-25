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
    // 文字区：底部 ~42% 高，底部留边 14px
    let area_h = ((h as f64) * 0.42).round() as usize;
    let area_top = h.saturating_sub(area_h + 14);

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

/// 生成默认封面 SVG（F2 架构）：共享底图 + 底部渐变遮罩 + 底部白字（渲染时实时叠）。
///
/// 明确结构，不做任何"检测图片明暗自动切字色"：
///   1) 底图按 2:3 居中裁切填满（`xMidYMid slice`，不变形、不留边）；
///   2) 底部线性渐变遮罩（上方透明 → 底部黑 0.75），压住文字区，底图不整体变灰；
///   3) 文字固定白色，放底部。
///
/// `base_href` 为内置渐变时传 None；为用户底图时传其 `data:` URL。
pub fn default_cover_svg(title: &str, author: &str, base_href: Option<&str>) -> String {
    const W: usize = 320;
    const H: usize = 480;
    const MARGIN: usize = 22;
    const ACCENT_X: usize = MARGIN - 9;
    let max_w = W - MARGIN * 2; // 276

    let lay = layout_cover(title, author, W, H);
    let n = lay.lines.len();
    let author_h = if lay.author.is_some() { lay.font_size + 8 } else { 0 };
    let block_h = n * lay.line_h + author_h;

    // 文字块在下部区内垂直居中
    let block_top = lay.area_top + lay.area_h.saturating_sub(block_h) / 2;
    let mut text = String::new();
    for (i, l) in lay.lines.iter().enumerate() {
        let y = block_top + i * lay.line_h + lay.font_size;
        text.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{y}\" font-size=\"{}\" font-weight=\"700\" fill=\"#f4ecdc\" letter-spacing=\"0.6\" font-family=\"Microsoft YaHei, Segoe UI, sans-serif\">{}</text>",
            lay.font_size,
            escape(l)
        ));
    }
    if let Some(a) = &lay.author {
        let afs = (lay.font_size * 62 / 100).max(13);
        let y = block_top + n * lay.line_h + afs;
        text.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{y}\" font-size=\"{afs}\" fill=\"#f4ecdc\" fill-opacity=\"0.62\" letter-spacing=\"0.6\" font-family=\"Microsoft YaHei, Segoe UI, sans-serif\">{}</text>",
            escape(a)
        ));
    }

    // 底图：用户底图（href）或内置渐变。
    let base = match base_href {
        Some(href) => format!(
            "<image href=\"{}\" x=\"0\" y=\"0\" width=\"{W}\" height=\"{H}\" preserveAspectRatio=\"xMidYMid slice\"/>",
            escape(href)
        ),
        None => format!("<rect width=\"{W}\" height=\"{H}\" fill=\"url(#cvbg)\"/>"),
    };

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{W}" height="{H}" viewBox="0 0 {W} {H}">
<defs>
<linearGradient id="cvbg" x1="0" y1="0" x2="0.85" y2="1"><stop offset="0" stop-color="#36485c"/><stop offset="0.55" stop-color="#212d3c"/><stop offset="1" stop-color="#141c26"/></linearGradient>
<linearGradient id="shade" x1="0" y1="0" x2="0" y2="1">
<stop offset="0" stop-color="#000000" stop-opacity="0"/>
<stop offset="0.55" stop-color="#000000" stop-opacity="0"/>
<stop offset="1" stop-color="#000000" stop-opacity="0.75"/>
</linearGradient>
</defs>
{base}
<rect width="{W}" height="{H}" fill="url(#shade)"/>
<rect x="{ACCENT_X}" y="{block_top}" width="3" height="{block_h}" rx="1.5" fill="#c8a86a" fill-opacity="0.85"/>
{text}
</svg>"##
    )
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
