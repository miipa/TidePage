use crate::models::TocEntry;
use regex::Regex;

pub struct MobiDoc {
    pub title: String,
    pub author: String,
    pub html: String,
    pub toc: Vec<TocEntry>,
    pub parts: Vec<(usize, usize)>,
}

fn clean_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let code = ch as u32;
        let is_kf8_marker = (0xE000..=0xF8FF).contains(&code);
        let is_bad_ctrl = code < 0x20 && !matches!(ch, '\n' | '\r' | '\t');
        if is_kf8_marker || is_bad_ctrl {
            continue;
        }
        out.push(ch);
    }
    out
}

pub fn parse(path: &str) -> Result<MobiDoc, String> {
    let book =
        mobi::Mobi::from_path(path).map_err(|e| format!("MOBI/AZW3 解析失败: {e:?}"))?;
    let title = book.title().trim().to_string();
    let author = book.author().unwrap_or_default().trim().to_string();
    let text = book.content_as_string_lossy();
    let html = clean_html(&text);

    let (toc, parts) = split_mobi_chapters(&html);

    Ok(MobiDoc {
        title: if title.is_empty() {
            std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            title
        },
        author,
        html,
        toc,
        parts,
    })
}

fn split_mobi_chapters(html: &str) -> (Vec<TocEntry>, Vec<(usize, usize)>) {
    let h_re = Regex::new(r"(?is)<h[1-6][^>]*>(.*?)</h[1-6]>").unwrap();
    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    let mut starts: Vec<(usize, String)> = Vec::new();
    for c in h_re.captures_iter(html) {
        let m = c.get(0).unwrap();
        let text: String = tag_re.replace_all(&c[1], "").trim().chars().take(60).collect();
        if text.is_empty() {
            continue;
        }
        if starts.last().map(|(p, _)| *p) == Some(m.start()) {
            continue;
        }
        starts.push((m.start(), text));
    }

    if starts.len() < 2 {
        starts.clear();
        let pb_re = Regex::new(r"(?i)<mbp:pagebreak\s*/?>").unwrap();
        let mut n = 0;
        for m in pb_re.find_iter(html) {
            n += 1;
            starts.push((m.start(), format!("第 {n} 页")));
        }
    }

    let mut toc = Vec::new();
    let mut parts = Vec::new();
    if starts.is_empty() {
        toc.push(TocEntry { title: "全文".into(), index: 0, chars: html.chars().count() as i64 });
        parts.push((0, html.len()));
        return (toc, parts);
    }
    if starts[0].0 > 0 {
        toc.push(TocEntry { title: "前言".into(), index: 0, chars: html[..starts[0].0].chars().count() as i64 });
        parts.push((0, starts[0].0));
    }
    for (i, (start, title)) in starts.iter().enumerate() {
        let end = if i + 1 < starts.len() {
            starts[i + 1].0
        } else {
            html.len()
        };
        let chars = html.get(*start..end).map(|s| s.chars().count() as i64).unwrap_or(0);
        toc.push(TocEntry {
            title: title.clone(),
            index: toc.len(),
            chars,
        });
        parts.push((*start, end));
    }
    (toc, parts)
}
