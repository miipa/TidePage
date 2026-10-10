use regex::Regex;
use serde::Serialize;

#[derive(Serialize)]
pub struct SuggestMeta {
    pub title: String,
    pub author: String,
}

fn clean_base(name: &str) -> String {
    let mut s = name.trim().to_string();
    s = s.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(s);
    let re_tag = Regex::new(r"^\[[^\]]{0,40}\]").unwrap();
    loop {
        let t = re_tag.replace(&s, "").trim_start().to_string();
        if t == s {
            break;
        }
        s = t;
    }
    s
}

pub fn from_filename(file_name: &str) -> SuggestMeta {
    let base = clean_base(file_name);

    let re_book_author = Regex::new(r"^《(.+?)》\s*(?:作者|著)?[:：\s]*\s*(.+?)$").unwrap();
    if let Some(c) = re_book_author.captures(&base) {
        let author = clean_author(c[2].trim());
        return SuggestMeta { title: c[1].trim().to_string(), author };
    }

    let re_paren = Regex::new(r"^(.+?)[（(]\s*(.+?)\s*[）)]\s*(?:\(?.*?\)?$)?").unwrap();
    if let Some(c) = re_paren.captures(&base) {
        let inner = c[2].trim();
        if inner.chars().count() <= 20 && looks_like_author(inner) {
            return SuggestMeta { title: c[1].trim().to_string(), author: clean_author(inner) };
        }
    }

    let re_author_sep = Regex::new(r"^(.+?)[\s\-—_]+(?:作者|著)\s*[:：]?\s*(.+?)$").unwrap();
    if let Some(c) = re_author_sep.captures(&base) {
        return SuggestMeta {
            title: c[1].trim().to_string(),
            author: clean_author(c[2].trim()),
        };
    }

    let re_zhi_tail = Regex::new(r"^(.+?)\s+([^\s]+?)\s*著\s*(\(?\d+(?:\.\d+)?\)?.*|V\d.*)?$").unwrap();
    if let Some(c) = re_zhi_tail.captures(&base) {
        let cand = c[2].trim();
        if cand.chars().count() <= 12 && cand != c[1].trim() {
            return SuggestMeta { title: c[1].trim().to_string(), author: cand.to_string() };
        }
    }

    SuggestMeta { title: base, author: String::new() }
}

fn looks_like_author(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > 16 {
        return false;
    }
    if t.contains("完本") || t.contains("精校") || t.contains("校对") || t.contains("系列") || t.contains("全集") || t.contains("套装") {
        return false;
    }
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    let is_cjk = t.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c) || c.is_alphabetic());
    is_cjk && !has_digit
}

fn clean_author(s: &str) -> String {
    let mut t = s.trim().to_string();
    for pat in ["作者:", "作者：", "著"] {
        t = t.replace(pat, "");
    }
    let re_paren = Regex::new(r"[（(][^）)]*[）)]").unwrap();
    t = re_paren.replace_all(&t, "").trim().to_string();
    t
}

pub fn from_text_start(text: &str) -> Option<String> {
    let head: String = text.chars().take(1200).collect();
    let re = Regex::new(r"(?m)(?:作者|著者|作 者)\s*[:：]\s*([^\s，,。;；:：\r\n]{1,20})").unwrap();
    if let Some(c) = re.captures(&head) {
        let a = c[1].trim().to_string();
        if !a.is_empty() && a != "暂无" {
            return Some(a);
        }
    }
    let re2 = Regex::new(r"(?m)^\s*(《.+?》)\s*[，,、]\s*([^\s，。]{1,16})\s*[著]").unwrap();
    if let Some(c) = re2.captures(&head) {
        return Some(c[2].trim().to_string());
    }
    None
}

pub fn suggest(path: &str, data_head: Option<&str>) -> SuggestMeta {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let mut m = from_filename(&name);
    if m.author.is_empty() {
        if let Some(head) = data_head {
            if let Some(a) = from_text_start(head) {
                m.author = a;
            }
        }
    }
    m
}
