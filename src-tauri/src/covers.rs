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

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
    // 当前行首在 chars 中的下标：供「西文不切词」回退时把残词整段还给下一行。
    let mut line_start = 0usize;
    // 回退断行会**丢弃**断点处的那个空白字符（不写进任何一行），
    // 故溢出判定要把它补回来，否则 "Hello wonderful world" 这种会被误判成溢出。
    let mut dropped = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let w = if is_wide(ch) { 2 } else { 1 };
        if cur_w + w > max_w && !cur.is_empty() {
            // 当前行已满：收行，准备下一行
            if lines.len() + 1 >= max_lines {
                // 已达行数上限：剩余字符全部视为溢出，收尾处理
                break;
            }
            // 西文不切词：断点若落在**词中间**（本行末字与下一字都是非全角、非空白），
            // 就退到本行最后一个空格处断行，把残词整段留给下一行。
            // 中文不受影响（全角字任意位置都可断）；整行没有空格的西文串也无处可退
            // （rposition 返回 None），两种情况都保持原有行为。
            let mut next_start = i;
            let last_is_word = cur
                .chars()
                .last()
                .map(|c| !is_wide(c) && !c.is_whitespace())
                .unwrap_or(false);
            if last_is_word && !is_wide(ch) && !ch.is_whitespace() {
                // 注意：std::str::Chars 不是 ExactSizeIterator，用不了 rposition，
                // 先收成 Vec 再对切片迭代器求最后一个空白位置。
                let cur_chars: Vec<char> = cur.chars().collect();
                if let Some(sp) = cur_chars.iter().rposition(|c| c.is_whitespace()) {
                    let keep: String = cur_chars[..sp].iter().collect();
                    if !keep.is_empty() {
                        cur = keep;
                        next_start = line_start + sp + 1;
                        dropped += 1; // 断点处的空白不再属于任何一行
                    }
                }
            }
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
            i = next_start;
            line_start = next_start;
            continue;
        }
        cur.push(ch);
        cur_w += w;
        i += 1;
    }
    if lines.len() < max_lines && !cur.is_empty() {
        lines.push(cur);
    }
    let consumed: usize = lines.iter().map(|l| l.chars().count()).sum::<usize>() + dropped;
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
    // 测试可观测：暴露 42% 文字区高度，供 covers.rs 的 fit 不变量断言 assert!(block_h <= l.area_h) 读取（生产渲染经同名局部变量计算，勿混淆）。
    #[allow(dead_code)]
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
    // 书框内缩。取 MARGIN-9 是有来历的：旧设计在这个 x 上画一条 3px 左侧竖线，
    // 高度等于书名块 —— 但小尺寸下它读起来像一截文本光标，而不是设计元素。
    // 改成整圈发丝书框后，那条竖线的位置正好成为框线：装饰条 → 书封。
    const FRAME_INSET: usize = MARGIN - 9;

    let lay = layout_cover(title, author, W, H);
    let n = lay.lines.len();
    // 顶部对齐：书名块贴着文字区顶边排（放在封面上方）。
    let block_top = lay.area_top;

    let text_fill = escape(&style.text);
    let accent_fill = escape(&style.accent);

    let mut text = String::new();
    for (i, l) in lay.lines.iter().enumerate() {
        let y = block_top + i * lay.line_h + lay.font_size;
        text.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{y}\" font-size=\"{}\" font-weight=\"700\" fill=\"{text_fill}\" letter-spacing=\"0.6\" font-family=\"Noto Sans SC, Microsoft YaHei, Segoe UI, sans-serif\">{}</text>",
            lay.font_size,
            escape(l)
        ));
    }
    if let Some(a) = &lay.author {
        let afs = (lay.font_size * 62 / 100).max(13);
        let y = block_top + n * lay.line_h + afs;
        text.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{y}\" font-size=\"{afs}\" fill=\"{text_fill}\" fill-opacity=\"0.62\" letter-spacing=\"0.6\" font-family=\"Noto Sans SC, Microsoft YaHei, Segoe UI, sans-serif\">{}</text>",
            escape(a)
        ));
    }

    // 出版者标记（colophon）：封底一条短 accent 线 + 极小字距化字标。
    // 两件事一起做：给封面一个「书封」该有的下沿收口，同时填掉下方那片一直空着的渐变
    // （旧版下半幅是纯空白色块，整屏书排开时像一排没渲染完的占位图）。
    let imprint = format!(
        "<rect x=\"{MARGIN}\" y=\"{line_y}\" width=\"26\" height=\"2\" rx=\"1\" fill=\"{accent_fill}\" fill-opacity=\"0.7\"/>\
<text x=\"{MARGIN}\" y=\"{text_y}\" font-size=\"10\" font-weight=\"600\" fill=\"{text_fill}\" fill-opacity=\"0.42\" letter-spacing=\"2.4\" font-family=\"Noto Sans SC, Microsoft YaHei, Segoe UI, sans-serif\">TIDEPAGE</text>",
        line_y = H - 44,
        text_y = H - 26,
    );

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
<!-- 底：径向暗角。旧版是 45° 线性渐变，在封面上会留下一道斜向色带（大屏平铺时尤其明显）。
     径向 + 光源固定在左上（cx/cy）与全应用的「单一光源」纪律一致。 -->
