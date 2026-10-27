use crate::models::TocEntry;
use crate::txt;
use regex::Regex;
use std::io::Write;
use std::path::Path;

pub struct ConvertedBook {
    pub title: String,
    pub author: String,
    pub toc: Vec<TocEntry>,
    pub chapters: Vec<String>,
    pub plain: String,
}

/// 块级标签：结束位置换行（p/div/br/li/h1-h6/tr 等），保证段落不粘连。
const HTML_BLOCK_TAGS: &[&str] = &[
    "address", "article", "aside", "blockquote", "br", "dd", "div", "dl", "dt", "figcaption",
    "figure", "footer", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "li", "main",
    "nav", "ol", "p", "pre", "section", "table", "tbody", "td", "tfoot", "th", "thead", "tr",
    "ul",
];

/// 解码单个 HTML 实体；未知实体保留原文（不吞字，宁可多显示一个 &xxx;）。
fn decode_html_entity(name: &str) -> String {
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        return u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("&{name};"));
    }
    if let Some(dec) = name.strip_prefix('#') {
        return dec
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("&{name};"));
    }
    match name {
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ",
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "ldquo" => "“",
        "rdquo" => "”",
        "lsquo" => "‘",
        "rsquo" => "’",
        "middot" => "·",
        "bull" => "•",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "times" => "×",
        "divide" => "÷",
        "deg" => "°",
        "plusmn" => "±",
        "laquo" => "«",
        "raquo" => "»",
        "sup2" => "²",
        "sup3" => "³",
        "frac12" => "½",
        _ => return format!("&{name};"),
    }
    .to_string()
}

/// J24：HTML → 纯文本。块级标签换行、行内标签不换行（避免「粗<b>体</b>」被断成两行），
/// 并解码命名/数字实体（旧实现遇到数字实体直接删空，正文丢字）。
pub fn html_to_text(html: &str) -> String {
    let tag_re = Regex::new(r"(?is)<[^>]*>").unwrap();
    let name_re = Regex::new(r"(?is)^<\s*/?\s*([a-zA-Z][a-zA-Z0-9]*)").unwrap();
    let mut text = String::with_capacity(html.len());
    let mut last = 0usize;
    for m in tag_re.find_iter(html) {
        text.push_str(&html[last..m.start()]);
        if let Some(c) = name_re.captures(m.as_str()) {
            let name = c[1].to_ascii_lowercase();
            if HTML_BLOCK_TAGS.contains(&name.as_str()) {
                text.push('\n');
            }
        }
        last = m.end();
    }
    text.push_str(&html[last..]);
    let ent_re = Regex::new(r"&(#x?[0-9A-Fa-f]+|[A-Za-z][A-Za-z0-9]+);").unwrap();
    let text = ent_re
        .replace_all(&text, |c: &regex::Captures| decode_html_entity(&c[1]))
        .into_owned();
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 MOBI / AZW3 读取文本，产出可写入 TXT / EPUB 的书
pub fn mobi_to_ebook_data(input_path: &str) -> Result<ConvertedBook, String> {
    let doc = crate::mobi::parse(input_path)?;
    let mut toc = Vec::new();
    let mut chapters = Vec::new();
    let mut plain = String::new();
    for (i, (s, e)) in doc.parts.iter().enumerate() {
        let slice = doc.html.get(*s..*e).unwrap_or("").to_string();
        let text = html_to_text(&slice);
        if text.is_empty() {
            continue;
        }
        let title = doc
            .toc
            .get(i)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| format!("第 {} 节", i + 1));
        chapters.push(text.clone());
        toc.push(TocEntry {
            title: title.clone(),
            index: toc.len(),
            chars: text.chars().count() as i64,
        });
        if !plain.is_empty() {
            plain.push('\n');
        }
        plain.push_str(&title);
        plain.push('\n');
        plain.push_str(text.trim());
    }
    if toc.is_empty() {
        let t = html_to_text(&doc.html);
        if !t.is_empty() {
            chapters.push(t.clone());
            toc.push(TocEntry {
                title: "全文".into(),
                index: 0,
                chars: t.chars().count() as i64,
            });
            plain = t;
        }
    }
    Ok(ConvertedBook {
        title: doc.title,
        author: doc.author,
        toc,
        chapters,
        plain,
    })
}

