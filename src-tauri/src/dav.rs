use crate::models::MoonProgress;
use crate::txt::decode_bytes;

pub struct DavClient {
    base: String,
    user: String,
    pass: String,
    http: reqwest::Client,
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path_enc = encode_path(path);
    let path = path_enc.trim_start_matches('/');
    if path.is_empty() {
        format!("{base}/")
    } else {
        format!("{base}/{path}")
    }
}

/// 按路径分段做 RFC3986 编码（保留 `/` 分隔），避免空格 / `+` / 中文等被错误解析
fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for seg in path.split('/') {
        if !out.is_empty() {
            out.push('/');
        }
        for b in seg.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
    }
    out
}

impl DavClient {
    pub fn new(base: &str, user: &str, pass: &str) -> Self {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(45))
            .connect_timeout(std::time::Duration::from_secs(20))
            // 后端边缘机 keep-alive 不稳，每次请求新建连接更稳
            .pool_max_idle_per_host(0)
            .build()
            .unwrap_or_default();
        DavClient {
            base: base.trim_end_matches('/').to_string(),
            user: user.into(),
            pass: pass.into(),
            http,
        }
    }

    fn auth(&self) -> Option<reqwest::header::HeaderValue> {
        use base64::Engine;
        let cred = format!("{}:{}", self.user, self.pass);
        reqwest::header::HeaderValue::from_str(&format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(cred)
        ))
        .ok()
    }

    /// 带重试的原始请求（覆盖传输错误与瞬时 429/5xx）
    async fn raw(
        &self,
        method: &str,
        url: &str,
        depth: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response, String> {
        let mut last: Option<String> = None;
        for attempt in 0..3 {
            let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "bad method")?;
            let mut req = self.http.request(method, url);
            if let Some(a) = self.auth() {
                req = req.header("Authorization", a);
            }
            if let Some(d) = depth {
                req = req.header("Depth", d);
            }
            if let Some(b) = &body {
                req = req.body(b.clone());
            }
            match req.send().await {
                Ok(r) => {
                    let s = r.status().as_u16();
                    if s == 429 || (500..=599).contains(&s) {
                        last = Some(format!("HTTP {s}"));
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        continue;
                    }
                    return Ok(r);
                }
                Err(e) => {
                    last = Some(e.to_string());
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| "请求失败".into()))
    }

    pub async fn test(&self) -> Result<bool, String> {
        let resp = self.raw("PROPFIND", &self.base, Some("0"), None).await?;
        Ok(resp.status().is_success() || resp.status().as_u16() == 207)
    }

    pub async fn ensure_dir(&self, path: &str) -> Result<(), String> {
        let resp = self.raw("MKCOL", &join_url(&self.base, path), Some("0"), None).await?;
        let code = resp.status().as_u16();
        if code == 201 || code == 405 {
            Ok(())
        } else {
            Err(format!("MKCOL {path} -> {code}"))
        }
    }

    pub async fn put(&self, path: &str, data: Vec<u8>) -> Result<(), String> {
        let resp = self
            .raw("PUT", &join_url(&self.base, path), None, Some(data))
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("PUT {path} -> {}", resp.status()))
        }
    }

    pub async fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let resp = self.raw("GET", &join_url(&self.base, path), None, None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("GET {path} -> {}", resp.status()));
        }
        Ok(Some(resp.bytes().await.map_err(|e| e.to_string())?.to_vec()))
    }

    pub async fn get_stream(
        &self,
        path: &str,
    ) -> Result<Option<reqwest::Response>, String> {
        let resp = self.raw("GET", &join_url(&self.base, path), None, None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("GET {path} -> {}", resp.status()));
        }
        Ok(Some(resp))
    }

    pub async fn list(&self, path: &str) -> Result<Vec<(String, bool, i64, i64)>, String> {
        let resp = self.raw("PROPFIND", &join_url(&self.base, path), Some("1"), None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(vec![]);
        }
        if !(resp.status().is_success() || resp.status().as_u16() == 207) {
            return Err(format!("PROPFIND {path} -> {}", resp.status()));
        }
        let body = resp.text().await.map_err(|e| e.to_string())?;
        let re = regex::Regex::new(r"(?is)<(?:d:)?response>(.*?)</(?:d:)?response>").unwrap();
        let href_re = regex::Regex::new(r"(?is)<(?:d:)?href>\s*(.*?)\s*</(?:d:)?href>").unwrap();
        let coll_re = regex::Regex::new(r"(?is)<(?:d:)?collection\s*/?>").unwrap();
        let size_re =
            regex::Regex::new(r"(?is)<(?:d:)?getcontentlength>\s*(\d+)\s*</(?:d:)?getcontentlength>")
                .unwrap();
        let lastmod_re =
            regex::Regex::new(r"(?is)<(?:d:)?getlastmodified>\s*(.*?)\s*</(?:d:)?getlastmodified>")
                .unwrap();
        let mut out = Vec::new();
        for c in re.captures_iter(&body) {
            let seg = &c[1];
            let href = match href_re.captures(seg) {
                Some(h) => h[1].to_string(),
                None => continue,
            };
            let decoded = percent_decode_path(&href);
            let is_coll = coll_re.is_match(seg) || decoded.ends_with('/');
            let name = decoded
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string();
            let size = size_re
                .captures(seg)
                .and_then(|s| s[1].parse::<i64>().ok())
                .unwrap_or(0);
            let mtime = lastmod_re
                .captures(seg)
                .and_then(|s| Some(parse_rfc822(&s[1])))
                .unwrap_or(0);
            out.push((name, is_coll, size, mtime));
        }
        Ok(out)
    }
}

