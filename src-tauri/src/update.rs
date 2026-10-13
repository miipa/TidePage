//! 检查更新：查询 GitHub Releases 的最新版本。
//!
//! 网络请求放在 Rust 侧发起（而不是前端 fetch），一是复用既有网络能力，二是避免
//! 为此放宽 WebView 的 CSP `connect-src`。只访问仓库的公开资源，不带任何凭据。
//!
//! 主数据源是仓库的 Atom feed（`releases.atom`，走 github.com，不占 `api.github.com`
//! 未认证 60 次/小时的限额）；REST API 仅在 feed 非 200 或解析失败时兜底。
//!
//! 注意本仓 release 的 tag 是 `release-YYYYMMDD`（不含版本号），真正的版本号只出现在
//! release 标题（`TidePage v0.1.25 · release-20261013`）与资源文件名
//! （`TidePage-0.1.25-windows-x64-portable.zip`）里，所以版本号统一由 `pick_version`
//! 按 tag → 标题 → 正文的顺序挑出，两条数据源共用同一入口。

use serde::Serialize;

const RELEASES_ATOM: &str = "https://github.com/miipa/TidePage/releases.atom";
const RELEASES_API: &str = "https://api.github.com/repos/miipa/TidePage/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/miipa/TidePage/releases/latest";

/// 最新 Release 的信息（字段全部来自 GitHub，缺省值用空串兜底）。
#[derive(Serialize)]
pub struct UpdateInfo {
    /// 最新版本号（已去掉可能的 `v` 前缀，如 `0.1.26`）。
    pub latest: String,
    /// 最新 Release 的网页地址（用于「前往下载」）。
    pub url: String,
    /// Release 标题（可能为空）。
    pub name: String,
    /// Release 说明（Markdown，可能为空）。
    pub notes: String,
    /// 发布时间（ISO 8601，可能为空）。
    pub published_at: String,
}

/// 依次从 tag、release 标题、正文（资源文件名）里挑出版本号，返回规整的 "x.y.z"。
///
/// `release-20261013` 这类日期 tag 没有 `x.y.z` 分量，`release` 前缀也不该被
/// `parseInt` 读成 0，所以只认 `v?x.y.z` 形态；三处都挑不出时返回 `None`，
/// 由调用方报「响应中没有版本信息」，绝不退化成 0.0.0 让前端永远显示「已是最新」。
fn pick_version(tag: &str, name: &str, body: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)v?(\d+\.\d+\.\d+)").ok()?;
    for cand in [tag, name, body] {
        if let Some(c) = re.captures(cand) {
            return Some(c[1].to_string());
        }
    }
    None
}

/// Atom feed 里第一个 `<entry>`（= 最新 release）里本次要用的字段。
struct AtomEntry {
    url: String,
    name: String,
    tag: String,
    /// `<content>` 解码后的文本，仅用于版本号兜底（正文是 HTML，不当 notes 用）。
    body: String,
}

/// 元素本地名（去掉 `prefix:`），规避 feed 的命名空间前缀差异。
fn local_name(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.into_owned(),
    }
}

/// 从 `<id>` 取 tag：优先 `.../releases/tag/<tag>` 之后的一段，否则取最后一个 `/` 段。
/// GitHub 实际给的是 `tag:github.com,2008:Repository/1412956926/release-20261013`
/// （后者即「最后一段」），两种形态都覆盖。
fn tag_from_id(id: &str) -> String {
    const PAT: &str = "/releases/tag/";
    if let Some(i) = id.find(PAT) {
        return id[i + PAT.len()..].trim_end_matches('/').to_string();
    }
    id.rsplit('/').next().unwrap_or("").trim_end_matches('/').to_string()
}

/// 解析 `<entry>` / `<link>` 上的属性，只认 `rel="alternate"` 的 href。
fn alternate_href(e: &quick_xml::events::BytesStart) -> Option<String> {
    let mut rel = String::new();
    let mut href = String::new();
    for a in e.attributes().flatten() {
        let key = local_name(a.key.as_ref());
        let val = a
            .unescape_value()
            .map(|v| v.into_owned())
            .unwrap_or_default();
        match key.as_str() {
            "rel" => rel = val,
            "href" => href = val,
            _ => {}
        }
    }
    if rel == "alternate" && !href.is_empty() {
        Some(href)
    } else {
        None
    }
}

