//! 书籍正文 HTML 净化：危险标签（连内容）与危险属性统一过滤，供 EPUB 与 MOBI
//! 章节渲染共用。仅复用 Cargo 已引入的 html5ever tokenizer，不新增第三方净化依赖。
//!
//! 规则：
//! - 危险标签（`script`/`style`/`iframe`/`object`/`embed`/`applet`/`form`/`meta`/
//!   `base`/`link`/`frame`）连同内容整体移除；
//! - 所有保留标签：删掉名字以 `on` 开头（不区分大小写）的事件属性；
//! - URL 属性（`href`/`src`/`xlink:href`/`action`/`formaction` 等）值为
//!   `javascript:`/`vbscript:`/非图片 `data:` 时整条删除（`data:image/...` 放行）。

use html5ever::tokenizer::{Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};
use std::cell::RefCell;
use std::rc::Rc;
#[cfg(feature = "sanitize-profile")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "sanitize-profile")]
use std::sync::LazyLock;
#[cfg(feature = "sanitize-profile")]
use std::time::Instant;

/// B8 profile：仅当进程启动时设置 TIDEPAGE_PARSE_PROFILE 才累计；生产零输出。
#[cfg(feature = "sanitize-profile")]
static PROFILE_ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("TIDEPAGE_PARSE_PROFILE").is_some());
#[cfg(feature = "sanitize-profile")]
static TOKENIZE_NS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static SINK_NS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static SERIALIZE_NS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static ESCAPE_NS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static URL_NS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static SERIALIZED_TAGS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static OUT_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "sanitize-profile")]
static PROFILE_PRINTED: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "sanitize-profile")]
fn profile_on() -> bool { *PROFILE_ON }
#[cfg(feature = "sanitize-profile")]
fn add_ns(slot: &AtomicU64, t: Instant) { slot.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed); }

