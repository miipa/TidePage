use crate::models::TocEntry;
use fancy_regex::Regex;

pub struct PlainDoc {
    pub text: String,
    pub ranges: Vec<(usize, usize)>,
    pub toc: Vec<TocEntry>,
}

fn line_bounds(text: &str, pos: usize) -> (usize, usize) {
    let start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[start..]
        .find('\n')
        .map(|i| start + i)
        .unwrap_or(text.len());
    (start, end)
}

pub fn split_plain(text: &str) -> PlainDoc {
    split_custom(text, crate::default_pattern()).unwrap_or_else(|_| PlainDoc {
        text: text.to_string(),
        ranges: vec![(0, text.len())],
        toc: vec![TocEntry {
            title: "全文".into(),
            index: 0,
            chars: text.chars().count() as i64,
        }],
    })
}

pub fn decode_and_split(path: &str, pattern: Option<&str>) -> Result<PlainDoc, String> {
    let data = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let (text, _enc) = decode_bytes(&data);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    match pattern {
        Some(p) if !p.trim().is_empty() => split_custom(&text, p),
        _ => Ok(split_plain(&text)),
    }
}

pub fn split_custom(text: &str, pattern: &str) -> Result<PlainDoc, String> {
    let full = if pattern.contains("(?m") {
        pattern.to_string()
    } else {
        format!("(?m){pattern}")
    };
    let re = Regex::new(&full).map_err(|e| format!("正则表达式无效: {e}"))?;
    let mut starts: Vec<(usize, String)> = Vec::new();
    for m in re.find_iter(text) {
        let m = m.map_err(|e| format!("正则执行出错: {e}"))?;
        let (s, _e) = line_bounds(text, m.start());
        if starts.last().map(|(p, _)| *p) == Some(s) {
            continue;
        }
        let title = text[s.._e].trim().to_string();
        if title.is_empty() {
            continue;
        }
        starts.push((s, title.chars().take(60).collect()));
    }

    let mut toc = Vec::new();
    let mut ranges = Vec::new();
    let mut push_ch = |title: &str, s: usize, e: usize, toc: &mut Vec<TocEntry>, ranges: &mut Vec<(usize, usize)>| {
        toc.push(TocEntry {
            title: title.to_string(),
            index: toc.len(),
            chars: text[s..e].chars().count() as i64,
        });
        ranges.push((s, e));
    };
    if starts.is_empty() {
        push_ch("全文", 0, text.len(), &mut toc, &mut ranges);
        return Ok(PlainDoc {
            text: text.to_string(),
            ranges,
            toc,
        });
    }
    if starts[0].0 > 0 {
        push_ch("前言", 0, starts[0].0, &mut toc, &mut ranges);
    }
    for (i, (start, title)) in starts.iter().enumerate() {
        let end = if i + 1 < starts.len() {
            starts[i + 1].0
        } else {
            text.len()
        };
        push_ch(title, *start, end, &mut toc, &mut ranges);
    }
    Ok(PlainDoc {
        text: text.to_string(),
        ranges,
        toc,
    })
}

pub fn decode_bytes(data: &[u8]) -> (String, &'static str) {
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
        let units: Vec<u16> = data[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return (String::from_utf16_lossy(&units), "utf-16le");
    }
    if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
        let units: Vec<u16> = data[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return (String::from_utf16_lossy(&units), "utf-16be");
    }
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(&data[3..]).into_owned(), "utf-8");
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return (s.to_string(), "utf-8");
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(data, true);
    let enc = det.guess(None, true);
    let (s, _, _) = enc.decode(data);
    (s.into_owned(), enc.name())
}

pub fn render_html(body: &str) -> String {
    let mut html = String::with_capacity(body.len() + 256);
    html.push_str("<div class=\"txt-body\">");
    for para in body.split('\n') {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        html.push_str("<p>");
        for ch in p.chars() {
            match ch {
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '&' => html.push_str("&amp;"),
                _ => html.push(ch),
            }
        }
        html.push_str("</p>");
    }
    html.push_str("</div>");
    html
}

/// 渲染 TXT 章节（第 1 段若以“第X章”等标题开头，作为 h2 标题显示）
pub fn render_html_with_title(body: &str) -> String {
    let mut html = String::with_capacity(body.len() + 256);
    html.push_str("<div class=\"txt-body\">");
    let mut first = true;
    for para in body.split('\n') {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        if first && is_heading(&p) {
            html.push_str("<h2>");
            escape_into(&mut html, p);
            html.push_str("</h2>");
            first = false;
            continue;
        }
        first = false;
        html.push_str("<p>");
        escape_into(&mut html, p);
        html.push_str("</p>");
    }
    html.push_str("</div>");
    html
}

fn is_heading(line: &str) -> bool {
    let t = line.trim().trim_end_matches(':').trim_end_matches('：');
    t.chars().count() <= 40
        && (t.starts_with("第")
            || (t.chars().all(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || c == '.') && t.len() <= 24))
}

fn escape_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}
