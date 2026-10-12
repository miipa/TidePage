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
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    let low = cleaned.to_ascii_lowercase();
    if low.starts_with("javascript:") || low.starts_with("vbscript:") {
        return true;
    }
    low.starts_with("data:") && !low.starts_with("data:image/")
}

fn escape_attr_into(out: &mut String, v: &str) {
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn escape_text_into(out: &mut String, v: &str) {
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
    let name = tag.name.to_string();
    let is_end = matches!(tag.kind, TagKind::EndTag);
    let mut s = String::with_capacity(name.len() + 8);
    s.push('<');
    if is_end {
        s.push('/');
    }
    s.push_str(&name);
    if !is_end {
        for a in tag.attrs.iter() {
            let an = a.name.local.to_string();
            let anl = an.to_ascii_lowercase();
            if anl.starts_with("on") {
                continue;
            }
            let v = a.value.to_string();
            if is_url_attr(&anl) && is_dangerous_url(&v) {
                continue;
            }
            s.push(' ');
            s.push_str(&an);
            if !v.is_empty() {
                s.push_str("=\"");
                escape_attr(&v, &mut s);
                s.push('"');
            }
        }
        if tag.self_closing {
            s.push_str("/>");
            out.push_str(&s);
            return;
        }
    }
    s.push('>');
    out.push_str(&s);
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
        let mut st = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let ln = tag.name.to_string().to_ascii_lowercase();
                let is_end = matches!(tag.kind, TagKind::EndTag);
                if is_dangerous_tag(&ln) {
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
    let _ = tok.feed(&q);
    tok.end();
    let out = std::mem::take(&mut st.borrow_mut().out);
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