fn parse_rfc822(s: &str) -> i64 {
    // 形如 "Sun, 06 Sep 2026 15:01:53 GMT"（不理会纯角度/月名，近似取年序）
    let s = s.trim();
    let day = s.get(..3).unwrap_or("");
    if !(day.eq_ignore_ascii_case("Sun")
        || day.eq_ignore_ascii_case("Mon")
        || day.eq_ignore_ascii_case("Tue")
        || day.eq_ignore_ascii_case("Wed")
        || day.eq_ignore_ascii_case("Thu")
        || day.eq_ignore_ascii_case("Fri")
        || day.eq_ignore_ascii_case("Sat"))
    {
        return 0;
    }
    let rest = s.splitn(2, ',').nth(1).unwrap_or("").trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 5 {
        return 0;
    }
    // [日, 月, 年, 时, 分, 秒[, GMT]]
    let dd: i64 = parts[0].parse().unwrap_or(0);
    let mon_txt = parts[1];
    let yy: i64 = parts[2].parse().unwrap_or(2000);
    let hhmmss = parts[3];
    let month: i64 = match mon_txt.to_ascii_uppercase().as_str() {
        "JAN" => 1, "FEB" => 2, "MAR" => 3, "APR" => 4, "MAY" => 5, "JUN" => 6,
        "JUL" => 7, "AUG" => 8, "SEP" => 9, "OCT" => 10, "NOV" => 11, "DEC" => 12,
        _ => 0,
    };
    if month == 0 {
        return 0;
    }
    let hms: Vec<&str> = hhmmss.split(':').collect();
    let hh: i64 = hms.first().and_then(|x| x.parse().ok()).unwrap_or(0);
    let mm: i64 = hms.get(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    let ss: i64 = hms.get(2).and_then(|x| x.parse().ok()).unwrap_or(0);
    // 简化为距离 1970 的秒（按公历近似）
    let mut y = yy;
    let mut days = 0i64;
    while y > 1970 {
        days += if (y - 1) % 4 == 0 && ((y - 1) % 100 != 0 || (y - 1) % 400 == 0) { 366 } else { 365 };
        y -= 1;
    }
    let mdays = [31, if yy % 4 == 0 && (yy % 100 != 0 || yy % 400 == 0) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for i in 0..(month - 1) {
        days += mdays[i as usize];
    }
    days += dd - 1;
    days * 86400 + hh * 3600 + mm * 60 + ss
}

fn percent_decode_path(s: &str) -> String {
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
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn decode_progress_content(data: &[u8]) -> String {
    let (s, _) = decode_bytes(data);
    s
}

pub fn parse_moonreader_content(s: &str) -> Vec<MoonProgress> {
    let mut out = Vec::new();
    for line in s.lines() {
        let e = parse_moonreader_line(line);
        if e.file.is_empty() && e.title.is_empty() {
            continue;
        }
        out.push(e);
    }
    out
}

/// 解析静读天下 .po 进度文件单行：`<时间戳>*<章节>@<偏移>#<ID>:<百分比>%`
pub fn parse_po_line(line: &str) -> (Option<f64>, String, i64) {
    let line = line.trim();
    let mut pct = None;
    let mut chapter_at = String::new();
    let mut updated_at: i64 = 0;
    let parts: Vec<&str> = line.split('*').collect();
    if let Some(ts) = parts.first() {
        updated_at = ts.trim().parse().unwrap_or(0);
    }
    if parts.len() > 1 {
        let rest = parts[1];
        if let Some(hash_idx) = rest.find('#') {
            chapter_at = rest[..hash_idx].to_string();
            let tail = &rest[hash_idx + 1..];
            if let Some(col) = tail.find(':') {
                let p = tail[col + 1..].trim_end_matches('%');
                pct = p.trim().parse::<f64>().ok();
                if let Some(p) = pct {
                    pct = Some(if p <= 1.0 { p * 100.0 } else { p });
                }
            }
        }
    }
    (pct, chapter_at, updated_at)
}

pub fn parse_moonreader_line(line_in: &str) -> MoonProgress {
    let line = line_in.trim();
    let mut file = String::new();
    let mut title = String::new();
    let mut chapter: Option<String> = None;
    let mut pct: Option<f64> = None;
    for part in line.split('*') {
        if let Some(v) = part.strip_prefix("file=") {
            file = v.to_string();
        } else if let Some(v) = part.strip_prefix("book.title=") {
            title = v.to_string();
        } else if let Some(v) = part.strip_prefix("chapter.title=") {
            chapter = Some(v.to_string());
        } else if let Some(v) = part.strip_prefix("book.percent=") {
            pct = v.replace(',', ".").parse::<f64>().ok();
        }
    }
    // 静读天下进度常见 0..1 百分比，统一换算为 0..100
    if let Some(p) = pct {
        let p = if p <= 1.0 { p * 100.0 } else { p };
        pct = Some(p);
    }
    MoonProgress {
        file,
        title,
        chapter,
        pct,
    }
}