#[cfg(feature = "sanitize-profile")]
struct ProfGuard { t: Option<Instant>, slot: &'static AtomicU64 }
#[cfg(feature = "sanitize-profile")]
impl ProfGuard {
    fn new(slot: &'static AtomicU64) -> Self { Self { t: profile_on().then(Instant::now), slot } }
}
#[cfg(feature = "sanitize-profile")]
impl Drop for ProfGuard {
    fn drop(&mut self) { if let Some(t) = self.t { add_ns(self.slot, t); } }
}

#[cfg(feature = "sanitize-profile")]
fn profile_print_once() {
    if !profile_on() || PROFILE_PRINTED.swap(1, Ordering::Relaxed) != 0 { return; }
    let ns = |a: &AtomicU64| a.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    let (tokenize, sink, serialize, escape, url) = (ns(&TOKENIZE_NS), ns(&SINK_NS), ns(&SERIALIZE_NS), ns(&ESCAPE_NS), ns(&URL_NS));
    eprintln!("[sanitize-profile] tokenize={tokenize:.1}ms sink={sink:.1}ms serialize={serialize:.1}ms escape={escape:.1}ms url={url:.1}ms tags={} outBytes={}", SERIALIZED_TAGS.load(Ordering::Relaxed), OUT_BYTES.load(Ordering::Relaxed));
}


/// 危险标签：连同内容整体移除。
pub fn is_dangerous_tag(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "iframe"
            | "object"
            | "embed"
            | "applet"
            | "form"
            | "meta"
            | "base"
            | "link"
            | "frame"
    )
}

/// 无内容（void）危险标签：只需丢弃标签本身，后续内容不应连带丢弃。
pub fn is_void_dangerous(name: &str) -> bool {
    matches!(name, "meta" | "base" | "link" | "frame" | "embed")
}

/// URL 类属性：值为危险协议时整条删除。
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_url_attr(name: &str) -> bool {
    matches!(
        name,
        "href"
            | "src"
            | "xlink:href"
            | "action"
            | "formaction"
            | "poster"
            | "background"
            | "cite"
            | "data"
            | "srcset"
    )
}

/// 值是否为危险协议（`javascript:` / `vbscript:` / 非图片 `data:`）。
/// 先剔除空白与控制字符（浏览器解析 URL 时会忽略它们，如 `java\tscript:`）。
pub fn is_dangerous_url(value: &str) -> bool {
    #[cfg(feature = "sanitize-profile")]
    let _g = ProfGuard::new(&URL_NS);
    fn starts_ci(value: &str, prefix: &str) -> bool {
        let mut it = value.chars().filter(|c| !c.is_whitespace() && !c.is_control());
        for pc in prefix.chars() {
            match it.next() {
                Some(c) if c.eq_ignore_ascii_case(&pc) => {}
                _ => return false,
            }
        }
        true
    }
    if starts_ci(value, "javascript:") || starts_ci(value, "vbscript:") {
        return true;
    }
    starts_ci(value, "data:") && !starts_ci(value, "data:image/")
}

fn escape_attr_into(out: &mut String, v: &str) {
    #[cfg(feature = "sanitize-profile")]
    let _g = ProfGuard::new(&ESCAPE_NS);
    if !v.bytes().any(|b| matches!(b, b'&' | b'"')) {
        out.push_str(v);
        return;
    }
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn escape_text_into(out: &mut String, v: &str) {
    #[cfg(feature = "sanitize-profile")]
    let _g = ProfGuard::new(&ESCAPE_NS);
    if !v.bytes().any(|b| matches!(b, b'&' | b'<' | b'>')) {
        out.push_str(v);
        return;
    }
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// 安全标签透传序列化（EPUB/MOBI 共享骨架）：按 on* 跳过、危险 URL 跳过的规则逐属性输出，
/// 自闭合写 `/>`、否则写 `>`。属性值转义器由调用方传入——EPUB 用 escape_attr_into，
/// MOBI 用其 U+FFFD-剔除版——这是两路唯一差异，故两路输出逐字节不变。
pub(crate) fn serialize_safe_tag(out: &mut String, tag: &Tag, escape_attr: impl Fn(&str, &mut String)) {
    #[cfg(feature = "sanitize-profile")]
    let _g = ProfGuard::new(&SERIALIZE_NS);
    #[cfg(feature = "sanitize-profile")]
    SERIALIZED_TAGS.fetch_add(1, Ordering::Relaxed);
    #[cfg(feature = "sanitize-profile")]
    let out_before = out.len();
    let name = tag.name.as_ref();
    let is_end = matches!(tag.kind, TagKind::EndTag);
    out.push('<');
    if is_end {
        out.push('/');
    }
    out.push_str(name);
    if !is_end {
        for a in tag.attrs.iter() {
            let an = a.name.local.as_ref();
            if an.len() >= 2 && an.as_bytes()[0].eq_ignore_ascii_case(&b'o') && an.as_bytes()[1].eq_ignore_ascii_case(&b'n') {
                continue;
            }
            let v = a.value.as_ref();
            if is_url_attr_ci(an) && is_dangerous_url(v) {
                continue;
            }
            out.push(' ');
            out.push_str(an);
            if !v.is_empty() {
                out.push_str("=\"");
                escape_attr(v, out);
                out.push('"');
            }
        }
        if tag.self_closing {
            out.push_str("/>");
            return;
        }
    }
    out.push('>');
    #[cfg(feature = "sanitize-profile")]
    OUT_BYTES.fetch_add((out.len() - out_before) as u64, Ordering::Relaxed);
}

fn is_url_attr_ci(name: &str) -> bool {
    #[cfg(feature = "sanitize-profile")]
    let _g = ProfGuard::new(&URL_NS);
    const URL_ATTRS: [&str; 10] = ["href", "src", "xlink:href", "action", "formaction", "poster", "background", "cite", "data", "srcset"];
    URL_ATTRS.iter().any(|a| name.eq_ignore_ascii_case(a))
}

fn is_dangerous_tag_ci(name: &str) -> bool {
    const DANGEROUS: [&str; 11] = ["script", "style", "iframe", "object", "embed", "applet", "form", "meta", "base", "link", "frame"];
    DANGEROUS.iter().any(|a| name.eq_ignore_ascii_case(a))
}

/// 危险标签的 drop-stack 转移（EPUB/MOBI 共享）：结束标签向下截断同名；开始标签若非 void
/// 非自闭合则压栈。`dangling=true`（仅 MOBI 悬空 RAWTEXT 模式）时不压栈——去标签、文本按
/// 普通处理；EPUB 恒传 false，与抽取前行为逐字节一致。调用方须已判定 is_dangerous_tag(&ln)
/// 为真，调用后立即 return（该 token 已被消费）。
pub(crate) fn handle_dangerous_tag(
    drop_stack: &mut Vec<String>,
    ln: String,
    is_end: bool,
    self_closing: bool,
    dangling: bool,
) {
    if is_end {
        if let Some(pos) = drop_stack.iter().rposition(|t| t == &ln) {
            drop_stack.truncate(pos);
        }
    } else if !self_closing && !is_void_dangerous(&ln) {
        if dangling {
            return;
        }
        drop_stack.push(ln);
    }
}

struct St {
    out: String,
    /// 已打开的危险容器标签栈（小写）；非空时丢弃其间所有 token。
    drop_stack: Vec<String>,
}

struct Sink(Rc<RefCell<St>>);

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        #[cfg(feature = "sanitize-profile")]
        let _g = ProfGuard::new(&SINK_NS);
        let mut st = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let is_end = matches!(tag.kind, TagKind::EndTag);
                if is_dangerous_tag_ci(tag.name.as_ref()) {
                    let ln = tag.name.as_ref().to_ascii_lowercase();
                    handle_dangerous_tag(&mut st.drop_stack, ln, is_end, tag.self_closing, false);
                    return TokenSinkResult::Continue;
                }
                if !st.drop_stack.is_empty() {
                    return TokenSinkResult::Continue;
                }
                serialize_safe_tag(&mut st.out, &tag, |v, out| escape_attr_into(out, v));
            }
            Token::CharacterTokens(chars) if st.drop_stack.is_empty() => {
                escape_text_into(&mut st.out, &chars);
            }
            // doctype/comment 等不进入正文片段，直接丢弃。
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// 净化 HTML 片段：移除危险标签（含内容）与危险属性，保留其余标签/文本结构。
pub fn sanitize_fragment(html: &str) -> String {
    let st = Rc::new(RefCell::new(St {
        out: String::with_capacity(html.len() + 256),
        drop_stack: Vec::new(),
    }));
    let tok = Tokenizer::new(
        Sink(st.clone()),
        html5ever::tokenizer::TokenizerOpts::default(),
    );
    let q = html5ever::tokenizer::BufferQueue::default();
    q.push_back(html.into());
    #[cfg(feature = "sanitize-profile")]
    let t0 = profile_on().then(Instant::now);
    let _ = tok.feed(&q);
    tok.end();
    #[cfg(feature = "sanitize-profile")]
    if let Some(t) = t0 { add_ns(&TOKENIZE_NS, t); }
    let out = std::mem::take(&mut st.borrow_mut().out);
    #[cfg(feature = "sanitize-profile")]
    profile_print_once();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(html: &str) -> String {
        sanitize_fragment(html)
    }

    #[test]
    fn removes_event_handlers_and_scripts() {
        let out = cleaned(
            r#"<p onclick="x()">a</p><img src="x" onerror="alert(1)"><script>evil()</script><svg onload="alert(2)"></svg>"#,
        );
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("onload"), "{out}");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("evil()"), "{out}");
    }