/// P0-2：forced 与阅读路径一致——用户在设置里显式指定的编码优先，None 时沿用自动探测。
pub fn txt_to_epub_data(
    input_path: &str,
    pattern: &str,
    forced: Option<&str>,
) -> Result<ConvertedBook, String> {
    let pattern = if pattern.trim().is_empty() { crate::default_pattern().to_string() } else { pattern.to_string() };
    let doc = txt::decode_and_split(input_path, Some(&pattern), forced)?;
    let plain = doc.text;
    let ranges = doc.ranges;
    let toc = doc.toc;
    let title = Path::new(input_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut chapters = Vec::new();
    for (start, end) in &ranges {
        chapters.push(plain[*start..*end].to_string());
    }
    Ok(ConvertedBook {
        title,
        author: String::new(),
        toc,
        chapters,
        plain,
    })
}

pub fn epub_to_txt_data(input_path: &str) -> Result<ConvertedBook, String> {
    let data = std::fs::read(input_path).map_err(|e| e.to_string())?;
    let doc = crate::epub::parse(&data)?;
    let mut out = String::new();
    let mut toc = Vec::new();
    // P1-2：一次打开 zip 取全部章节文本；旧实现每章 doc_text 都重建 ZipArchive+CiMap。
    let texts = crate::epub::collect_texts(&data, &doc.spine);
    for (i, doc_path) in doc.spine.iter().enumerate() {
        let text = texts.get(i).cloned().unwrap_or_default();
        let _ = doc_path;
        let title = doc
            .toc
            .iter()
            .find(|t| t.index == i)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        if !title.is_empty() {
            toc.push(TocEntry {
                title: title.clone(),
                index: toc.len(),
                chars: text.chars().count() as i64,
            });
            out.push_str(&title);
            out.push('\n');
        }
        let before = out.len();
        out.push_str(text.trim());
        out.push_str("\n\n");
        if title.is_empty() {
            toc.push(TocEntry {
                title: format!("第 {} 节", i + 1),
                index: toc.len(),
                chars: 0,
            });
        }
        let _ = before;
    }
    let title = if doc.title.is_empty() {
        Path::new(input_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        doc.title.clone()
    };
    Ok(ConvertedBook {
        title,
        author: doc.author,
        toc,
        chapters: vec![],
        plain: out,
    })
}

pub struct EpubOptions<'a> {
    pub cover: Option<(&'a [u8], &'a str)>,
    pub author: Option<&'a str>,
    pub css: Option<&'a str>,
}

/// 原子写：同目录临时文件 → 闭包写入 → sync_all → rename 覆盖目标。
/// 失败删除临时文件并返回 Err，目标文件字节保持不变（不会留下截断的半成品）。
/// 调用方都已处在 spawn_blocking 里，这里只用同步阻塞 IO。
fn atomic_write<F>(path: &str, write: F) -> Result<(), String>
where
    F: FnOnce(&mut std::fs::File) -> Result<(), String>,
{
    let target = Path::new(path);
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .ok_or_else(|| format!("目标路径无效：{path}"))?
        .to_string_lossy();
    // 临时文件带进程号：同目录（同卷）才能保证 rename 原子替换。
    let tmp = dir.join(format!("{name}.tidepage-tmp-{}", std::process::id()));
    // 上次异常退出可能残留同名临时文件；只清我们自己命名的文件。
    let _ = std::fs::remove_file(&tmp);
    let res = (|| -> Result<(), String> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| format!("创建临时文件失败：{e}"))?;
        write(&mut f)?;
        f.sync_all().map_err(|e| format!("写入磁盘失败：{e}"))?;
        Ok(())
    })();
    match res {
        Ok(()) => match std::fs::rename(&tmp, target) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(format!("替换目标文件失败：{e}"))
            }
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn write_epub(book: &ConvertedBook, opts_in: &EpubOptions, out_path: &str) -> Result<(), String> {
    atomic_write(out_path, |file| write_epub_into(book, opts_in, file))
}

