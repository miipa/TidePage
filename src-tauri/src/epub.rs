use crate::models::TocEntry;
use crate::txt::escape_html;
use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

pub struct EpubDoc {
    pub title: String,
    pub author: String,
    pub spine: Vec<String>,
    pub toc: Vec<TocEntry>,
    pub cover_href: Option<String>,
    pub spine_chars: Vec<i64>,
    pub total_chars: i64,
    pub total_text_chars: i64,
}

type CiMap = HashMap<String, usize>;

fn build_ci<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> CiMap {
    let mut map: CiMap = HashMap::new();
    let count = zip.len();
    for i in 0..count {
        if let Ok(f) = zip.by_index(i) {
            let nm = f.name().replace('\\', "/").to_lowercase();
            map.entry(nm.clone()).or_insert(i);
            if let Some(base) = nm.rsplit('/').next() {
                map.entry(format!("b:{base}")).or_insert(i);
            }
            if let Some(rest) = nm.strip_prefix("./") {
                map.entry(rest.to_string()).or_insert(i);
            }
        }
    }
    map
}

fn resolve_idx(ci: &CiMap, name: &str) -> Option<usize> {
    let want = name.replace('\\', "/").to_lowercase();
    let want = want.strip_prefix("./").unwrap_or(&want);
    ci.get(want)
        .or_else(|| ci.get(&format!("b:{}", want.rsplit('/').next().unwrap_or(want))))
        .copied()
}

