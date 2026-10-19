use crate::models::TocEntry;
use crate::txt::escape_html;
use regex::Regex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub struct EpubDoc {
    pub title: String,
    pub author: String,
    pub spine: Vec<String>,
    pub toc: Vec<TocEntry>,
    /// 每个目录项对应的章节在 spine 中的结束下标（不含）。有些 EPUB 把一个章节拆成
    /// 多个小文件、目录只指向首个小文件，必须靠这个区间把整章内容合起来。
    pub toc_end: Vec<usize>,
    pub cover_href: Option<String>,
    pub spine_chars: Vec<i64>,
    pub total_chars: i64,
    pub total_text_chars: i64,
    /// OPF manifest 中的图片资源（zip 内绝对路径，按声明顺序）。供图片查看器快速列举。
    pub image_hrefs: Vec<String>,
}

pub type CiMap = HashMap<String, usize>;

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

/// 解码文本，兼容 UTF-8 与带 BOM 的 UTF-16（部分工具生成的 XML 是 UTF-16）。
fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return decode_utf16(bytes, true);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return decode_utf16(bytes, false);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_utf16(bytes: &[u8], little: bool) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2);
    let mut i = 2; // 跳过 BOM
    while i + 1 < bytes.len() {
        let v = if little {
            u16::from_le_bytes([bytes[i], bytes[i + 1]])
        } else {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        };
        units.push(v);
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

/// 从 container.xml 文本里取出 rootfile 的 full-path，兼容单引号/双引号（原来只认双引号，
/// 导致 `full-path='...'` 的 EPUB 报「container.xml 中未找到 OPF」）。
fn opf_path_from_container(container: &str) -> Option<String> {
    let re_d = Regex::new(r#"(?i)full-path\s*=\s*"([^"]+)""#).ok()?;
    if let Some(c) = re_d.captures(container) {
        return Some(c[1].to_string());
    }
    let re_s = Regex::new(r#"(?i)full-path\s*=\s*'([^']+)'"#).ok()?;
    re_s.captures(container).map(|c| c[1].to_string())
}

/// container.xml 缺失/损坏时的兜底：在压缩包里找一个 .opf（优先 content.opf）。
fn find_opf_in_zip(ci: &CiMap) -> Option<String> {
    let mut fallback: Option<String> = None;
    for k in ci.keys() {
        if k.starts_with("b:") {
            continue;
        }
        let path = k.strip_prefix("./").unwrap_or(k);
        if !path.ends_with(".opf") {
            continue;
        }
        if path.ends_with("/content.opf") || path == "content.opf" {
            return Some(path.to_string());
        }
        if fallback.is_none() {
            fallback = Some(path.to_string());
        }
    }
    fallback
}

fn zip_has(ci: &CiMap, name: &str) -> bool {
    resolve_idx(ci, name).is_some()
}

/// 判断是否受 DRM/加密保护（掌阅 zhangyue、或其他声明了 encryption.xml 的加密书）。
fn is_encrypted(ci: &CiMap) -> bool {
    zip_has(ci, "META-INF/encryption.xml") || zip_has(ci, "META-INF/zhangyue-expansion.xml")
}

fn looks_like_opf(s: &str) -> bool {
    let low = s.to_ascii_lowercase();
    low.contains("<package") || low.contains("<opf:package")
}

const ERR_ENCRYPTED: &str =
    "这本 EPUB 是加密的，需要官方阅读器才能打开；请获取无 DRM 的版本后再导入。";

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

/// 快速解析：只解析 container/OPF/目录/封面，**不做全书逐章文本统计**。
/// 打开书籍与取章节走这里，避免 7000+ 章的书在打开时卡 5 秒以上。
/// 需要全书字数时用 `parse_with_chars`（后台元数据阶段）。
/// 接收 `&[u8]` 借用（内部 `Cursor` 也只借用），调用方无需为解析复制整份书字节。
pub fn parse(data: &[u8]) -> Result<EpubDoc, String> {
    parse_impl(data, false).map(|(doc, _)| doc)
}

/// 完整解析：额外统计每一章的字符数（供全书字数/总页数估算，后台执行）。
/// 打开书籍与取章节走快速路径（`parse` / `parse_with_ci`），不统计字数。
pub fn parse_with_chars(data: &[u8]) -> Result<EpubDoc, String> {
    parse_impl(data, true).map(|(doc, _)| doc)
}

/// 全书字数统计：`(每章字数, 全书字数 total_chars, 纯字数 total_text_chars)`。
///
/// `total / pure` 必须取 `EpubDoc` 的 spine 逐文档求和，**不能把 toc 的字数求和**：
/// 同一个 spine 文档被多个目录项指向时会被重复计数；toc 数组只服务「全书总页数」的按章估算。
///
/// 为什么单独走一次完整解析：快速解析刻意不做全书逐章统计（数千章可达数秒），
/// 于是 toc 里的 chars 与 total_chars 全是 0。调用方（书详情/本书统计、全书总页数）
/// 按需补算一次，并共享同一份缓存。
pub fn epub_word_stats(data: &[u8]) -> Result<(Vec<i64>, i64, i64), String> {
    let doc = parse_with_chars(data)?;
    let per_chapter = doc.toc.iter().map(|t| t.chars).collect();
    Ok((per_chapter, doc.total_chars, doc.total_text_chars))
}



/// 快速解析并**同时返回条目索引**（避免调用方再扫一遍 zip 建 CiMap）。
pub fn parse_with_ci(data: &[u8]) -> Result<(EpubDoc, CiMap), String> {
    parse_impl(data, false)
}

fn parse_impl(data: &[u8], compute_chars: bool) -> Result<(EpubDoc, CiMap), String> {
    let cursor = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("EPUB 打开失败: {e}"))?;
    let ci = build_ci(&mut zip);

    let container = zip_read(&mut zip, &ci, "META-INF/container.xml")
        .ok_or("缺少 META-INF/container.xml")?;
    let container_str = decode_text(&container);
    let opf_path = opf_path_from_container(&container_str)
        .or_else(|| find_opf_in_zip(&ci))
        .ok_or_else(|| {
            if is_encrypted(&ci) {
                ERR_ENCRYPTED.to_string()
            } else {
                "container.xml 中未找到 OPF，且压缩包内没有 .opf 文件".to_string()
            }
        })?;

    let opf_bytes = zip_read(&mut zip, &ci, &opf_path).ok_or("OPF 文件缺失")?;
    let opf = decode_text(&opf_bytes);
    if !looks_like_opf(&opf) {
        return Err(if is_encrypted(&ci) {
            ERR_ENCRYPTED.to_string()
        } else {
            "OPF 内容无法解析（文件可能已加密或损坏）".to_string()
        });
    }
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

    // OPF manifest 里的图片资源（按声明顺序），供图片查看器快速列举（无需扫描全部章节）。
    let image_hrefs: Vec<String> = items
        .iter()
        .filter(|(_, _, meta)| {
            let (_, mtype) = meta.split_once('\u{1}').unwrap_or(("", ""));
            mtype.starts_with("image")
        })
        .map(|(_, href, _)| norm_path(&opf_dir, href))
        .collect();

    let spine_idx: Vec<Option<usize>> = spine.iter().map(|s| resolve_idx(&ci, s)).collect();
    let nworkers = rayon::current_num_threads().max(1);
    // 全书逐章文本统计（7000+ 章时可达数秒）。仅在需要字数的后台元数据阶段执行；
    // 打开书籍/取章节的快速路径跳过，保证首屏尽快出现。
    let spine_chars: Vec<(i64, i64)> = if !compute_chars {
        vec![(0i64, 0i64); spine_idx.len()]
    } else {
        let counter = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new(vec![(0i64, 0i64); spine_idx.len()]);
        let spine_idx_ref: &[Option<usize>] = &spine_idx;
        let data_ref: &[u8] = data;
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

    // 每章 = 从本章目录项指向的 spine 下标，直到下一个目录项（或 spine 末尾）。
    // 这样「一章拆成多个小文件」的 EPUB 也能显示完整正文。
    let toc_end: Vec<usize> = (0..toc.len())
        .map(|i| {
            let start = toc[i].index;
            let next = toc.get(i + 1).map(|t| t.index).unwrap_or(spine.len());
            next.max(start + 1).min(spine.len())
        })
        .collect();

    let total_chars: i64 = spine_chars.iter().map(|c| c.0).sum();
    let total_text_chars: i64 = spine_chars.iter().map(|c| c.1).sum();

    Ok((
        EpubDoc {
            title,
            author,
            spine,
            toc,
            toc_end,
            cover_href,
            spine_chars: spine_chars.iter().map(|c| c.0).collect(),
            total_chars,
            total_text_chars,
            image_hrefs,
        },
        ci,
    ))
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

/// 从 OPF 文本里找封面图片的 href（不解析全书）：
///   1) `<meta name="cover" content="ID">` → 找同 ID 的 `<item ... href=...>`；
///   2) 退而求其次：`<item properties="cover-image" href=...>`。
fn cover_href_from_opf(opf: &str) -> Option<String> {
    if let Ok(re) = Regex::new(
        r#"(?is)<meta[^>]*\bname\s*=\s*["']cover["'][^>]*\bcontent\s*=\s*["']([^"']+)["']"#,
    ) {
        if let Some(c) = re.captures(opf) {
            let id = c[1].to_string();
            let esc = regex::escape(&id);
            if let Ok(re2) = Regex::new(&format!(
                r#"(?is)<item[^>]*\bid\s*=\s*["']{esc}["'][^>]*\bhref\s*=\s*["']([^"']+)["']"#
            )) {
                if let Some(c2) = re2.captures(opf) {
                    return Some(c2[1].to_string());
                }
            }
            if let Ok(re3) = Regex::new(&format!(
                r#"(?is)<item[^>]*\bhref\s*=\s*["']([^"']+)["'][^>]*\bid\s*=\s*["']{esc}["']"#
            )) {
                if let Some(c3) = re3.captures(opf) {
                    return Some(c3[1].to_string());
                }
            }
        }
    }
    if let Ok(re) = Regex::new(
        r#"(?is)<item\b[^>]*\bproperties\s*=\s*["'][^"']*cover-image[^"']*["'][^>]*>"#,
    ) {
        if let Some(m) = re.find(opf) {
            if let Ok(hre) = Regex::new(r#"(?is)\bhref\s*=\s*["']([^"']+)["']"#) {
                if let Some(c) = hre.captures(m.as_str()) {
                    return Some(c[1].to_string());
                }
            }
        }
    }
    None
}

/// **封面提取前的 EOCD 预检**：只读文件尾最多 1 KiB 定位 EOCD（`PK\x05\x06`），
/// 取出中央目录大小；CD > 8 MiB 或找不到 EOCD 即返回 Err，**不进入 `ZipArchive::new`**。
/// 用于在解析前挡住畸形/超大中央目录的文件（否则 `ZipArchive::new` 可能长时间不返回）。
/// 该函数只读 EOCD 搜索范围（≤1 KiB），不读其它字节。
pub fn precheck_eocd(path: &str) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_CD_BYTES: u64 = 8 * 1024 * 1024;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len < 22 {
        return Err("文件过小，不是有效 ZIP/EPUB".to_string());
    }
    let read_n = len.min(1024);
    f.seek(SeekFrom::End(-(read_n as i64))).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; read_n as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    // 在尾部 ≤1KiB 内找最后一个 EOCD 签名。
    let sig = [0x50u8, 0x4b, 0x05, 0x06];
    let pos = buf
        .windows(4)
        .rposition(|w| w == sig)
        .ok_or("尾部 1KiB 内未找到 EOCD 签名")?;
    if pos + 16 > buf.len() {
        return Err("EOCD 记录被截断".to_string());
    }
    // EOCD +12：中央目录大小(u32 LE)。
    let cd_size = u32::from_le_bytes([buf[pos + 12], buf[pos + 13], buf[pos + 14], buf[pos + 15]]) as u64;
    if cd_size > MAX_CD_BYTES {
        return Err(format!("中央目录过大: {cd_size} 字节 (> 8 MiB)"));
    }
    Ok(())
}

/// **解析阶段**的读取预算上限：字节数与 read 调用次数（两者任一耗尽即判为病态）。
///
/// 背景：zip 2.4.x 的 `get_metadata` 在「中央目录某条目解析失败」时会回退为
/// **在全文件内向前搜索更早的 EOCD 候选**，以 2 KiB 窗口回扫整个文件——对若干
/// 局部头偏移指向垃圾/零区的畸形条目，会读取数百 MiB 且大量 read 调用、长时间不返回。
/// 仅靠 EOCD 预检（CD 大小 > 8 MiB / 无 EOCD）无法拦住这类文件（其 CD 可能只有几百 KiB）。
///
/// 上限取值由本模块 `budget_probe` 的实测决定（正常书解析期消耗见下）：
/// - **字节**：正常 EPUB 解析期仅读 CD + 各条目局部头，实测 18 MB 书 10–312 KB、
///   100 MB 书 33–154 KB、434 MB 书 1.1 MB；病态回扫读数百 MiB。故 8 MiB 有 >7x 余量。
/// - **调用次数**：正常书随条目数增长，实测 18 MB 书 270–9684 次、124 MB 书 4806 次、
///   147 MB 书 5394 次、434 MB 书 **34683** 次。**初值 5000 会误伤正常书**，故按实测
///   校准为 200000（约 5.8x 余量）；病态文件则由 8 MiB 字节上限更早拦下。
pub const MAX_PARSE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PARSE_CALLS: u64 = 200_000;

/// worker 与看门狗共享的读取预算/取消令牌。
pub struct CoverReadBudget {
    // staged: 解析预算计量基准；仅 TP_EPUB_COVER cargo test cover_budget_bench 读取（校准 MAX_PARSE_CALLS）。
    #[allow(dead_code)]
    bytes_limit: u64,
    #[allow(dead_code)]
    calls_limit: u64,
    bytes_left: AtomicU64,
    calls_left: AtomicU64,
    /// 解析阶段为 true（执行上限）；`finish_parse()` 后为 false（只保留取消）。
    limiting: AtomicBool,
    /// 预算（字节或调用次数）被 worker 自身耗尽。
    exhausted: AtomicBool,
    /// 看门狗超时取消。
    cancelled: AtomicBool,
}

impl CoverReadBudget {
    pub fn new(bytes_limit: u64, calls_limit: u64) -> Self {
        Self {
            bytes_limit,
            calls_limit,
            bytes_left: AtomicU64::new(bytes_limit),
            calls_left: AtomicU64::new(calls_limit),
            limiting: AtomicBool::new(true),
            exhausted: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }

    /// 解析成功：解除上限（封面数据读取不受限），仅保留取消语义。
    fn finish_parse(&self) {
        self.limiting.store(false, Ordering::SeqCst);
    }

    /// 看门狗超时：令 worker 内下一次 read 立即失败退出（消除线程泄漏）。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 预算是否被耗尽（病态文件的判据，由 worker 侧读取）。
    pub fn exhausted(&self) -> bool {
        self.exhausted.load(Ordering::SeqCst)
    }

    /// 解析阶段已消耗的 read 调用次数 / 字节数（供计量）。
    /// staged: 仅 #[test] cover_budget_bench 诊断用；与上面两字段四者同加 #[allow]。
    #[allow(dead_code)]
    pub fn calls_used(&self) -> u64 {
        self.calls_limit.saturating_sub(self.calls_left.load(Ordering::SeqCst))
    }
    #[allow(dead_code)]
    pub fn bytes_used(&self) -> u64 {
        self.bytes_limit.saturating_sub(self.bytes_left.load(Ordering::SeqCst))
    }
}

impl Default for CoverReadBudget {
    fn default() -> Self {
        Self::new(MAX_PARSE_BYTES, MAX_PARSE_CALLS)
    }
}

/// 带读取预算的 reader：解析阶段超过字节/调用上限即返回 io::Error（中止 zip 解析）。
/// 预算为共享的 `Arc<CoverReadBudget>`：看门狗可 `cancel()` 取消。
struct BudgetReader<R> {
    inner: R,
    budget: Arc<CoverReadBudget>,
}

impl<R> BudgetReader<R> {
    fn new(inner: R, budget: Arc<CoverReadBudget>) -> Self {
        Self { inner, budget }
    }
}

impl<R: Read> Read for BudgetReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.budget.cancelled.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "cover read cancelled",
            ));
        }
        let limiting = self.budget.limiting.load(Ordering::SeqCst);
        if limiting {
            let c = self.budget.calls_left.load(Ordering::SeqCst);
            if c == 0 {
                self.budget.exhausted.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "cover read call limit exceeded",
                ));
            }
            self.budget.calls_left.store(c - 1, Ordering::SeqCst);
        }
        let n = self.inner.read(buf)?;
        if limiting {
            let b = self.budget.bytes_left.load(Ordering::SeqCst);
            if n as u64 > b {
                self.budget.bytes_left.store(0, Ordering::SeqCst);
                self.budget.exhausted.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "cover read byte budget exceeded",
                ));
            }
            self.budget.bytes_left.store(b - n as u64, Ordering::SeqCst);
        }
        Ok(n)
    }
}