<radialGradient id="cvbg" cx="0.30" cy="0.14" r="1.10"><stop offset="0" stop-color="{g1}"/><stop offset="0.55" stop-color="{g2}"/><stop offset="1" stop-color="{g3}"/></radialGradient>
<linearGradient id="shade" x1="0" y1="0" x2="0" y2="1">
<stop offset="0" stop-color="{shade_color}" stop-opacity="{shade_op}"/>
<stop offset="0.42" stop-color="{shade_color}" stop-opacity="0"/>
<stop offset="1" stop-color="{shade_color}" stop-opacity="0"/>
</linearGradient>
<!-- 纸纹：去饱和的低频湍流，alpha 上限 8.5%。只盖底、不盖字 ——
     文字在滤镜之后绘制，笔画不会被噪点啃边。 -->
<filter id="grain" x="0" y="0" width="100%" height="100%" color-interpolation-filters="sRGB">
<feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="3" stitchTiles="stitch"/>
<feColorMatrix type="matrix" values="0.33 0.33 0.33 0 0  0.33 0.33 0.33 0 0  0.33 0.33 0.33 0 0  0 0 0 0.085 0"/>
</filter>
<filter id="tsh" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="1" stdDeviation="1.1" flood-color="{shadow_color}" flood-opacity="0.45"/></filter>
</defs>
{base}
<rect width="{W}" height="{H}" fill="url(#shade)"/>
<rect width="{W}" height="{H}" filter="url(#grain)"/>
<rect x="{FRAME_INSET}" y="{FRAME_INSET}" width="{fw}" height="{fh}" fill="none" stroke="{accent_fill}" stroke-opacity="0.34" stroke-width="1"/>
{imprint}
<g filter="url(#tsh)">
{text}
</g>
</svg>"##,
        g1 = escape(&style.g1),
        g2 = escape(&style.g2),
        g3 = escape(&style.g3),
        fw = W - FRAME_INSET * 2,
        fh = H - FRAME_INSET * 2,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn wrap_does_not_split_latin_word() {
        // 宽度 12 恰好放不下 "Hello wonderful"：必须退到空格处断行，
        // 而不是把 "wonderful" 切成 "wonder"/"ful"。
        let (l, of) = wrap_by_width("Hello wonderful world", 12, 4);
        assert!(!of);
        assert_eq!(l, vec!["Hello".to_string(), "wonderful".to_string(), "world".to_string()]);
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
    fn cover_has_editorial_frame_grain_and_imprint() {
        // 第九轮重做：底从 45° 线性渐变改为径向暗角（线性渐变会在封面上留一道斜向色带），
        // 原先那条「像文本光标」的左侧 3px 竖线改成整圈发丝书框，并补纸纹与出版者标记。
        let svg = default_cover_svg("围城", "钱钟书", None);
        assert!(
            svg.contains("<radialGradient id=\"cvbg\""),
            "expected radial base gradient"
        );
        assert!(
            !svg.contains("<linearGradient id=\"cvbg\""),
            "diagonal linear base gradient should be gone"
        );
        assert!(
            svg.contains("x=\"13\" y=\"13\" width=\"294\" height=\"454\" fill=\"none\""),
            "expected hairline cover frame"
        );
        assert!(
            svg.contains("id=\"grain\"") && svg.contains("feTurbulence"),
            "expected paper grain filter"
        );
        assert!(svg.contains("TIDEPAGE"), "expected publisher imprint");
        // 旧的左侧竖线是 3px 宽 + rx=1.5，书框不该带圆角
        assert!(!svg.contains("width=\"3\" height="), "old left accent bar should be gone");
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