    #[test]
    fn equivalence_case_attrs_urls_entities_selfclosing() {
        // 大小写标签/属性
        assert_eq!(cleaned(r#"<DIV CLASS="x">A</DIV>"#), r#"<div class="x">A</div>"#);
        assert_eq!(cleaned(r#"<a HREF="javascript:alert(1)">x</a>"#), r#"<a>x</a>"#);
        assert_eq!(cleaned(r#"<img SRC="data:image/png;base64,AA">"#), r#"<img src="data:image/png;base64,AA">"#);
        // 空白/控制字符/大小写绕过
        assert!(is_dangerous_url(" JaVa\tScRiPt:alert(1)"));
        assert!(is_dangerous_url("\u{0}vbscript:msgbox"));
        assert!(!is_dangerous_url("data:image/svg+xml;base64,AA"));
        assert!(is_dangerous_url("data:text/html,x"));
        // 实体转义
        assert_eq!(cleaned(r#"a &amp; b < c > d"#), r#"a &amp; b &lt; c &gt; d"#);
        assert_eq!(cleaned(r#"<p title="a&quot;b&c">t</p>"#), r#"<p title="a&quot;b&amp;c">t</p>"#);
        // 自闭合/嵌套危险标签
        assert_eq!(cleaned(r#"<br/><img src="x"/>"#), r#"<br/><img src="x"/>"#);
        assert_eq!(cleaned(r#"<div><script><b>x</b></script><span>ok</span></div>"#), r#"<div><span>ok</span></div>"#);
        assert_eq!(cleaned(r#"<style>p{}</style><meta charset=x>"#), "");
    }


    /// B8：合成 ~15MB HTML 的子阶段 profile（默认 ignore；需 TIDEPAGE_PARSE_PROFILE=1 运行）。
    #[test]
    #[ignore]
    fn profile_synthetic_15mb() {
        let mut html = String::with_capacity(16 * 1024 * 1024);
        while html.len() < 15 * 1024 * 1024 {
            html.push_str(r#"<p class="c" onclick="x()" title="a&amp;b">普通文本 text &amp; more <b>粗体</b> <a href="https://example.com/a?b=1&amp;c=2">链接</a></p>"#);
            html.push('\n');
        }
        eprintln!("[sanitize-profile] inputBytes={}", html.len());
        let out = sanitize_fragment(&html);
        eprintln!("[sanitize-profile] resultBytes={}", out.len());
        assert!(!out.is_empty());
    }

    #[test]
    fn dangerous_url_no_alloc_semantics() {
        for s in ["javascript:1", "JAVASCRIPT:1", " java\nscript:1", "vbscript:1", "data:text/html,1"] {
            assert!(is_dangerous_url(s), "{s:?}");
        }
        for s in ["https://x", "data:image/png;base64,AA", "data:image/", "mailto:a@b", "/a:b"] {
            assert!(!is_dangerous_url(s), "{s:?}");
        }
    }

    #[test]
    fn removes_dangerous_url_protocols() {
        let out = cleaned(
            r#"<a href="javascript:alert(1)">x</a><a href="java&#09;script:alert(2)">y</a><a href="vbscript:msgbox">z</a><a href="data:text/html,<script>1</script>">w</a>"#,
        );
        assert!(!out.contains("javascript:"), "{out}");
        assert!(!out.contains("vbscript:"), "{out}");
        assert!(!out.contains("data:text/html"), "{out}");
        // 链接文本结构仍在
        assert!(out.contains(">x</a>"), "{out}");
    }

    #[test]
    fn removes_iframe_and_protocol_selectors() {
        let out = cleaned(r#"<iframe src="https://evil.example"></iframe><p>after</p>"#);
        assert!(!out.contains("iframe"), "{out}");
        assert!(!out.contains("evil.example"), "{out}");
        assert!(out.contains("<p>after</p>"), "{out}");
    }

    #[test]
    fn removes_object_form_meta_base_link() {
        let out = cleaned(
            r#"<object data="evil.swf"><param name="x"/></object><form action="javascript:post()"><input name="a"/></form><meta http-equiv="refresh" content="0"><base href="https://evil/"><link rel="stylesheet" href="x.css"><p>keep</p>"#,
        );
        assert!(!out.contains("object"), "{out}");
        assert!(!out.contains("<form"), "{out}");
        assert!(!out.contains("<meta"), "{out}");
        assert!(!out.contains("<base"), "{out}");
        assert!(!out.contains("<link"), "{out}");
        assert!(!out.contains("evil.swf"), "{out}");
        assert!(out.contains("<p>keep</p>"), "{out}");
    }

    #[test]
    fn keeps_normal_images_and_structure() {
        let out = cleaned(r#"<p><img src="a.jpg" alt="cover"/></p>"#);
        assert!(out.contains("<p>"), "{out}");
        assert!(out.contains("src=\"a.jpg\""), "{out}");
        assert!(out.contains("alt=\"cover\""), "{out}");
        assert!(out.contains("<img"), "{out}");
    }

    #[test]
    fn allows_data_image_and_relative_href() {
        let out = cleaned(
            r##"<img src="data:image/png;base64,AAAA"/><a href="chapter2.xhtml">next</a><a href="#top">top</a>"##,
        );
        assert!(out.contains("data:image/png;base64,AAAA"), "{out}");
        assert!(out.contains("chapter2.xhtml"), "{out}");
        assert!(out.contains("#top"), "{out}");
    }

    #[test]
    fn escapes_text_so_stray_markup_cannot_reform() {
        let out = cleaned(r#"<p>a &lt;b&gt; c &amp; d</p>"#);
        assert!(out.contains("&lt;b&gt;"), "{out}");
        assert!(out.contains("&amp;"), "{out}");
    }

    // ===== 骨架抽取特征测试（phase 1）：锁定序列化/危险转移/属性转义的逐字节输出，
    // 证明抽出共享 helper 后两路输出不变。断言值为抽取前的真实输出。 =====
    #[test]
    fn char_self_closing_dangerous_not_swallow() {
        // 自闭合危险标签只丢自身，不入 drop-stack，不吞后续文本。
        assert_eq!(sanitize_fragment("<script/>after<embed/>end"), "afterend");
    }

    #[test]
    fn char_void_meta_dropped_content_flows() {
        assert_eq!(
            sanitize_fragment("<p>a</p><meta charset=\"x\"><p>b</p>"),
            "<p>a</p><p>b</p>"
        );
    }

    #[test]
    fn char_attr_escaping_exact() {
        assert_eq!(
            sanitize_fragment("<a href=\"x?a=1&b=2\" title='q\"q'>t</a>"),
            "<a href=\"x?a=1&amp;b=2\" title=\"q&quot;q\">t</a>"
        );
    }

    #[test]
    fn char_img_self_closing_preserved_exact() {
        assert_eq!(
            sanitize_fragment("<p><img src=\"a.jpg\" alt=\"c\"/></p>"),
            "<p><img src=\"a.jpg\" alt=\"c\"/></p>"
        );
    }

    /// R10-3：正文 <a> 的 id/name 属性必须保留（内部锚点跳转依赖），危险属性照旧删除。
    #[test]
    fn preserves_anchor_id_and_name_attributes() {
        let out = cleaned(
            r##"<p><a id="top"></a><a name="sec2"></a><a id="x" href="#y" onclick="e()">t</a></p>"##,
        );
        assert!(out.contains(r#"id="top""#), "{out}");
        assert!(out.contains(r#"name="sec2""#), "{out}");
        assert!(out.contains(r##"href="#y""##), "{out}");
        assert!(!out.contains("onclick"), "{out}");
    }

    // 固定 void 危险标签集：mobi.rs::decode_and_sanitize 复用此判定，
    // 这里钉死集合，防止两条清洗路径对 void 集静默分叉。
    #[test]
    fn void_dangerous_set_is_pinned() {
        for t in ["meta", "base", "link", "frame", "embed"] {
            assert!(is_void_dangerous(t), "expected void-dangerous: {t}");
        }
        for t in ["script", "style", "iframe", "object", "form", "p", "img"] {
            assert!(!is_void_dangerous(t), "must not be void-dangerous: {t}");
        }
    }
}