fn zip_read<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    name: &str,
) -> Option<Vec<u8>> {
    let idx = resolve_idx(ci, name)?;
    let mut f = zip.by_index(idx).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn norm_path(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href);
    let href = percent_decode(href);
    let combined = if href.starts_with('/') {
        href.trim_start_matches('/').to_string()
    } else {
        let base_dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut parts: Vec<&str> = if base_dir.is_empty() {
            vec![]
        } else {
            base_dir.split('/').collect()
        };
        for seg in href.split('/') {
            match seg {
                "." => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        parts.join("/")
    };
    combined
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse(data: Vec<u8>) -> Result<EpubDoc, String> {
    let cursor = std::io::Cursor::new(&data);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("EPUB 打开失败: {e}"))?;
    let ci = build_ci(&mut zip);

    let container = zip_read(&mut zip, &ci, "META-INF/container.xml")
        .ok_or("缺少 META-INF/container.xml")?;
    let container_str = String::from_utf8_lossy(&container).into_owned();
    let opf_path = Regex::new(r#"full-path\s*=\s*"([^"]+)""#)
        .ok()
        .and_then(|re| re.captures(&container_str))
        .map(|c| c[1].to_string())
        .ok_or("container.xml 中未找到 OPF")?;

    let opf_bytes = zip_read(&mut zip, &ci, &opf_path).ok_or("OPF 文件缺失")?;
    let opf = String::from_utf8_lossy(&opf_bytes).into_owned();
    let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let mut title = String::new();
    let mut author = String::new();
    let mut cover_href: Option<String> = None;
    let mut items: Vec<(String, String, String)> = Vec::new();
    let mut spine_ids: Vec<String> = Vec::new();

    let mut reader = quick_xml::Reader::from_str(&opf);
    reader.config_mut().trim_text(true);
    let mut cur_elem = String::new();
    let mut in_meta = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                let name = String::from_utf8_lossy(&name).into_owned();
                if name.ends_with("metadata") {
                    in_meta = true;
                }
                if name == "item" {
                    let mut id = String::new();
                    let mut href = String::new();
                    let mut props = String::new();
                    let mut mtype = String::new();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        let val = String::from_utf8_lossy(&a.value).into_owned();
                        match key.as_str() {
                            "id" => id = val,
                            "href" => href = val,
                            "properties" => props = val,
                            "media-type" => mtype = val,
                            _ => {}
                        }
                    }
                    items.push((id, href, format!("{props}\u{1}{mtype}")));
                }
                if name == "itemref" {
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        if key == "idref" {
                            spine_ids.push(String::from_utf8_lossy(&a.value).into_owned());
                        }
                    }
                }
                cur_elem = name;
            }
            Ok(quick_xml::events::Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if name == "item" {
                    let mut id = String::new();
                    let mut href = String::new();
                    let mut props = String::new();
                    let mut mtype = String::new();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        let val = String::from_utf8_lossy(&a.value).into_owned();
                        match key.as_str() {
                            "id" => id = val,
                            "href" => href = val,
                            "properties" => props = val,
                            "media-type" => mtype = val,
                            _ => {}
                        }
                    }
                    items.push((id, href, format!("{props}\u{1}{mtype}")));
                }
                if name == "itemref" {
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        if key == "idref" {
                            spine_ids.push(String::from_utf8_lossy(&a.value).into_owned());
                        }
                    }
                }
                if name == "meta" && in_meta {
                    let mut mn = String::new();
                    let mut mv = String::new();
                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.as_ref()).to_lowercase();
                        let val = String::from_utf8_lossy(&a.value).into_owned();
                        if key == "name" {
                            mn = val;
                        } else if key == "content" {
                            mv = val;
                        }
                    }
                    if mn == "cover" && !mv.is_empty() {
                        for (id, href, _) in &items {
                            if id == &mv {
                                cover_href = Some(href.clone());
                            }
                        }
                    }
                }
            }
            Ok(quick_xml::events::Event::Text(t)) => {
                let txt = t.unescape().unwrap_or_default().trim().to_string();
                if !txt.is_empty() {
                    if cur_elem.ends_with("title") && title.is_empty() && in_meta {
                        title = txt;
                    } else if cur_elem.ends_with("creator") && author.is_empty() && in_meta {
                        author = txt;
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                if name.ends_with("metadata") {
                    in_meta = false;
                }
                cur_elem.clear();
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    if title.is_empty() {
        title = Path::new(&opf_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    let mut spine: Vec<String> = Vec::new();
    for idref in &spine_ids {
        for (id, href, meta) in &items {
            if id == idref {
                let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
                if mtype.contains("html") || mtype.is_empty() || mtype.contains("xml") {
                    spine.push(norm_path(&opf_dir, href));
                }
            }
        }
    }
    if spine.is_empty() {
        for (_, href, meta) in &items {
            let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
            if mtype.contains("html") {
                spine.push(norm_path(&opf_dir, href));
            }
        }
    }

    if cover_href.is_none() {
        for (id, href, meta) in &items {
            let (props, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
            if props.contains("cover-image") || (mtype.starts_with("image") && id.contains("cover"))
            {
                cover_href = Some(norm_path(&opf_dir, href));
                break;
            }
        }
    } else {
        cover_href = cover_href.map(|h| norm_path(&opf_dir, &h));
    }

    let spine_idx: Vec<Option<usize>> = spine.iter().map(|s| resolve_idx(&ci, s)).collect();
    let nworkers = rayon::current_num_threads().max(1);
    let spine_chars: Vec<(i64, i64)> = {
        let counter = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new(vec![(0i64, 0i64); spine_idx.len()]);
        let spine_idx_ref: &[Option<usize>] = &spine_idx;
        let data_ref: &[u8] = &data;
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..nworkers {
                handles.push(s.spawn(|| {
                    let mut zip = match zip::ZipArchive::new(std::io::Cursor::new(data_ref)) {
                        Ok(z) => z,
                        Err(_) => return,
                    };
                    loop {
                        let i = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= spine_idx_ref.len() {
                            break;
                        }
                        let Some(member) = spine_idx_ref[i] else { continue };
                        if let Ok(mut f) = zip.by_index(member) {
                            let mut b = Vec::new();
                            if f.read_to_end(&mut b).is_ok() {
                                let html = String::from_utf8_lossy(&b).into_owned();
                                let text = html_to_text(&html);
                                let total = text.chars().count() as i64;
                                let pure = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
                                results.lock().unwrap()[i] = (total, pure);
                            }
                        }
                    }
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        });
        results.into_inner().unwrap_or_else(|e| e.into_inner())
    };

    let toc = extract_toc(&mut zip, &ci, &opf_dir, &items, &spine, &title, &spine_chars);

    let total_chars: i64 = spine_chars.iter().map(|c| c.0).sum();
    let total_text_chars: i64 = spine_chars.iter().map(|c| c.1).sum();

    Ok(EpubDoc {
        title,
        author,
        spine,
        toc,
        cover_href,
        spine_chars: spine_chars.iter().map(|c| c.0).collect(),
        total_chars,
        total_text_chars,
    })
}

fn extract_toc<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    opf_dir: &str,
    items: &[(String, String, String)],
    spine: &[String],
    fallback: &str,
    spine_chars: &[(i64, i64)],
) -> Vec<TocEntry> {
    let mut pairs: Vec<(String, String)> = Vec::new();

    let nav_href = items
        .iter()
        .find(|(_, _, meta)| meta.split('\u{1}').next().unwrap_or("").contains("nav"))
        .map(|(_, href, _)| href.clone());
    if let Some(nav) = nav_href {
        let full = norm_path(opf_dir, &nav);
        if let Some(bytes) = zip_read(zip, ci, &full) {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(seg) = find_toc_nav_section(&html) {
                pairs = extract_anchor_pairs(&seg, &full);
            }
        }
    }

    if pairs.is_empty() {
        let ncx_href = items
            .iter()
            .find(|(id, _, _)| id.eq_ignore_ascii_case("ncx"))
            .map(|(_, href, _)| href.clone())
            .or_else(|| {
                items
                    .iter()
                    .find(|(_, _, meta)| {
                        let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
                        mtype.contains("ncx")
                    })
                    .map(|(_, href, _)| href.clone())
            })
            .or_else(|| {
                zip.file_names()
                    .find(|n| n.to_lowercase().ends_with(".ncx"))
                    .map(|s| s.to_string())
            });
        if let Some(ncx) = ncx_href {
            let full = norm_path(opf_dir, &ncx);
            if let Some(bytes) = zip_read(zip, ci, &full) {
                let xml = String::from_utf8_lossy(&bytes).into_owned();
                pairs = extract_ncx_pairs(&xml, &full);
            }
        }
    }

    fn spine_pos(spine: &[String], target: &str) -> Option<usize> {
        if let Some(i) = spine.iter().position(|s| s == target) {
            return Some(i);
        }
        let t = target.replace('\\', "/").to_lowercase();
        let tb = t.rsplit('/').next().unwrap_or(&t);
        spine.iter().position(|s| {
            let sl = s.replace('\\', "/").to_lowercase();
            sl == t || sl.rsplit('/').next().unwrap_or(&sl) == tb
        })
    }

    let mut toc: Vec<TocEntry> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (text, href) in pairs {
        let target = norm_path(opf_dir, &href);
        if let Some(idx) = spine_pos(spine, &target) {
            if !used.contains(&idx) {
                used.push(idx);
                toc.push(TocEntry {
                    title: text.chars().take(80).collect(),
                    index: idx,
                    chars: spine_chars.get(idx).map(|c| c.0).unwrap_or(0),
                });
            }
        }
    }
    if toc.is_empty() {
        for (i, doc) in spine.iter().enumerate() {
            let name = doc.rsplit('/').next().unwrap_or(doc);
            toc.push(TocEntry {
                title: format!("{fallback} - {}", name),
                index: i,
                chars: spine_chars.get(i).map(|c| c.0).unwrap_or(0),
            });
        }
    }
    toc
}

fn find_toc_nav_section(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut search_from = 0;
    while let Some(pos) = lower[search_from..].find("<nav") {
        let abs = search_from + pos;
        let end = lower[abs..].find("</nav>").map(|e| abs + e + 6);
        if let Some(end) = end {
            let seg = &html[abs..end];
            if seg.to_lowercase().contains("toc") || seg.contains("<h") {
                return Some(seg.to_string());
            }
            search_from = end;
        } else {
            break;
        }
    }
    None
}

fn extract_anchor_pairs(html: &str, base_doc: &str) -> Vec<(String, String)> {
    let re = Regex::new(r#"(?is)<a\s[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    let mut out = Vec::new();
    for c in re.captures_iter(html) {
        let href = c[1].to_string();
        let inner = tag_re.replace_all(&c[2], "");
        let text = inner
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_string();
        if text.is_empty() || href.starts_with("http") {
            continue;
        }
        out.push((text, href));
    }
    let _ = base_doc;
    out
}

fn extract_ncx_pairs(xml: &str, base_doc: &str) -> Vec<(String, String)> {
    let re = Regex::new(
        r#"(?is)<navPoint[^>]*>.*?<text[^>]*>(.*?)</text>.*?<content[^>]*src\s*=\s*["']([^"']+)["']"#,
    )
    .unwrap();
    let mut out = Vec::new();
    for c in re.captures_iter(xml) {
        let text = c[1]
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_string();
        out.push((text, c[2].to_string()));
    }
    let _ = base_doc;
    out
}

pub fn resolve_in_doc(doc_path: &str, href: &str) -> String {
    norm_path(doc_path, href)
}

pub fn chapter_html(zip_bytes: &[u8], doc_path: &str) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let ci = build_ci(&mut zip);
    let bytes = zip_read(&mut zip, &ci, doc_path).ok_or("章节文件缺失")?;
    let html = String::from_utf8_lossy(&bytes).into_owned();

    let body = extract_body(&html);

    let css_link = Regex::new(r#"(?is)<link[^>]+rel\s*=\s*["']?stylesheet["']?[^>]*>"#).unwrap();
    let mut styled = css_link.replace_all(&body, "").into_owned();
    let style_re = Regex::new(r#"(?is)<style[^>]*>.*?</style>"#).unwrap();
    styled = style_re.replace_all(&styled, "").into_owned();
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    styled = script_re.replace_all(&styled, "").into_owned();

    let img_re = Regex::new(
        r#"(?i)(src|xlink:href|href)\s*=\s*"([^"]+\.(?:png|jpe?g|gif|bmp|webp|svg))""#,
    )
    .unwrap();
    styled = img_re
        .replace_all(&styled, |caps: &regex::Captures| {
            let attr = caps[1].to_string();
            let target = resolve_in_doc(doc_path, &caps[2]);
            if let Some(bytes) = zip_read(&mut zip, &ci, &target) {
                let mime = match target.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "bmp" => "image/bmp",
                    "webp" => "image/webp",
                    "svg" => "image/svg+xml",
                    _ => "application/octet-stream",
                };
                format!("{attr}=\"data:{mime};base64,{}\"", crate::covers::b64(&bytes))
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();

    let css_url_re =
        Regex::new(r#"(?i)url\(\s*["']?([^"')]+\.(?:png|jpe?g|gif|bmp|webp|svg))["']?\s*\)"#)
            .unwrap();
    styled = css_url_re
        .replace_all(&styled, |caps: &regex::Captures| {
            let target = resolve_in_doc(doc_path, &caps[1]);
            if let Some(bytes) = zip_read(&mut zip, &ci, &target) {
                let mime = match target.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "bmp" => "image/bmp",
                    "webp" => "image/webp",
                    "svg" => "image/svg+xml",
                    _ => "application/octet-stream",
                };
                format!("url(data:{mime};base64,{})", crate::covers::b64(&bytes))
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();

    // 去掉内联 style，保证阅读器字体/颜色与用户自定义 CSS 生效
    let style_attr_re =
        Regex::new(r#"(?is)\sstyle\s*=\s*"[^"]*"|\sstyle\s*=\s*'[^']*'"#).unwrap();
    styled = style_attr_re.replace_all(&styled, " ").into_owned();

    Ok(format!(
        "<div class=\"epub-doc\" data-doc=\"{}\">{}</div>",
        escape_html(doc_path),
        styled
    ))
}

fn extract_body(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<body") {
        let after = lower[start..].find('>').map(|i| start + i + 1).unwrap_or(start);
        let end = lower.rfind("</body>").unwrap_or(html.len());
        if after < end {
            return html[after..end].to_string();
        }
    }
    html.to_string()
}

pub fn doc_text(zip_bytes: &[u8], doc_path: &str) -> String {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return String::new(),
    };
    let ci = build_ci(&mut zip);
    let bytes = match zip_read(&mut zip, &ci, doc_path) {
        Some(b) => b,
        None => return String::new(),
    };
    let html = String::from_utf8_lossy(&bytes).into_owned();
    html_to_text(&html)
}

pub fn html_to_text(html: &str) -> String {
    let body = extract_body(html);
    let block_end = Regex::new(r"(?i)</(?:p|div|h[1-6]|li|tr|blockquote|section)>").unwrap();
    let br = Regex::new(r"(?i)<br\s*/?>").unwrap();
    let tag = Regex::new(r"(?s)<[^>]*>").unwrap();
    let s = block_end.replace_all(&body, "\n").into_owned();
    let s = br.replace_all(&s, "\n").into_owned();
    let s = tag.replace_all(&s, "").into_owned();
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

pub fn read_cover(zip_bytes: &[u8], cover_href: &str) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let ci = build_ci(&mut zip);
    zip_read(&mut zip, &ci, cover_href)
}

pub fn find_cover_fallback(zip_bytes: &[u8], spine: &[String]) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let ci = build_ci(&mut zip);

    let img_re =
        Regex::new(r#"(?i)<img[^>]+(?:src|xlink:href)\s*=\s*["']([^"']+\.(?:png|jpe?g|gif|bmp|webp))["']"#)
            .ok()?;

    let mut candidates: Vec<String> = Vec::new();
    for doc in spine.iter().take(4) {
        if let Some(bytes) = zip_read(&mut zip, &ci, doc) {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            for c in img_re.captures_iter(&html) {
                candidates.push(resolve_in_doc(doc, &c[1]));
            }
            if !candidates.is_empty() {
                break;
            }
        }
    }

    for name in candidates {
        if let Some(bytes) = zip_read(&mut zip, &ci, &name) {
            if bytes.len() > 4096 {
                return Some(bytes);
            }
        }
    }

    let mut big: Option<(u64, Vec<u8>)> = None;
    for i in 0..zip.len() {
        if let Ok(mut f) = zip.by_index(i) {
            let name = f.name().to_lowercase();
            if (name.ends_with(".jpg") || name.ends_with(".jpeg") || name.ends_with(".png"))
                && (name.contains("cover") || f.size() > 60_000)
            {
                let size = f.size();
                if big.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
                    let mut buf = Vec::with_capacity(size as usize);
                    std::io::Read::read_to_end(&mut f, &mut buf).ok();
                    big = Some((size, buf));
                }
            }
        }
    }
    big.map(|(_, b)| b)
}

pub fn spine_docs_with_titles(zip_bytes: &[u8], spine: &[String]) -> Vec<(String, String)> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return spine.iter().map(|s| (String::new(), s.clone())).collect(),
    };
    let ci = build_ci(&mut zip);
    let h_re = Regex::new(r"(?is)<h[1-3][^>]*>(.*?)</h[1-3]>").unwrap();
    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    spine
        .iter()
        .map(|doc| {
            let title = zip_read(&mut zip, &ci, doc)
                .map(|b| {
                    let html = String::from_utf8_lossy(&b).into_owned();
                    h_re.captures(&html)
                        .map(|c| {
                            tag_re
                                .replace_all(&c[1], "")
                                .trim()
                                .chars()
                                .take(60)
                                .collect::<String>()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let name = doc.rsplit('/').next().unwrap_or(doc).to_string();
            let title = if title.is_empty() { name } else { title };
            (title, doc.clone())
        })
        .collect()
}