/// 把整本 EPUB 写进 atomic_write 提供的临时文件句柄。
fn write_epub_into(
    book: &ConvertedBook,
    opts_in: &EpubOptions,
    file: &mut std::fs::File,
) -> Result<(), String> {
    let author = opts_in.author.map(|a| a.trim()).filter(|a| !a.is_empty()).unwrap_or(&book.author);
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("mimetype", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored))
        .map_err(|e| e.to_string())?;
    zip.write_all(b"application/epub+zip").map_err(|e| e.to_string())?;

    zip.start_file("META-INF/container.xml", opts).map_err(|e| e.to_string())?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
    )
    .map_err(|e| e.to_string())?;

    let uuid = format!("{:032x}", crc_of(book.title.as_bytes()));
    let mut manifest = String::new();
    manifest.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    manifest.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    manifest.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    let mut cover_item = String::new();
    let mut cover_file = String::new();
    let mut cover_mime = String::new();
    if let Some((_, ext)) = opts_in.cover {
        let ext = ext.to_lowercase();
        let (name, mime) = match ext.as_str() {
            "jpg" | "jpeg" => ("cover.jpg", "image/jpeg"),
            "gif" => ("cover.gif", "image/gif"),
            "webp" => ("cover.webp", "image/webp"),
            _ => ("cover.png", "image/png"),
        };
        manifest.push_str(&format!(
            "<item id=\"cover-image\" href=\"{name}\" media-type=\"{mime}\" properties=\"cover-image\"/>\n"
        ));
        cover_item = "cover-image".to_string();
        cover_file = name.to_string();
        cover_mime = mime.to_string();
        manifest.push_str("<item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\" properties=\"cover\"/>\n");
    }
    for (i, _) in book.chapters.iter().enumerate() {
        manifest.push_str(&format!(
            "<item id=\"c{}\" href=\"c{}.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            i, i
        ));
    }

    let mut spine = String::new();
    if !cover_item.is_empty() {
        spine.push_str("<itemref idref=\"cover\"/>\n");
    }
    for (i, _) in book.chapters.iter().enumerate() {
        spine.push_str(&format!("<itemref idref=\"c{}\"/>\n", i));
    }

    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid" xml:lang="zh">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">urn:uuid:{uuid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:language>zh</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
    {cover_meta}
  </metadata>
  <manifest>
{manifest}
  </manifest>
  <spine toc="ncx">
{spine}
  </spine>
</package>"#,
        uuid = uuid,
        title = crate::txt::escape_html(&book.title),
        author = crate::txt::escape_html(author),
        modified = crate::db::day_string(),
        cover_meta = if cover_item.is_empty() {
            String::new()
        } else {
            format!("<meta name=\"cover\" content=\"{}\"/>", cover_item)
        },
        manifest = manifest,
        spine = spine,
    );

    zip.start_file("OEBPS/content.opf", opts).map_err(|e| e.to_string())?;
    zip.write_all(opf.as_bytes()).map_err(|e| e.to_string())?;

    let css = format!(
        "body{{margin:1em;line-height:1.7}}h1{{font-size:1.4em;margin:0.5em 0 1em}}p{{text-indent:2em;margin:0.4em 0}}\n{}",
        opts_in.css.unwrap_or("")
    );
    zip.start_file("OEBPS/style.css", opts).map_err(|e| e.to_string())?;
    zip.write_all(css.as_bytes()).map_err(|e| e.to_string())?;

    let mut navpoints = String::new();
    let mut play = 0;
    for (i, ch) in book.toc.iter().enumerate() {
        if i >= book.chapters.len() {
            break;
        }
        play += 1;
        navpoints.push_str(&format!(
            r#"<navPoint id="np{}" playOrder="{}"><navLabel><text>{}</text></navLabel><content src="c{}.xhtml"/></navPoint>"#,
            i,
            play,
            crate::txt::escape_html(&ch.title),
            i
        ));
    }
    let ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:{uuid}"/><meta name="dtb:depth" content="1"/></head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>{navpoints}</navMap>
