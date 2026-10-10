//! 检查更新：查询 GitHub Releases 的最新版本。
//!
//! 网络请求放在 Rust 侧发起（而不是前端 fetch），一是复用既有网络能力，二是避免
//! 为此放宽 WebView 的 CSP `connect-src`。只访问仓库的公开 API，不带任何凭据。

use serde::Serialize;

const RELEASES_API: &str = "https://api.github.com/repos/miipa/TidePage/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/miipa/TidePage/releases/latest";

/// 最新 Release 的信息（字段全部来自 GitHub API，缺省值用空串兜底）。
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
        return Err(format!("服务器返回 {status}"));
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败：{e}"))?;

    let tag = v.get("tag_name").and_then(|x| x.as_str()).unwrap_or("");
    if tag.is_empty() {
        return Err("响应中没有版本信息".to_string());
    }

    Ok(Some(UpdateInfo {
        latest: tag.trim_start_matches(['v', 'V']).to_string(),
        url: v
            .get("html_url")
            .and_then(|x| x.as_str())
            .unwrap_or(RELEASES_PAGE)
            .to_string(),
        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        notes: v.get("body").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        published_at: v
            .get("published_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    }))
}
