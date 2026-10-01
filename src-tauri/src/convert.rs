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

pub fn html_to_text(html: &str) -> String {
    let tag_re = Regex::new(r"(?is)<[^>]+>").unwrap();
    let s = tag_re.replace_all(html, "\n").into_owned();
    let ent_re = Regex::new(r"&(#?x?[0-9A-Za-z]+);").unwrap();
    let s = ent_re
        .replace_all(&s, |c: &regex::Captures| {
            match &c[1] {
                "nbsp" => " ".to_string(),
                "amp" => "&".to_string(),
                "lt" => "<".to_string(),
                "gt" => ">".to_string(),
                "quot" => "\"".to_string(),
                "apos" => "'".to_string(),
                _ => String::new(),
            }
        })
        .into_owned();
    s.lines()
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

pub fn txt_to_epub_data(input_path: &str, pattern: &str) -> Result<ConvertedBook, String> {
    let pattern = if pattern.trim().is_empty() { crate::default_pattern().to_string() } else { pattern.to_string() };
    let doc = txt::decode_and_split(input_path, Some(&pattern))?;
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
    let doc = crate::epub::parse(data.clone())?;
    let mut out = String::new();
    let mut toc = Vec::new();
    for (i, doc_path) in doc.spine.iter().enumerate() {
        let text = crate::epub::doc_text(&data, doc_path);
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

pub fn write_epub(book: &ConvertedBook, opts_in: &EpubOptions, out_path: &str) -> Result<(), String> {
    let author = opts_in.author.map(|a| a.trim()).filter(|a| !a.is_empty()).unwrap_or(&book.author);
    let file = std::fs::File::create(out_path).map_err(|e| e.to_string())?;
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
    std::fs::write(out_path, &book.plain).map_err(|e| e.to_string())?;
    Ok(())
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

fn sanitize_xhtml(html: &str) -> String {
    let tag_re = Regex::new(r"(?is)<\s*(script|style|link|meta|head|!doctype)[^>]*>").unwrap();
    let s = tag_re.replace_all(html, "").into_owned();
    let close_re = Regex::new(r"(?is)</\s*(script|style|link|meta|head)\s*>").unwrap();
    close_re.replace_all(&s, "").into_owned()
}

pub fn ai_to_epub(title: &str, author: &str, items: &[AiExportItem], out_path: &str) -> Result<(), String> {
    let book = build_ai_book(title, author, items);
    let file = std::fs::File::create(out_path).map_err(|e| e.to_string())?;
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

    let uuid = format!("{:032x}", crc_of(format!("ai:{title}").as_bytes()));
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