</ncx>"#,
        uuid = uuid,
        title = crate::txt::escape_html(&book.title),
        navpoints = navpoints,
    );
    zip.start_file("OEBPS/toc.ncx", opts).map_err(|e| e.to_string())?;
    zip.write_all(ncx.as_bytes()).map_err(|e| e.to_string())?;

    let nav_items: String = book
        .toc
        .iter()
        .enumerate()
        .take(book.chapters.len())
        .map(|(i, ch)| {
            format!(
                "<li><a href=\"c{}.xhtml\">{}</a></li>",
                i,
                crate::txt::escape_html(&ch.title)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let nav_xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>{t}</title></head>
<body><nav epub:type="toc" id="toc"><h1>{t}</h1><ol>
{items}
</ol></nav></body></html>"#,
        t = crate::txt::escape_html(&book.title),
        items = nav_items,
    );
    zip.start_file("OEBPS/nav.xhtml", opts).map_err(|e| e.to_string())?;
    zip.write_all(nav_xhtml.as_bytes()).map_err(|e| e.to_string())?;

    if let Some((bytes, _)) = opts_in.cover {
        zip.start_file(format!("OEBPS/{cover_file}"), opts).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
        let _ = &cover_mime;
        let cover_page = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>{t}</title><style>html,body{{margin:0;padding:0;height:100%}}img{{width:100%;height:100%;object-fit:cover;display:block}}</style></head>
<body><section epub:type="cover"><img src="{f}" alt="{t}" /></section></body></html>"#,
            t = crate::txt::escape_html(&book.title),
            f = cover_file,
        );
        zip.start_file("OEBPS/cover.xhtml", opts).map_err(|e| e.to_string())?;
        zip.write_all(cover_page.as_bytes()).map_err(|e| e.to_string())?;
    }

    for (i, body) in book.chapters.iter().enumerate() {
        let title = book.toc.get(i).map(|t| t.title.as_str()).unwrap_or("");
        let body = {
            let mut lines = body.splitn(2, '\n');
            let first = lines.next().unwrap_or("").trim();
            let rest = lines.next().unwrap_or("");
            if !first.is_empty() && !title.is_empty() && (first.starts_with(title) || title.starts_with(first)) {
                rest
            } else {
                body.as_str()
            }
        };
        let paras: String = body
            .split('\n')
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| format!("<p>{}</p>", crate::txt::escape_html(l)))
            .collect::<Vec<_>>()
            .join("\n");
        let xhtml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>{t}</title><link rel="stylesheet" type="text/css" href="style.css"/></head>
<body><h2>{t}</h2>
{paras}
</body></html>"#,
            t = crate::txt::escape_html(title),
            paras = paras,
        );
        zip.start_file(format!("OEBPS/c{}.xhtml", i), opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(xhtml.as_bytes()).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn crc_of(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn write_txt(book: &ConvertedBook, out_path: &str) -> Result<(), String> {
    atomic_write(out_path, |f| {
        f.write_all(book.plain.as_bytes())
            .map_err(|e| e.to_string())
    })
}

pub struct AiExportItem {
    pub title: String,
    pub html: String,
}

fn build_ai_book(title: &str, author: &str, items: &[AiExportItem]) -> ConvertedBook {
    let mut toc = Vec::new();
    let mut chapters = Vec::new();
    let mut plain = String::new();
    if !author.is_empty() {
        plain.push_str(author);
        plain.push_str("\n\n");
    }
    for (i, it) in items.iter().enumerate() {
        let t = if it.title.trim().is_empty() {
            format!("片段 {}", i + 1)
        } else {
            it.title.trim().to_string()
        };
        let text = html_to_text(&it.html);
        if text.trim().is_empty() {
            continue;
        }
        chapters.push(text.clone());
        toc.push(TocEntry {
            title: t.clone(),
            index: toc.len(),
            chars: text.chars().count() as i64,
        });
        if !plain.is_empty() {
            plain.push_str("\n\n");
        }
        plain.push_str(&t);
        plain.push('\n');
        plain.push_str(text.trim());
    }
    ConvertedBook {
        title: title.to_string(),
        author: author.to_string(),
        toc,
        chapters,
        plain,
    }
}

pub fn ai_to_txt(title: &str, author: &str, items: &[AiExportItem], out_path: &str) -> Result<(), String> {
    let book = build_ai_book(title, author, items);
    write_txt(&book, out_path)
}

/// J25：AI 导出 EPUB 前做属性级净化。
/// 旧实现只删 script/style 等标签名，img 的 onerror 与 javascript: 链接会原样进书；
/// 复用正文净化（sanitize_fragment）后危险标签连内容一起丢弃、非法属性/URL 被剥离，
/// 合法 p/a/img 保留。
fn sanitize_xhtml(html: &str) -> String {
    crate::sanitize::sanitize_fragment(html)
}

pub fn ai_to_epub(title: &str, author: &str, items: &[AiExportItem], out_path: &str) -> Result<(), String> {
    let book = build_ai_book(title, author, items);
    atomic_write(out_path, |file| write_ai_epub_into(&book, author, file))
}

/// 把 AI 导出的 EPUB 写进 atomic_write 提供的临时文件句柄。
fn write_ai_epub_into(
    book: &ConvertedBook,
    author: &str,
    file: &mut std::fs::File,
) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("mimetype", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored))
        .map_err(|e| e.to_string())?;
    zip.write_all(b"application/epub+zip").map_err(|e| e.to_string())?;

    zip.start_file("META-INF/container.xml", opts).map_err(|e| e.to_string())?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
    )
    .map_err(|e| e.to_string())?;

    let uuid = format!("{:032x}", crc_of(format!("ai:{}", book.title).as_bytes()));
    let mut manifest = String::new();
    manifest.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    manifest.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    manifest.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for (i, _) in book.chapters.iter().enumerate() {
        manifest.push_str(&format!(
            "<item id=\"c{}\" href=\"c{}.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            i, i
        ));
    }
    let mut spine = String::new();
    for (i, _) in book.chapters.iter().enumerate() {
        spine.push_str(&format!("<itemref idref=\"c{}\"/>\n", i));
    }

    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid" xml:lang="zh">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">urn:uuid:{uuid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:language>zh</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
{manifest}
  </manifest>
  <spine toc="ncx">
{spine}
  </spine>
</package>"#,
        uuid = uuid,
        title = crate::txt::escape_html(&book.title),
        author = crate::txt::escape_html(if author.is_empty() { "TidePage AI" } else { author }),
        modified = crate::db::day_string(),
        manifest = manifest,
        spine = spine,
    );

    zip.start_file("OEBPS/content.opf", opts).map_err(|e| e.to_string())?;
    zip.write_all(opf.as_bytes()).map_err(|e| e.to_string())?;

    let css = "body{margin:1em;line-height:1.8}h1,h2{font-size:1.3em;margin:0.8em 0 1em}p{margin:0.5em 0}pre{background:rgba(128,128,128,0.12);padding:8px;border-radius:6px;overflow-x:auto;font-size:0.9em}code{font-family:Consolas,monospace}blockquote{margin:0.8em 1.5em;opacity:0.85}";
    zip.start_file("OEBPS/style.css", opts).map_err(|e| e.to_string())?;
    zip.write_all(css.as_bytes()).map_err(|e| e.to_string())?;

    let mut navpoints = String::new();
    for (i, ch) in book.toc.iter().enumerate() {
        navpoints.push_str(&format!(
            r#"<navPoint id="np{i}" playOrder="{i1}"><navLabel><text>{t}</text></navLabel><content src="c{i}.xhtml"/></navPoint>"#,
            i1 = i + 1,
            t = crate::txt::escape_html(&ch.title),
        ));
    }
    let ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:{uuid}"/><meta name="dtb:depth" content="1"/></head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>{navpoints}</navMap>