impl<R: std::io::Seek> std::io::Seek for BudgetReader<R> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// **只提取封面**：以 `File`(Seek) 打开 zip，仅读取中央目录 + `META-INF/container.xml`
/// + OPF + 封面图片三个条目，其余条目一律不解压（不读全文）。返回封面图字节（不含任何元数据）。
/// `budget` 为共享读取预算/取消令牌（看门狗可 cancel 取消）。
pub fn extract_cover_file(path: &str, budget: &Arc<CoverReadBudget>) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    extract_cover_from_reader(std::io::BufReader::new(file), budget.clone())
}

/// 泛型版（供计量测试用 counting reader 精确统计读取字节数）。
pub fn extract_cover_from_reader<R: Read + std::io::Seek>(
    reader: R,
    budget: Arc<CoverReadBudget>,
) -> Result<Option<Vec<u8>>, String> {
    // 解析阶段带字节/调用上限：拦住 zip 2.4.x 因中央目录条目损坏而回扫整文件的病态读取。
    let mut zip = zip::ZipArchive::new(BudgetReader::new(reader, budget.clone()))
        .map_err(|e| format!("EPUB 打开失败: {e}"))?;
    // 解析成功：解除上限，封面数据（可达 32 MiB）读取不受限；取消语义仍保留。
    budget.finish_parse();
    // 仅读中央目录建立「小写名→索引」表（by_index_raw 不解压条目数据）。
    let mut idx_of: HashMap<String, usize> = HashMap::new();
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index_raw(i) {
            idx_of.insert(f.name().trim_start_matches("./").to_ascii_lowercase(), i);
        }
    }
    let read_entry = |zip: &mut zip::ZipArchive<BudgetReader<R>>, name: &str| -> Option<Vec<u8>> {
        let key = name.trim_start_matches("./").to_ascii_lowercase();
        let i = *idx_of.get(&key)?;
        let mut f = zip.by_index(i).ok()?;
        let mut b = Vec::new();
        f.read_to_end(&mut b).ok()?;
        Some(b)
    };
    let container = match read_entry(&mut zip, "META-INF/container.xml") {
        Some(b) => decode_text(&b),
        None => return Ok(None),
    };
    let opf_path = match opf_path_from_container(&container) {
        Some(p) => p,
        None => return Ok(None),
    };
    let opf = match read_entry(&mut zip, &opf_path) {
        Some(b) => decode_text(&b),
        None => return Ok(None),
    };
    let href = match cover_href_from_opf(&opf) {
        Some(h) => h,
        None => return Ok(None),
    };
    let full = norm_path(&opf_path, &href);
    Ok(read_entry(&mut zip, &full).filter(|b| !b.is_empty()))
}