/// 解析 `releases.atom`，取**第一个 `<entry>`**（= 最新 release）。
///
/// 刻意跳过 feed 自身的 `<title>Release notes from TidePage</title>`：它在 `<entry>`
/// 之外，靠 `in_entry` 门闩隔离。解析不出任何 entry 时返回 `None`（交给 API 兜底）。
fn parse_latest_atom(xml: &str) -> Option<AtomEntry> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_entry = false;
    let mut cur = String::new();
    let mut url = String::new();
    let mut name = String::new();
    let mut id = String::new();
    let mut body = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = local_name(e.name().as_ref());
                if tag == "entry" {
                    if in_entry {
                        // 嵌套 entry（不该出现）：丢弃外层，以内层为准
                        url.clear();
                        name.clear();
                        id.clear();
                        body.clear();
                    }
                    in_entry = true;
                    cur = tag;
                    continue;
                }
                if in_entry && tag == "link" {
                    if url.is_empty() {
                        if let Some(h) = alternate_href(&e) {
                            url = h;
                        }
                    }
                }
                cur = tag;
            }
            Ok(Event::Empty(e)) => {
                // `<link ... />` 无子元素，属性在 Empty 事件上
                let tag = local_name(e.name().as_ref());
                if in_entry && tag == "link" && url.is_empty() {
                    if let Some(h) = alternate_href(&e) {
                        url = h;
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if !in_entry {
                    continue;
                }
                let txt = t.unescape().unwrap_or_default().to_string();
                match cur.as_str() {
                    "title" if name.is_empty() => name = txt,
                    "id" if id.is_empty() => id = txt,
                    "content" => body.push_str(&txt),
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let tag = local_name(e.name().as_ref());
                if tag == "entry" && in_entry {
                    let mut tag_s = tag_from_id(&id);
                    if tag_s.is_empty() {
                        // id 缺失/异常时退回 link 的 `.../releases/tag/<tag>`
                        tag_s = tag_from_id(&url);
                    }
                    return Some(AtomEntry {
                        url,
                        name,
                        tag: tag_s,
                        body,
                    });
                }
                cur.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

/// 非 2xx 的可读错误：403/429 是 GitHub 的限流（未认证 API 60 次/小时），
/// 单独给出「限流」提示；其余维持原来的 `服务器返回 {status}`。
fn status_err(status: reqwest::StatusCode) -> String {
    let code = status.as_u16();
    if code == 403 || code == 429 {
        "GitHub 接口请求过于频繁（触发限流），请稍后再试".to_string()
    } else {
        format!("服务器返回 {status}")
    }
}

/// 查询最新发布版本。
///
/// 返回 `Ok(None)` 表示仓库**尚无任何 Release**（GitHub 返回 404），不是错误；
/// 其余网络/解析问题返回 `Err`（可读中文，交给前端提示）。
#[tauri::command]
pub async fn check_update() -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("初始化网络失败：{e}"))?;

    // ① 主源：Atom feed（不计 API 限额）。非 200 或解析不出 entry 时落到下面的 API 兜底。
    if let Ok(resp) = client
        .get(RELEASES_ATOM)
        .header("User-Agent", "TidePage")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(xml) = resp.text().await {
                if let Some(entry) = parse_latest_atom(&xml) {
                    let latest = pick_version(&entry.tag, &entry.name, &entry.body)
                        .ok_or_else(|| "响应中没有版本信息".to_string())?;
                    return Ok(Some(UpdateInfo {
                        latest,
                        url: entry.url,
                        name: entry.name,
                        // Atom 的 <content> 是 HTML（不是 Markdown 说明），发布时间本次也
                        // 不取 —— 前端只用 latest/url，两字段留空串。
                        notes: String::new(),
                        published_at: String::new(),
                    }));
                }
            }
        }
    }

    // ② 兜底：GitHub REST API（保持 404 → Ok(None) 语义）。
    let resp = client
        .get(RELEASES_API)
        // GitHub API 要求带 User-Agent，否则直接 403。
        .header("User-Agent", "TidePage")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("网络请求失败：{e}"))?;

    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(status_err(status));
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败：{e}"))?;

    let tag = v.get("tag_name").and_then(|x| x.as_str()).unwrap_or("");
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
    let body = v.get("body").and_then(|x| x.as_str()).unwrap_or("");
    let latest = pick_version(tag, name, body).ok_or_else(|| "响应中没有版本信息".to_string())?;

    Ok(Some(UpdateInfo {
        latest,
        url: v
            .get("html_url")
            .and_then(|x| x.as_str())
            .unwrap_or(RELEASES_PAGE)
            .to_string(),
        name: name.to_string(),
        notes: body.to_string(),
        published_at: v
            .get("published_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_version_from_date_tag() {
        // tag 里没有 x.y.z → 落到 release 标题
        assert_eq!(
            pick_version("release-20261013", "TidePage v0.1.25 · release-20261013", ""),
            Some("0.1.25".to_string())
        );
    }

    #[test]
    fn pick_version_from_semver_tag() {
        assert_eq!(
            pick_version("v0.1.26", "TidePage v0.1.26", ""),
            Some("0.1.26".to_string())
        );
    }

    #[test]
    fn pick_version_from_asset_filename() {
        // 标题也不含版本号时，靠正文里的资源文件名兜底
        assert_eq!(
            pick_version(
                "release-20261011",
                "TidePage release-20261011",
                "<code>TidePage-0.1.25-windows-x64-portable.zip</code>"
            ),
            Some("0.1.25".to_string())
        );
    }

    #[test]
    fn pick_version_none_when_nothing_matches() {
        // 三处都挑不出版本号：返回 None（由调用方报错），不退化成 0.0.0
        assert_eq!(
            pick_version("release-20261014", "TidePage release-20261014", ""),
            None
        );
    }

    /// Atom 解析：只取第一个 <entry>，feed 自身的 <title> 不能被当成 release 标题。
    #[test]
    fn parse_atom_first_entry() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <id>tag:github.com,2008:https://github.com/miipa/TidePage/releases</id>
  <link type="text/html" rel="alternate" href="https://github.com/miipa/TidePage/releases"/>
  <title>Release notes from TidePage</title>
  <updated>2026-10-13T12:32:56Z</updated>
  <entry>
    <id>tag:github.com,2008:Repository/1412956926/release-20261013</id>
    <updated>2026-10-13T13:14:57Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/miipa/TidePage/releases/tag/release-20261013"/>
    <title>TidePage v0.1.25 · release-20261013</title>
    <content type="html">&lt;h3&gt;修复&lt;/h3&gt;&lt;p&gt;&lt;code&gt;TidePage-0.1.25-windows-x64-portable.zip&lt;/code&gt;&lt;/p&gt;</content>
  </entry>
  <entry>
    <id>tag:github.com,2008:Repository/1412956926/release-20261012</id>
    <link rel="alternate" type="text/html" href="https://github.com/miipa/TidePage/releases/tag/release-20261012"/>
    <title>TidePage v0.1.24 · release-20261012</title>
    <content type="html">older</content>
  </entry>
</feed>"#;
        let e = parse_latest_atom(xml).expect("应解析出第一个 entry");
        assert_eq!(
            e.url,
            "https://github.com/miipa/TidePage/releases/tag/release-20261013"
        );
        assert_eq!(e.name, "TidePage v0.1.25 · release-20261013");
        assert_eq!(e.tag, "release-20261013");
        assert!(e.body.contains("TidePage-0.1.25-windows-x64-portable.zip"));
        assert_eq!(pick_version(&e.tag, &e.name, &e.body), Some("0.1.25".to_string()));
    }

    /// Atom 里没有 <entry>（仓库无 release）→ None，交给 API 兜底。
    #[test]
    fn parse_atom_empty_feed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Release notes from TidePage</title>
</feed>"#;
        assert!(parse_latest_atom(xml).is_none());
    }

    #[test]
    fn tag_from_id_shapes() {
        assert_eq!(
            tag_from_id("tag:github.com,2008:Repository/1412956926/release-20261013"),
            "release-20261013"
        );
        assert_eq!(
            tag_from_id("https://github.com/miipa/TidePage/releases/tag/v0.1.26"),
            "v0.1.26"
        );
    }

    #[test]
    fn rate_limit_is_readable() {
        assert_eq!(
            status_err(reqwest::StatusCode::FORBIDDEN),
            "GitHub 接口请求过于频繁（触发限流），请稍后再试"
        );
        assert_eq!(
            status_err(reqwest::StatusCode::TOO_MANY_REQUESTS),
            "GitHub 接口请求过于频繁（触发限流），请稍后再试"
        );
        assert_eq!(
            status_err(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
            "服务器返回 500 Internal Server Error"
        );
    }

    /// 真实抓取自测（联网，默认 ignore，避免离线跑测试失败）。
    /// 手动执行：`cargo test --lib -- --ignored --nocapture`。
    /// 断言形状 + 打印实际值；不钉死具体版本号，否则下一次发版这条测试就会过期。
    #[tokio::test]
    #[ignore = "需要联网：手动验证真实抓取结果"]
    async fn real_fetch_latest_release() {
        let info = check_update()
            .await
            .expect("网络请求不应失败")
            .expect("仓库应已有 release");
        assert!(
            regex::Regex::new(r"^\d+\.\d+\.\d+$").unwrap().is_match(&info.latest),
            "latest 应是规整的 x.y.z，实际：{}",
            info.latest
        );
        assert!(
            info.url
                .starts_with("https://github.com/miipa/TidePage/releases/tag/"),
            "url 应指向 release tag，实际：{}",
            info.url
        );
        assert!(!info.name.is_empty(), "name（release 标题）不应为空");
        println!(
            "latest={} name={} url={}",
            info.latest, info.name, info.url
        );
    }
}