</ncx>"#,
        uuid = uuid,
        title = crate::txt::escape_html(&book.title),
        navpoints = navpoints,
    );
    zip.start_file("OEBPS/toc.ncx", opts).map_err(|e| e.to_string())?;
    zip.write_all(ncx.as_bytes()).map_err(|e| e.to_string())?;

    let nav_items: String = book
        .toc
        .iter()
        .enumerate()
        .map(|(i, ch)| format!("<li><a href=\"c{i}.xhtml\">{}</a></li>", crate::txt::escape_html(&ch.title)))
        .collect::<Vec<_>>()
        .join("\n");
    let nav_xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>{t}</title></head>
<body><nav epub:type="toc" id="toc"><h1>{t}</h1><ol>
{items}
</ol></nav></body></html>"#,
        t = crate::txt::escape_html(&book.title),
        items = nav_items,
    );
    zip.start_file("OEBPS/nav.xhtml", opts).map_err(|e| e.to_string())?;
    zip.write_all(nav_xhtml.as_bytes()).map_err(|e| e.to_string())?;

    for (i, body_html) in book.chapters.iter().enumerate() {
        let title = book.toc.get(i).map(|t| t.title.as_str()).unwrap_or("");
        let safe = sanitize_xhtml(body_html);
        let xhtml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>{t}</title><link rel="stylesheet" type="text/css" href="style.css"/></head>
<body><h2>{t}</h2>
{body}
</body></html>"#,
            t = crate::txt::escape_html(title),
            body = safe,
        );
        zip.start_file(format!("OEBPS/c{i}.xhtml"), opts).map_err(|e| e.to_string())?;
        zip.write_all(xhtml.as_bytes()).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tp_convert_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn tmp_leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tidepage-tmp-"))
            .collect()
    }

    fn small_book() -> ConvertedBook {
        ConvertedBook {
            title: "书名".into(),
            author: "作者".into(),
            toc: vec![TocEntry { title: "第一章".into(), index: 0, chars: 4 }],
            chapters: vec!["第一章\n正文内容".into()],
            plain: "第一章\n正文内容".into(),
        }
    }

    /// H2①：目标已存在时成功写入并原子替换，目录里不留临时文件。
    #[test]
    fn atomic_write_replaces_existing_target() {
        let d = tmpdir("ok");
        let target = d.join("out.txt");
        std::fs::write(&target, b"old").unwrap();
        atomic_write(target.to_str().unwrap(), |f| {
            f.write_all(b"new").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(tmp_leftovers(&d).is_empty(), "成功路径不得残留临时文件");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// H2②：闭包中途返回 Err 后旧文件字节不变、无临时文件残留。
    #[test]
    fn atomic_write_failure_keeps_old_bytes_and_cleans_tmp() {
        let d = tmpdir("fail");
        let target = d.join("out.txt");
        std::fs::write(&target, b"old-bytes").unwrap();
        let err = atomic_write(target.to_str().unwrap(), |f| {
            f.write_all(b"partial").map_err(|e| e.to_string())?;
            Err("模拟写入失败".to_string())
        })
        .unwrap_err();
        assert!(err.contains("模拟写入失败"));
        assert_eq!(std::fs::read(&target).unwrap(), b"old-bytes", "失败不得改动旧文件");
        assert!(tmp_leftovers(&d).is_empty(), "失败路径必须清理临时文件");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 对外接口仍走原子写：write_txt 替换旧内容，write_epub 产出可打开的 zip 且不留临时文件。
    #[test]
    fn write_txt_and_write_epub_use_atomic_write() {
        let d = tmpdir("api");
        let book = small_book();
        let txt = d.join("out.txt");
        std::fs::write(&txt, b"old").unwrap();
        write_txt(&book, txt.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::read(&txt).unwrap(), book.plain.as_bytes());

        let epub = d.join("out.epub");
        std::fs::write(&epub, b"old-epub").unwrap();
        write_epub(
            &book,
            &EpubOptions { cover: None, author: None, css: None },
            epub.to_str().unwrap(),
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&epub).unwrap()).unwrap();
        assert!(zip.by_name("mimetype").is_ok(), "写出的文件应是合法 EPUB zip");
        assert!(zip.by_name("OEBPS/c0.xhtml").is_ok());
        assert!(tmp_leftovers(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 父目录不存在 ⇒ Err（不产生临时文件，也不 panic）。
    #[test]
    fn atomic_write_missing_parent_is_error() {
        let d = tmpdir("noparent");
        let target = d.join("missing").join("out.txt");
        let err = atomic_write(target.to_str().unwrap(), |f| {
            f.write_all(b"x").map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(!err.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
    /// J24：实体与数字实体解码；未知实体保留；行内标签不换行、块级标签换行。
    #[test]
    fn html_to_text_entities_and_block_inline() {
        assert_eq!(html_to_text("<p>a &amp; b</p>"), "a & b");
        assert_eq!(html_to_text("&#20013;&#x6587;"), "中文");
        assert_eq!(html_to_text("&unknown;"), "&unknown;");
        assert_eq!(html_to_text("粗<b>体</b>不换行"), "粗体不换行");
        assert_eq!(html_to_text("<p>第一段</p><p>第二段</p>"), "第一段\n第二段");
        assert_eq!(html_to_text("行1<br/>行2"), "行1\n行2");
        assert_eq!(html_to_text("<div>甲<span>乙</span></div><div>丙</div>"), "甲乙\n丙");
        assert_eq!(html_to_text("<li>一</li><li>二</li>"), "一\n二");
    }

    /// J25：危险标签/属性被清除，合法 p/a/img 保留。
    #[test]
    fn sanitize_xhtml_strips_dangerous_attrs_and_tags() {
        let out = sanitize_xhtml(
            r#"<p>ok</p><img src="x" onerror="alert(1)"><a href="javascript:alert(2)">bad</a><script>evil()</script><style>p{}</style><iframe src="x"></iframe><object></object><form></form>"#,
        );
        assert!(out.contains("<p>ok</p>"), "{out}");
        assert!(out.contains("src=\"x\""), "{out}");
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("javascript:"), "{out}");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("evil()"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("<iframe"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("<object"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("<form"), "{out}");
        // 合法链接保留
        let ok = sanitize_xhtml(r#"<a href="https://example.com/x">链接</a>"#);
        assert!(ok.contains("https://example.com/x"), "{ok}");
    }

    /// P0-2：显式编码优先（GB18030 无 BOM），未知编码名回退自动探测且不报错。
    #[test]
    fn txt_to_epub_respects_forced_encoding() {
        let d = tmpdir("forced-enc");
        let p = d.join("gbk_book.txt");
        let (gb, _, _) = encoding_rs::GB18030.encode("第一章 你好世界，潮汐页。");
        std::fs::write(&p, &gb).unwrap();
        let path = p.to_str().unwrap();
        // 显式 gb18030：正文正确。
        let book = txt_to_epub_data(path, "", Some("gb18030")).unwrap();
        assert!(book.plain.contains("你好世界"), "{}", book.plain);
        // 未指定：自动链仍产出（GB18030 采样判定）。
        let auto = txt_to_epub_data(path, "", None).unwrap();
        assert!(auto.plain.contains("你好世界"), "{}", auto.plain);
        // 未知编码名：不得失败，回退自动探测。
        let unknown = txt_to_epub_data(path, "", Some("not-a-real-encoding")).unwrap();
        assert!(unknown.plain.contains("你好世界"), "{}", unknown.plain);
        let _ = std::fs::remove_dir_all(&d);
    }

}

#[cfg(test)]
mod r11_convert_tests {
    use super::*;

    fn build_epub_two_chapters() -> Vec<u8> {
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
            zw.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            zw.start_file("OEBPS/content.opf", deflated).unwrap();
            zw.write_all("<?xml version=\"1.0\" encoding=\"UTF-8\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>测试书</dc:title><dc:creator>作者</dc:creator></metadata><manifest><item id=\"c1\" href=\"c1.xhtml\" media-type=\"application/xhtml+xml\"/><item id=\"c2\" href=\"c2.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"c1\"/><itemref idref=\"c2\"/></spine></package>".as_bytes()).unwrap();
            zw.start_file("OEBPS/c1.xhtml", deflated).unwrap();
            zw.write_all("<html><body><p>第一章正文内容</p></body></html>".as_bytes()).unwrap();
            zw.start_file("OEBPS/c2.xhtml", deflated).unwrap();
            zw.write_all("<html><body><p>第二章正文内容</p></body></html>".as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    /// P1-2：EPUB→TXT 一次打开 zip 产出全部章节（顺序/内容与逐章 doc_text 等价）。
    #[test]
    fn epub_to_txt_collects_all_chapters_in_one_pass() {
        let d = std::env::temp_dir().join(format!("tp_epub_conv_r11_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("book.epub");
        std::fs::write(&p, build_epub_two_chapters()).unwrap();
        let book = epub_to_txt_data(p.to_str().unwrap()).unwrap();
        assert!(book.plain.contains("第一章正文内容"), "{}", book.plain);
        assert!(book.plain.contains("第二章正文内容"), "{}", book.plain);
        let first = book.plain.find("第一章正文内容").unwrap();
        let second = book.plain.find("第二章正文内容").unwrap();
        assert!(first < second, "章节顺序必须与 spine 一致");
        assert_eq!(book.title, "测试书");
        // 与逐章 doc_text 的结果一致（单章纯文本）。
        let data = std::fs::read(&p).unwrap();
        let doc = crate::epub::parse(&data).unwrap();
        assert_eq!(doc.spine.len(), 2);
        let texts = crate::epub::collect_texts(&data, &doc.spine);
        assert_eq!(texts.len(), 2);
        assert!(texts[0].contains("第一章正文内容") && texts[1].contains("第二章正文内容"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