/// **只提取封面（CBZ=图片 zip）**：seek 打开 zip，按条目名排序取第一张图片，仅解压该条目。
pub fn extract_cbz_cover_file(path: &str, budget: &Arc<CoverReadBudget>) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(BudgetReader::new(std::io::BufReader::new(file), budget.clone()))
        .map_err(|e| e.to_string())?;
    budget.finish_parse(); // 解析成功：解除上限
    let mut imgs: Vec<(String, usize)> = Vec::new();
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index_raw(i) {
            let n = f.name().to_string();
            let l = n.to_ascii_lowercase();
            if l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png")
                || l.ends_with(".webp") || l.ends_with(".gif")
            {
                imgs.push((n, i));
            }
        }
    }
    if imgs.is_empty() {
        return Ok(None);
    }
    imgs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut b = Vec::new();
    zip.by_index(imgs[0].1)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    Ok(if b.is_empty() { None } else { Some(b) })
}


/// 按已缓存的 `CiMap` 读取 zip 内任意条目（供按需图片服务使用）。
pub fn read_entry(zip_bytes: &[u8], ci: &CiMap, name: &str) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    zip_read(&mut zip, ci, name)
}

/// 供协议处理器的 MIME 判断。
pub fn mime_for_path(target: &str) -> &'static str {
    mime_of(target)
}

/// 从图片字节头部解析宽高（webp/jpeg/png/gif/bmp），用于在正文里预置 `width/height`
/// 属性，避免图片解码后布局跳变引发反复重排。
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 16 {
        return None;
    }
    // PNG
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) && bytes.len() >= 24 {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((w, h));
    }
    // GIF
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some((w, h));
    }
    // BMP
    if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        let w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]).unsigned_abs();
        let h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]).unsigned_abs();
        return Some((w, h));
    }
    // WebP
    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let fourcc = &bytes[12..16];
        if fourcc == b"VP8X" {
            let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            return Some((w, h));
        }
        if fourcc == b"VP8 " && bytes.len() >= 30 {
            let w = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3FFF;
            let h = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3FFF;
            return Some((w, h));
        }
        if fourcc == b"VP8L" && bytes.len() >= 25 {
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            return Some((w, h));
        }
    }
    // JPEG：扫描 SOF 段
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&marker)
                && marker != 0xC4
                && marker != 0xC8
                && marker != 0xCC
            {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

/// 可复用外部 `CiMap`（避免每章重跑 build_ci）与按需图片 URL（book_id=Some 时不内联 base64）。
/// `keep_styles=true` 时保留原书样式表（作用域隔离后内联），否则剥离全部原书 CSS。
pub fn chapter_html_ci(
    zip_bytes: &[u8],
    ci: Option<&CiMap>,
    doc_path: &str,
    book_id: Option<i64>,
    keep_styles: bool,
) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let owned;
    let ci = match ci {
        Some(c) => c,
        None => {
            owned = build_ci(&mut zip);
            &owned
        }
    };
    let bytes = zip_read(&mut zip, ci, doc_path).ok_or("章节文件缺失")?;
    let html = String::from_utf8_lossy(&bytes).into_owned();
    Ok(render_doc_html(&mut zip, ci, doc_path, &html, book_id, keep_styles))
}

/// 带外部 `CiMap` 与按需图片 URL 的多文件章节渲染。
pub fn chapter_html_multi_ci(
    zip_bytes: &[u8],
    ci: Option<&CiMap>,
    doc_paths: &[String],
    book_id: Option<i64>,
    keep_styles: bool,
) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let owned;
    let ci = match ci {
        Some(c) => c,
        None => {
            owned = build_ci(&mut zip);
            &owned
        }
    };
    let mut out = String::new();
    for doc_path in doc_paths {
        if let Some(bytes) = zip_read(&mut zip, ci, doc_path) {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            out.push_str(&render_doc_html(&mut zip, ci, doc_path, &html, book_id, keep_styles));
        }
    }
    Ok(out)
}

/// 生成按需图片 URL（不内联 base64，避免单章 HTML 达到几百 KB）。
/// 形如 `http://tidepage.localhost/epubimg/<book_id>/<b64url(zip内路径)>`。
pub fn epub_image_url(book_id: i64, target: &str) -> String {
    use base64::Engine;
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(target.as_bytes());
    format!("http://tidepage.localhost/epubimg/{book_id}/{enc}")
}

/// 把单个 XHTML 文档渲染为可内联进阅读器的 HTML 片段（复用已打开的 zip 与索引）。
/// `book_id=Some` 时图片改写为按需 URL（懒加载）；`None` 时回退 base64 内联。
fn render_doc_html<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    doc_path: &str,
    html: &str,
    book_id: Option<i64>,
    keep_styles: bool,
) -> String {
    let body = extract_body(html);

    // 原书样式：keep_styles=true 时把 <head> 的样式表与文档内 <style> 作用域隔离后内联，
    // 否则整体剥离（由阅读器「覆盖书籍排版」等全局设置决定版式）。
    let mut book_css = String::new();
    if keep_styles {
        for href in head_stylesheet_hrefs(html) {
            let target = resolve_in_doc(doc_path, &href);
            let mut visited = std::collections::HashSet::new();
            load_css_recursive(zip, ci, &target, book_id, &mut visited, &mut book_css, 0);
        }
        for css in inline_style_blocks(html) {
            book_css.push_str(&rewrite_css_urls(&css, doc_path, zip, ci, book_id));
            book_css.push('\n');
        }
    }

    // 统一净化：移除危险标签（连内容，如 script/iframe/object/form/link/style 等）
    // 与危险属性（on*、javascript: 等），同时保留合法 <img src="...jpg">、<a href>、
    // style 等，供下方按需图片 URL 改写与 keep_styles 逻辑继续处理。
    let mut styled = crate::sanitize::sanitize_fragment(&body);

    // 先处理整段 <img ...>：改写 src 为按需 URL，并预置 width/height/loading/decoding，
    // 让浏览器在图片解码前就占好位，避免布局跳变与反复重排（卡死根因之一）。
    let tag_re = Regex::new(r#"(?is)<img\b[^>]*>"#).unwrap();
    styled = tag_re
        .replace_all(&styled, |caps: &regex::Captures| {
            process_img_tag(&caps[0], zip, ci, doc_path, book_id)
        })
        .into_owned();

    // 再处理非 <img> 的图片引用（如 SVG <image xlink:href>、<a href="x.jpg">）。
    let img_re = Regex::new(
        r#"(?i)(src|xlink:href|href)\s*=\s*"([^"]+\.(?:png|jpe?g|gif|bmp|webp|svg))""#,
    )
    .unwrap();
    styled = img_re
        .replace_all(&styled, |caps: &regex::Captures| {
            let attr = caps[1].to_string();
            let target = resolve_in_doc(doc_path, &caps[2]);
            match book_id {
                Some(bid) => format!("{attr}=\"{}\"", epub_image_url(bid, &target)),
                None => {
                    if let Some(bytes) = zip_read(zip, ci, &target) {
                        let mime = mime_of(&target);
                        format!("{attr}=\"data:{mime};base64,{}\"", crate::covers::b64(&bytes))
                    } else {
                        caps[0].to_string()
                    }
                }
            }
        })
        .into_owned();

    let css_url_re =
        Regex::new(r#"(?i)url\(\s*["']?([^"')]+\.(?:png|jpe?g|gif|bmp|webp|svg))["']?\s*\)"#)
            .unwrap();
    styled = css_url_re
        .replace_all(&styled, |caps: &regex::Captures| {
            let target = resolve_in_doc(doc_path, &caps[1]);
            match book_id {
                Some(bid) => {
                    if resolve_idx(ci, &target).is_some() {
                        format!("url({})", epub_image_url(bid, &target))
                    } else {
                        caps[0].to_string()
                    }
                }
                None => {
                    if let Some(bytes) = zip_read(zip, ci, &target) {
                        let mime = mime_of(&target);
                        format!("url(data:{mime};base64,{})", crate::covers::b64(&bytes))
                    } else {
                        caps[0].to_string()
                    }
                }
            }
        })
        .into_owned();

    // 关闭「覆盖书籍排版」时保留内联 style（属于原书排版的一部分）；
    // 开启时去掉内联 style，保证阅读器字体/颜色与用户自定义 CSS 生效。
    if !keep_styles {
        let style_attr_re =
            Regex::new(r#"(?is)\sstyle\s*=\s*"[^"]*"|\sstyle\s*=\s*'[^']*'"#).unwrap();
        styled = style_attr_re.replace_all(&styled, " ").into_owned();
    }

    // body 的 class（如 body.cip）迁移到包裹层，保证原书 body 级样式仍能命中。
    let body_cls = body_classes(html);
    let mut classes = String::from("epub-doc");
    if !body_cls.is_empty() {
        classes.push(' ');
        classes.push_str(&body_cls);
    }

    let mut style_tag = String::new();
    if keep_styles && !book_css.trim().is_empty() {
        let scoped = scope_book_css(&book_css);
        if !scoped.trim().is_empty() {
            style_tag = format!("<style class=\"tp-book-css\">{scoped}</style>");
        }
    }

    format!(
        "<div class=\"{}\" data-doc=\"{}\">{}{}</div>",
        escape_html(&classes),
        escape_html(doc_path),
        style_tag,
        styled
    )
}

/// 取出 `<head>...</head>` 之间的内容（原书样式表引用都在这里）。
fn extract_head(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<head") {
        let after = lower[start..].find('>').map(|i| start + i + 1).unwrap_or(start);
        let end = lower[after..].find("</head>").map(|i| after + i).unwrap_or(html.len());
        if after < end {
            return html[after..end].to_string();
        }
    }
    String::new()
}

/// 文档 `<head>` 中所有 `<link rel=stylesheet>` 的 href（原样，未解析）。
fn head_stylesheet_hrefs(html: &str) -> Vec<String> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    static HREF: OnceLock<Regex> = OnceLock::new();
    static REL: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r#"(?is)<link\b[^>]*>"#).unwrap());
    let href = HREF.get_or_init(|| Regex::new(r#"(?is)href\s*=\s*["']([^"']+)["']"#).unwrap());
    let rel = REL.get_or_init(|| Regex::new(r#"(?is)rel\s*=\s*["']?([^"'\s>]+)"#).unwrap());
    let head = extract_head(html);
    let mut out = Vec::new();
    for cap in tag.captures_iter(&head) {
        let t = &cap[0];
        let low = t.to_ascii_lowercase();
        let is_css = rel
            .captures(t)
            .map(|c| c[1].to_ascii_lowercase().contains("stylesheet"))
            .unwrap_or(false)
            || low.contains(".css");
        if !is_css {
            continue;
        }
        if let Some(h) = href.captures(t) {
            out.push(h[1].to_string());
        }
    }
    out
}

/// 文档内所有 `<style>` 块的内容。
fn inline_style_blocks(html: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?is)<style[^>]*>(.*?)</style>"#).unwrap());
    re.captures_iter(html).map(|c| c[1].to_string()).collect()
}

/// `<body class="...">` 的 class 值。
fn body_classes(html: &str) -> String {
    static BODY: OnceLock<Regex> = OnceLock::new();
    static CLS: OnceLock<Regex> = OnceLock::new();
    let body = BODY.get_or_init(|| Regex::new(r#"(?is)<body\b[^>]*>"#).unwrap());
    let cls = CLS.get_or_init(|| Regex::new(r#"(?is)class\s*=\s*["']([^"']*)["']"#).unwrap());
    if let Some(m) = body.find(html) {
        if let Some(c) = cls.captures(m.as_str()) {
            return c[1].trim().to_string();
        }
    }
    String::new()
}

/// 读取一个样式表并递归内联其 `@import`，重写其中 url() 为按需资源 URL。
fn load_css_recursive<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    css_path: &str,
    book_id: Option<i64>,
    visited: &mut std::collections::HashSet<String>,
    out: &mut String,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let key = css_path.replace('\\', "/").to_lowercase();
    if !visited.insert(key) {
        return;
    }
    let Some(bytes) = zip_read(zip, ci, css_path) else {
        return;
    };
    let css = String::from_utf8_lossy(&bytes).into_owned();
    static IMPORT: OnceLock<Regex> = OnceLock::new();
    let import_re = IMPORT
        .get_or_init(|| Regex::new(r#"(?is)@import\s+(?:url\(\s*)?["']?([^"')]+)["']?\s*\)?\s*;"#).unwrap());
    for cap in import_re.captures_iter(&css) {
        let imp = cap[1].trim();
        if imp.is_empty()
            || imp.starts_with("http")
            || imp.starts_with("//")
            || imp.starts_with("data:")
        {
            continue;
        }
        let target = resolve_in_doc(css_path, imp);
        load_css_recursive(zip, ci, &target, book_id, visited, out, depth + 1);
    }
    let no_import = import_re.replace_all(&css, "").into_owned();
    static CHARSET: OnceLock<Regex> = OnceLock::new();
    let no_charset = CHARSET
        .get_or_init(|| Regex::new(r#"(?is)@charset\s+["'][^"']*["']\s*;"#).unwrap())
        .replace_all(&no_import, "")
        .into_owned();
    out.push_str(&rewrite_css_urls(&no_charset, css_path, zip, ci, book_id));
    out.push('\n');
}

/// 把 CSS 中的 `url(...)` 指向 zip 内资源的引用改写为按需资源 URL（字体/图片均可）。
fn rewrite_css_urls<R: Read + std::io::Seek>(
    css: &str,
    css_path: &str,
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    book_id: Option<i64>,
) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?i)url\(\s*["']?([^"')]+)["']?\s*\)"#).unwrap());
    re.replace_all(css, |caps: &regex::Captures| {
        let raw = caps[1].trim();
        if raw.is_empty()
            || raw.starts_with("data:")
            || raw.starts_with("http")
            || raw.starts_with("//")
            || raw.starts_with('#')
            || raw.starts_with("res:")
            || raw.starts_with("res%3A")
            || raw.starts_with("file:")
        {
            return caps[0].to_string();
        }
        let target = resolve_in_doc(css_path, raw);
        if resolve_idx(ci, &target).is_none() {
            return caps[0].to_string();
        }
        match book_id {
            Some(bid) => format!("url({})", epub_image_url(bid, &target)),
            None => {
                if let Some(bytes) = zip_read(zip, ci, &target) {
                    let mime = mime_of(&target);
                    format!("url(data:{mime};base64,{})", crate::covers::b64(&bytes))
                } else {
                    caps[0].to_string()
                }
            }
        }
    })
    .into_owned()
}

/// 把原书 CSS 作用域隔离到 `#chapter-content` 内，避免污染阅读器 UI。
/// 做法：所有普通选择器统一加 `#chapter-content` 前缀（`html`/`body`/`:root` 映射到包裹层
/// `.epub-doc`）；`@font-face` / `@keyframes` 等需要全局生效的 at-rule 提到前缀外；
/// `@media` 等分组 at-rule 递归处理。
fn scope_book_css(css: &str) -> String {
    let (global, scoped) = split_css_scope(css);
    let mut out = String::new();
    if !global.trim().is_empty() {
        out.push_str(global.trim_end());
        out.push('\n');
    }
    if !scoped.trim().is_empty() {
        out.push_str(scoped.trim_end());
        out.push('\n');
    }
    out
}

fn split_css_scope(css: &str) -> (String, String) {
    let mut global = String::new();
    let mut scoped = String::new();
    let bytes = css.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                i = i + 2 + end + 2;
            } else {
                break;
            }
            continue;
        }
        if c == b'}' || c == b';' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        let mut paren = 0i32;
        let mut quote: Option<u8> = None;
        while j < n {
            let ch = bytes[j];
            if let Some(q) = quote {
                if ch == b'\\' {
                    j += 2;
                    continue;
                }
                if ch == q {
                    quote = None;
                }
                j += 1;
                continue;
            }
            match ch {
                b'"' | b'\'' => quote = Some(ch),
                b'(' => paren += 1,
                b')' => paren -= 1,
                b'{' if paren == 0 => break,
                b';' if paren == 0 => break,
                _ => {}
            }
            j += 1;
        }
        if j >= n {
            break;
        }
        if bytes[j] == b';' {
            i = j + 1;
            continue;
        }
        let prelude = css[start..j].trim();
        let block_start = j + 1;
        let mut k = block_start;
        let mut depth = 1i32;
        let mut quote2: Option<u8> = None;
        while k < n {
            let ch = bytes[k];
            if let Some(q) = quote2 {
                if ch == b'\\' {
                    k += 2;
                    continue;
                }
                if ch == q {
                    quote2 = None;
                }
                k += 1;
                continue;
            }
            match ch {
                b'"' | b'\'' => quote2 = Some(ch),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        let block = &css[block_start..k.min(n)];
        let lower = prelude.to_ascii_lowercase();
        if lower.starts_with("@font-face")
            || lower.starts_with("@keyframes")
            || lower.starts_with("@-webkit-keyframes")
            || lower.starts_with("@page")
            || lower.starts_with("@property")
            || lower.starts_with("@counter-style")
        {
            global.push_str(prelude);
            global.push('{');
            global.push_str(block);
            global.push_str("}\n");
        } else if lower.starts_with("@media")
            || lower.starts_with("@supports")
            || lower.starts_with("@layer")
            || lower.starts_with("@container")
            || lower.starts_with("@scope")
        {
            let (g2, s2) = split_css_scope(block);
            global.push_str(&g2);
            scoped.push_str(prelude);
            scoped.push('{');
            scoped.push_str(&s2);
            scoped.push_str("}\n");
        } else if lower.starts_with('@') {
            // 其余未知 at-rule 丢弃，避免污染
        } else {
            let sel = scope_selector_list(prelude);
            if !sel.is_empty() {
                scoped.push_str(&sel);
                scoped.push('{');
                scoped.push_str(block);
                scoped.push_str("}\n");
            }
        }
        i = k + 1;
    }
    (global, scoped)
}

fn scope_selector_list(sel: &str) -> String {
    let parts = split_top_commas(sel);
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        out.push(format!("#chapter-content {}", map_root_selectors(p)));
    }
    out.join(",")
}

fn map_root_selectors(sel: &str) -> String {
    // 把选择器里的 html/body/:root 类型选择器替换为包裹层 .epub-doc（保留 UTF-8 原文）。
    let b = sel.as_bytes();
    let n = b.len();
    let is_delim = |c: u8| {
        c.is_ascii_whitespace()
            || matches!(c, b'>' | b'+' | b'~' | b',' | b'.' | b'#' | b':' | b'[' | b']' | b'(' | b')' | b'*')
    };
    let mut out = String::with_capacity(n + 16);
    let mut i = 0usize;
    let mut last = 0usize;
    while i < n {
        let c = b[i];
        if c == b'"' || c == b'\'' {
            i += 1;
            while i < n {
                if b[i] == c {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let before_ok = i == 0 || is_delim(b[i - 1]);
        if before_ok {
            let kw_len = if i + 5 <= n && &b[i..i + 5] == b":root" {
                5
            } else if i + 4 <= n
                && (b[i..i + 4].eq_ignore_ascii_case(b"html")
                    || b[i..i + 4].eq_ignore_ascii_case(b"body"))
            {
                4
            } else {
                0
            };
            if kw_len > 0 {
                let end = i + kw_len;
                if end >= n || is_delim(b[end]) {
                    out.push_str(&sel[last..i]);
                    out.push_str(".epub-doc");
                    i = end;
                    last = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&sel[last..]);
    out
}

fn split_top_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for ch in s.chars() {
        if let Some(q) = quote {
            cur.push(ch);
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                cur.push(ch);
            }
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// 处理单个 `<img ...>` 标签：改写 src 为按需 URL（或内联 base64），
/// 读取图片头部尺寸并预置 width/height，补 loading/decoding。
fn process_img_tag<R: Read + std::io::Seek>(
    tag: &str,
    zip: &mut zip::ZipArchive<R>,
    ci: &CiMap,
    doc_path: &str,
    book_id: Option<i64>,
) -> String {
    let low = tag.to_ascii_lowercase();
    if !low.starts_with("<img") {
        return tag.to_string();
    }
    static IMG_ATTR_RE: OnceLock<Regex> = OnceLock::new();
    let attr_re = IMG_ATTR_RE.get_or_init(|| {
        Regex::new(r#"(?i)(src|xlink:href)\s*=\s*"([^"]+\.(?:png|jpe?g|gif|bmp|webp|svg))""#).unwrap()
    });
    let Some(caps) = attr_re.captures(tag) else {
        return tag.to_string();
    };
    let target = resolve_in_doc(doc_path, &caps[2]);
    // 读取图片头部尺寸（不内联整图）。失败则不加 width/height，由 CSS 兜底。
    let dims = zip_read(zip, ci, &target).and_then(|b| image_size(&b));
    let attr = &caps[1];
    let new_src = match book_id {
        Some(bid) => format!("{attr}=\"{}\"", epub_image_url(bid, &target)),
        None => {
            if let Some(bytes) = zip_read(zip, ci, &target) {
                let mime = mime_of(&target);
                format!("{attr}=\"data:{mime};base64,{}\"", crate::covers::b64(&bytes))
            } else {
                caps[0].to_string()
            }
        }
    };
    let mut out = tag.to_string();
    out.replace_range(caps.get(0).unwrap().start()..caps.get(0).unwrap().end(), &new_src);
    // 追加属性（在标签名之后）。不用 loading="lazy"：页模式是多列 + transform 容器，
    // Chromium 的懒加载可见性判断不可靠，会导致图片永不加载（页模式图片空白）。
    let mut extra = String::from(" decoding=\"async\"");
    if let Some((w, h)) = dims {
        if w > 0 && h > 0 {
            extra.push_str(&format!(" width=\"{w}\" height=\"{h}\""));
        }
    }
    let name_end = out
        .to_ascii_lowercase()
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(out.len());
    out.insert_str(name_end, &extra);
    out
}

fn mime_of(target: &str) -> &'static str {
    match target.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
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
    static BLOCK_END: OnceLock<Regex> = OnceLock::new();
    static BR: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    let block_end = BLOCK_END
        .get_or_init(|| Regex::new(r"(?i)</(?:p|div|h[1-6]|li|tr|blockquote|section)>").unwrap());
    let br = BR.get_or_init(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
    let tag = TAG.get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let body = extract_body(html);
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

/// 一次性提取整本书每章纯文本：只开一次 zip、只建一次 CiMap，
/// 避免按 spine 逐章调用 `doc_text` 时重复 O(条目数) 重建索引（大书会退化到分钟级）。
pub fn collect_texts(zip_bytes: &[u8], spine: &[String]) -> Vec<String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return vec![String::new(); spine.len()],
    };
    let ci = build_ci(&mut zip);
    let mut out = Vec::with_capacity(spine.len());
    for doc in spine {
        let text = match zip_read(&mut zip, &ci, doc) {
            Some(b) => {
                let html = String::from_utf8_lossy(&b);
                html_to_text(&html)
            }
            None => String::new(),
        };
        out.push(text);
    }
    out
}

/// 漫画判据扫描：按 spine 顺序逐篇读取纯文本，回调 `(spine 下标, 纯文本)`；
/// 回调返回 false 表示调用方已得出结论，立即停止（普通小说第一章就会停）。
/// `budget` 是累计**解压字节**预算：耗尽即停止，不再解压后续文档。
/// 返回实际消耗的字节数，供 `[open] is_manga` 诊断日志说明扫描了多少。
pub fn scan_spine_text<F: FnMut(usize, &str) -> bool>(
    zip_bytes: &[u8],
    spine: &[String],
    budget: usize,
    mut visit: F,
) -> usize {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return 0,
    };
    let ci = build_ci(&mut zip);
    let mut used = 0usize;
    for (i, doc) in spine.iter().enumerate() {
        if used >= budget {
            break;
        }
        let Some(bytes) = zip_read(&mut zip, &ci, doc) else {
            continue;
        };
        used = used.saturating_add(bytes.len());
        let html = String::from_utf8_lossy(&bytes);
        let text = html_to_text(&html);
        if !visit(i, &text) {
            break;
        }
    }
    used
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

/// 用正则把某个标签的文本内容替换为新值（保留开标签属性）。
fn set_tag_text(xml: &str, tag: &str, value: &str) -> Option<String> {
    let esc = escape_html(value);
    let pat = format!(r"(?s)(<{tag}[^>]*>).*?(</{tag}>)");
    let re = Regex::new(&pat).ok()?;
    if re.is_match(xml) {
        Some(
            re.replace(xml, |c: &regex::Captures| format!("{}{}{}", &c[1], esc, &c[2]))
                .into_owned(),
        )
    } else {
        None
    }
}

/// 直接覆盖写回 EPUB 的 OPF 元数据（dc:title / dc:creator），不生成备份。
pub fn write_meta(path: &str, title: &str, author: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let cursor = std::io::Cursor::new(&data);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("EPUB 打开失败: {e}"))?;
    let ci = build_ci(&mut zip);

    let container = zip_read(&mut zip, &ci, "META-INF/container.xml")
        .ok_or("缺少 META-INF/container.xml")?;
    let container_str = decode_text(&container);
    let opf_path = opf_path_from_container(&container_str)
        .or_else(|| find_opf_in_zip(&ci))
        .ok_or_else(|| {
            if is_encrypted(&ci) {
                ERR_ENCRYPTED.to_string()
            } else {
                "container.xml 中未找到 OPF".to_string()
            }
        })?;

    let opf_bytes = zip_read(&mut zip, &ci, &opf_path).ok_or("OPF 文件缺失")?;
    let opf = decode_text(&opf_bytes);

    let mut new_opf = set_tag_text(&opf, "dc:title", title);
    new_opf = new_opf.and_then(|x| set_tag_text(&x, "dc:creator", author).or_else(|| {
        // 没有 dc:creator 时，在 dc:title 之后插入一个
        let esc = escape_html(author);
        let re = Regex::new(r"(?s)(</dc:title>)").ok()?;
        Some(
            re.replace(&x, |c: &regex::Captures| {
                format!("{c1}<dc:creator>{esc}</dc:creator>", c1 = &c[1])
            })
            .into_owned(),
        )
    }));
    let new_opf = new_opf.ok_or("OPF 中未找到 dc:title 标签，无法写回")?;

    // 读入所有条目
    let mut entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        entries.push((name, buf));
    }
    drop(zip);

    let opf_key = opf_path.replace('\\', "/").to_lowercase();
    let tmp = format!("{path}.tidepage-tmp");
    {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zw = zip::ZipWriter::new(file);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // 按 EPUB 规范：mimetype 首个且不压缩
        if entries.iter().any(|(n, _)| n.eq_ignore_ascii_case("mimetype")) {
            zw.start_file("mimetype", stored).map_err(|e| e.to_string())?;
            zw.write_all(b"application/epub+zip").map_err(|e| e.to_string())?;
        }
        for (name, buf) in &entries {
            if name.eq_ignore_ascii_case("mimetype") || name.ends_with('/') {
                continue;
            }
            zw.start_file(name.clone(), deflated).map_err(|e| e.to_string())?;
            if name.replace('\\', "/").to_lowercase() == opf_key {
                zw.write_all(new_opf.as_bytes()).map_err(|e| e.to_string())?;
            } else {
                zw.write_all(buf).map_err(|e| e.to_string())?;
            }
        }
        zw.finish().map_err(|e| e.to_string())?;
    }
    std::fs::copy(&tmp, path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 计量：对给定文件，(1) 从尾部 EOCD 读出中央目录大小；(2) 用共享计数 reader 执行一次
    /// `ZipArchive::new`，报告读取字节总数（用看门狗防止畸形 CD 卡死）。
    /// 用法：TP_ZIP_BENCH=<epub> cargo test zipnew_bytes_bench -- --nocapture
    #[test]
    fn zipnew_bytes_bench() {
        use std::io::{Read, Seek, SeekFrom};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let Ok(spec) = std::env::var("TP_ZIP_BENCH") else { return };
        // 支持 "@文件" 形式（文件首行是 UTF-8 路径），避免命令行传非 ASCII 路径被转码。
        let path = if let Some(f) = spec.strip_prefix('@') {
            std::fs::read_to_string(f).unwrap().lines().next().unwrap().trim().to_string()
        } else {
            spec
        };
        let file_len = std::fs::metadata(&path).unwrap().len();
        // EOCD：读尾部 ≤1KiB，取中央目录大小(+12)。
        let (eocd_found, cd_size) = {
            let mut f = std::fs::File::open(&path).unwrap();
            let n = file_len.min(1024);
            f.seek(SeekFrom::End(-(n as i64))).unwrap();
            let mut b = vec![0u8; n as usize];
            f.read_exact(&mut b).unwrap();
            match b.windows(4).rposition(|w| w == [0x50, 0x4b, 0x05, 0x06]) {
                Some(p) if p + 16 <= b.len() => (
                    true,
                    u32::from_le_bytes([b[p + 12], b[p + 13], b[p + 14], b[p + 15]]) as u64,
                ),
                _ => (false, 0),
            }
        };
        // ZipArchive::new 读取字节（共享计数，看门狗 30s）。
        #[derive(Default)]
        struct Stats { calls: u64, bytes: u64, min_off: u64, max_end: u64, seq: bool, first: Vec<(u64,u64)>, last: Vec<(u64,u64)>, below_cd: u64, at_cd: u64 }
        let cd_off = {
            // 再从 EOCD 取 CD 起始偏移(+16)，用于分区域统计。
            let mut f = std::fs::File::open(&path).unwrap();
            let n = file_len.min(1024);
            f.seek(SeekFrom::End(-(n as i64))).unwrap();
            let mut b = vec![0u8; n as usize];
            f.read_exact(&mut b).unwrap();
            match b.windows(4).rposition(|w| w == [0x50,0x4b,0x05,0x06]) {
                Some(p) if p + 20 <= b.len() => u32::from_le_bytes([b[p+16],b[p+17],b[p+18],b[p+19]]) as u64,
                _ => 0,
            }
        };
        let stats = Arc::new(std::sync::Mutex::new(Stats { min_off: u64::MAX, ..Default::default() }));
        struct Counting<R> { inner: R, pos: u64, cd_off: u64, stats: Arc<std::sync::Mutex<Stats>> }
        impl<R: std::io::Read> std::io::Read for Counting<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                let mut s = self.stats.lock().unwrap();
                let start = self.pos; let end = self.pos + n as u64;
                s.calls += 1; s.bytes += n as u64;
                if start < s.min_off { s.min_off = start; }
                if end > s.max_end { s.max_end = end; }
                if start < self.cd_off { s.below_cd += n as u64; } else { s.at_cd += n as u64; }
                if s.first.len() < 6 { s.first.push((start, n as u64)); }
                s.last.push((start, n as u64)); if s.last.len() > 6 { s.last.remove(0); }
                self.pos = end;
                Ok(n)
            }
        }
        impl<R: std::io::Seek> std::io::Seek for Counting<R> {
            fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> { let r = self.inner.seek(p)?; self.pos = r; Ok(r) }
        }
        let counter = Arc::new(AtomicU64::new(0));
        let c2 = counter.clone();
        let s2 = stats.clone();
        let p2 = path.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done2 = done.clone();
        std::thread::spawn(move || {
            let f = std::fs::File::open(&p2).unwrap();
            let c = Counting { inner: f, pos: 0, cd_off, stats: s2 };
            let _ = zip::ZipArchive::new(c);
            done2.store(true, Ordering::SeqCst);
        });
        let t0 = std::time::Instant::now();
        while !done.load(Ordering::SeqCst) && t0.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let finished = done.load(Ordering::SeqCst);
        let s = stats.lock().unwrap();
        eprintln!(
            "[zipnew_bench] file_size={} eocd_found={} cd_size={} cd_offset={} zipnew_finished={} bytes_read={} calls={} min_off={} max_end={} below_cd={} at_cd={} first={:?} last={:?} elapsed_ms={}",
            file_len, eocd_found, cd_size, cd_off, finished, counter.load(Ordering::SeqCst),
            s.calls, s.min_off, s.max_end, s.below_cd, s.at_cd, s.first, s.last, t0.elapsed().as_millis()
        );
    }

    /// 计量：对给定文件执行**带预算**的封面提取，输出解析阶段消耗的 read 调用次数/字节数、
    /// 是否命中预算（病态）、耗时与结果。用于为 `MAX_PARSE_CALLS` 取值提供依据，并对失败文件归类。
    /// 用法：TP_EPUB_COVER=<epub|@pathfile> cargo test cover_budget_bench -- --nocapture
    #[test]
    fn cover_budget_bench() {
        let Ok(spec) = std::env::var("TP_EPUB_COVER") else { return };
        let path = if let Some(f) = spec.strip_prefix('@') {
            std::fs::read_to_string(f).unwrap().lines().next().unwrap().trim().to_string()
        } else {
            spec
        };
        let t = std::time::Instant::now();
        let bytes = std::env::var("TP_EPUB_COVER_BYTES").ok().and_then(|s| s.parse().ok()).unwrap_or(MAX_PARSE_BYTES);
        let calls = std::env::var("TP_EPUB_COVER_CALLS").ok().and_then(|s| s.parse().ok()).unwrap_or(MAX_PARSE_CALLS);
        let budget = Arc::new(CoverReadBudget::new(bytes, calls));
        let r = extract_cover_file(&path, &budget);
        let desc = match &r {
            Ok(Some(b)) => format!("Ok(cover {} bytes)", b.len()),
            Ok(None) => "Ok(no cover)".to_string(),
            Err(e) => format!("Err({e})"),
        };
        let kind = if r.is_ok() {
            "ok"
        } else if budget.exhausted() {
            "cover-pathological"
        } else {
            "cover-error"
        };
        eprintln!(
            "[cover_budget] kind={kind} exhausted={} calls_used={} bytes_used={} result={desc} elapsed_ms={}",
            budget.exhausted(), budget.calls_used(), budget.bytes_used(), t.elapsed().as_millis()
        );
    }

    /// 计量：对病态文件执行带预算的封面提取，并在 500ms 后 cancel（模拟看门狗取消），
    /// 验证 worker 线程随即结束（不再泄漏）。用法：TP_EPUB_CANCEL=<epub> cargo test cover_cancel_bench -- --nocapture
    #[test]
    fn cover_cancel_bench() {
        let Ok(spec) = std::env::var("TP_EPUB_CANCEL") else { return };
        let path = if let Some(f) = spec.strip_prefix('@') {
            std::fs::read_to_string(f).unwrap().lines().next().unwrap().trim().to_string()
        } else {
            spec
        };
        let budget = Arc::new(CoverReadBudget::default());
        let b = budget.clone();
        let p = path.clone();
        let t = std::time::Instant::now();
        let h = std::thread::spawn(move || extract_cover_file(&p, &b).is_err());
        std::thread::sleep(std::time::Duration::from_millis(500));
        budget.cancel(); // 取消
        let is_err = h.join().unwrap();
        eprintln!(
            "[cover_cancel] is_err={} elapsed_ms={}",
            is_err,
            t.elapsed().as_millis()
        );
    }

    /// 计量：用共享计数的 reader 统计 extract_cover 各阶段实际从磁盘拉取的字节数。
    /// 用法：TP_COVER_BENCH=<epub 路径> cargo test cover_bytes_bench -- --nocapture
    #[test]
    fn cover_bytes_bench() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let Ok(path) = std::env::var("TP_COVER_BENCH") else { return };
        struct Counting<R> { inner: R, read: Arc<AtomicU64> }
        impl<R: std::io::Read> std::io::Read for Counting<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                self.read.fetch_add(n as u64, Ordering::SeqCst);
                Ok(n)
            }
        }
        impl<R: std::io::Seek> std::io::Seek for Counting<R> {
            fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> { self.inner.seek(p) }
        }
        let file_len = std::fs::metadata(&path).unwrap().len();
        let counter = Arc::new(AtomicU64::new(0));
        let c = Counting { inner: std::fs::File::open(&path).unwrap(), read: counter.clone() };
        let snap = || counter.load(Ordering::SeqCst);

        let mut zip = zip::ZipArchive::new(c).unwrap();
        let after_open = snap(); // EOCD + 中央目录

        let read_named = |zip: &mut zip::ZipArchive<Counting<std::fs::File>>, name: &str| -> Option<Vec<u8>> {
            let mut e = zip.by_name(name).ok()?;
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut b).ok()?;
            Some(b)
        };
        let container = read_named(&mut zip, "META-INF/container.xml").map(|b| decode_text(&b)).unwrap_or_default();
        let after_container = snap();
        let opf_path = opf_path_from_container(&container).unwrap_or_default();
        let opf = if opf_path.is_empty() { String::new() }
                  else { read_named(&mut zip, &opf_path).map(|b| decode_text(&b)).unwrap_or_default() };
        let after_opf = snap();
        let href = cover_href_from_opf(&opf).unwrap_or_default();
        let full = if href.is_empty() { String::new() } else { norm_path(&opf_path, &href) };
        let cover = if full.is_empty() { None } else { read_named(&mut zip, &full) };
        let after_cover = snap();
        let cover_len = cover.as_ref().map(|b| b.len()).unwrap_or(0);

        eprintln!(
            "[cover_bench] file_size={} total_read={} ratio={:.4} | open_CD={} container={} opf={} cover_entry={} (cover_decoded={}) path={}",
            file_len, after_cover, after_cover as f64 / file_len as f64,
            after_open, after_container - after_open, after_opf - after_container,
            after_cover - after_opf, cover_len, path
        );
    }

    /// 设置 TIDEPAGE_PROFILE_EPUB 时，校验按需图片 URL 能被 read_entry 正确解析。
    #[test]
    fn epubimg_urls_resolve() {
        let path = match std::env::var("TIDEPAGE_PROFILE_EPUB") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return,
        };
        let data = std::fs::read(&path).unwrap();
        let (doc, ci) = parse_with_ci(&data).unwrap();
        let mut total = 0usize;
        let mut ok = 0usize;
        for idx in [0usize, 1, 100, 1000, 3000] {
            if idx >= doc.toc.len() {
                continue;
            }
            let start = doc.toc[idx].index;
            let end = doc.toc_end.get(idx).copied().unwrap_or(start + 1).max(start + 1).min(doc.spine.len());
            let docs: Vec<String> = (start..end).filter_map(|si| doc.spine.get(si).cloned()).collect();
            let html = chapter_html_multi_ci(&data, Some(&ci), &docs, Some(42), false).unwrap();
            let re = Regex::new(r"epubimg/42/([A-Za-z0-9_\-]+)").unwrap();
            for caps in re.captures_iter(&html) {
                total += 1;
                let target = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(caps[1].as_bytes())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default();
                if read_entry(&data, &ci, &target).map(|b| !b.is_empty()).unwrap_or(false) {
                    ok += 1;
                }
            }
        }
        eprintln!("[epubimg] urls={total} resolved={ok}");
        assert!(total > 0, "no epubimg urls generated");
        assert_eq!(total, ok, "some epubimg urls did not resolve");
    }

    /// 任务三/五共用的内存 EPUB 夹具：2 章、NCX 目录、正文可统计字数。
    fn tiny_epub() -> Vec<u8> {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("mimetype", stored).unwrap();
            zw.write_all(b"application/epub+zip").unwrap();
            zw.start_file("META-INF/container.xml", deflated).unwrap();
            zw.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            zw.start_file("OEBPS/content.opf", deflated).unwrap();
            zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#).unwrap();
            zw.start_file("OEBPS/toc.ncx", deflated).unwrap();
            zw.write_all(
                r#"<?xml version="1.0" encoding="UTF-8"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="n1" playOrder="1"><navLabel><text>第一章</text></navLabel><content src="c1.xhtml"/></navPoint><navPoint id="n2" playOrder="2"><navLabel><text>第二章</text></navLabel><content src="c2.xhtml"/></navPoint></navMap></ncx>"#
                    .as_bytes(),
            )
            .unwrap();
            for (name, body) in [
                ("OEBPS/c1.xhtml", "第一章正文，这里是一段足够长的文字用来统计字数。"),
                ("OEBPS/c2.xhtml", "第二章正文，同样是一段用来统计字数的文字。"),
            ] {
                zw.start_file(name, deflated).unwrap();
                zw.write_all(format!("<html><body><p>{body}</p></body></html>").as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 任务三/五的共用路径（book_char_counts 与 get_book_word_count 都走 epub_word_stats）：
    /// 快速解析 parse 不给任何字数，完整解析必须给出每章真实字数与全书 total/pure，
    /// 且与 parse_with_chars 逐项一致；total/pure 取自 spine 逐文档求和（不是 toc 求和）。
    #[test]
    fn full_parse_fills_chapter_and_total_chars() {
        let data = tiny_epub();
        let fast = parse(&data).unwrap();
        assert!(fast.toc.len() >= 2, "目录应有两条: {:?}", fast.toc.len());
        assert!(
            fast.toc.iter().all(|t| t.chars == 0),
            "快速解析不得统计每章字数: {:?}",
            fast.toc.iter().map(|t| t.chars).collect::<Vec<_>>()
        );
        assert_eq!(fast.total_chars, 0, "快速解析不得统计全书字数");
        assert_eq!(fast.total_text_chars, 0);

        let (per_chapter, total, pure) = epub_word_stats(&data).unwrap();
        assert_eq!(per_chapter.len(), fast.toc.len(), "每章字数条数应等于目录条数");
        assert!(
            per_chapter.iter().all(|c| *c > 0),
            "每章字数应大于 0: {per_chapter:?}"
        );
        assert!(total > 0, "total 应大于 0: {total}");
        assert!(pure > 0, "pure 应大于 0: {pure}");
        assert!(pure <= total, "纯字数不应超过总字数: {pure}/{total}");

        let full = parse_with_chars(&data).unwrap();
        assert_eq!(total, full.total_chars, "total 必须等于 parse_with_chars 的 total_chars");
        assert_eq!(pure, full.total_text_chars, "pure 必须等于 total_text_chars");
        assert_eq!(
            per_chapter,
            full.toc.iter().map(|t| t.chars).collect::<Vec<_>>(),
            "每章字数应与完整解析的目录字数逐项一致"
        );
    }
}
