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
            // 不设 client 级总超时：总超时会把整段请求（含 body 传输）一并计时，
            // 大文件流式下载/上传会中途 abort。改用 read_timeout（两次收到字节的最大间隔）
            // 捕获死连接而不限制总时长；连接超时与连接池策略保持不变。
            .read_timeout(std::time::Duration::from_secs(45))
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

    /// 带重试的原始请求（仅 Authorization/Depth 的便捷入口）。
    async fn raw(
        &self,
        method: &str,
        url: &str,
        depth: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response, String> {
        self.raw_ex(method, url, depth, body, &[]).await
    }

    /// 带重试的原始请求（覆盖传输错误与瞬时 429/5xx）；extra_headers 承载 Destination 等自定义头。
    async fn raw_ex(
        &self,
        method: &str,
        url: &str,
        depth: Option<&str>,
        body: Option<Vec<u8>>,
        extra_headers: &[(&str, &str)],
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
            for (k, v) in extra_headers {
                req = req.header(*k, *v);
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
        let code = resp.status().as_u16();
        // 成功：2xx / 207(Multi-Status)。405 多因部分服务器不允许对根做 PROPFIND，视为可连通。
        if resp.status().is_success() || code == 207 || code == 405 {
            return Ok(true);
        }
        // 失败：给出可读中文原因（命令签名保持 Result<bool, String> 不变，前端 catch 直接显示）。
        Err(match code {
            401 | 403 => "账号或密码错误 (HTTP 401/403)".to_string(),
            404 => "服务器地址或路径不存在 (HTTP 404)".to_string(),
            _ => format!("服务器返回 HTTP {code}"),
        })
    }

    pub async fn ensure_dir(&self, path: &str) -> Result<(), String> {
        // 逐级创建父目录：把 path 按 '/' 切成累积前缀（"a"、"a/b"、"a/b/c"…），
        // 对每一级做 MKCOL。201(已创建)/405(已存在)视为成功，其它码报错。
        // 空段（连续斜杠/前后导斜杠）跳过。深路径 PUT 前先建全父级，避免严格服务器 409。
        let mut prefix = String::new();
        for seg in path.split('/') {
            if seg.is_empty() {
                continue;
            }
            if prefix.is_empty() {
                prefix.push_str(seg);
            } else {
                prefix.push('/');
                prefix.push_str(seg);
            }
            let resp = self
                .raw("MKCOL", &join_url(&self.base, &prefix), Some("0"), None)
                .await?;
            let code = resp.status().as_u16();
            if code == 201 || code == 405 {
                continue;
            } else {
                return Err(format!("MKCOL {prefix} -> {code}"));
            }
        }
        Ok(())
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
        // 5 个正则只在首次使用时编译一次并复用（避免每次 list 重新编译）。
        use std::sync::OnceLock;
        static RE: OnceLock<regex::Regex> = OnceLock::new();
        static HREF_RE: OnceLock<regex::Regex> = OnceLock::new();
        static COLL_RE: OnceLock<regex::Regex> = OnceLock::new();
        static SIZE_RE: OnceLock<regex::Regex> = OnceLock::new();
        static LASTMOD_RE: OnceLock<regex::Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            regex::Regex::new(r"(?is)<(?:d:)?response>(.*?)</(?:d:)?response>").unwrap()
        });
        let href_re = HREF_RE.get_or_init(|| {
            regex::Regex::new(r"(?is)<(?:d:)?href>\s*(.*?)\s*</(?:d:)?href>").unwrap()
        });
        let coll_re = COLL_RE.get_or_init(|| {
            regex::Regex::new(r"(?is)<(?:d:)?collection\s*/?>").unwrap()
        });
        let size_re = SIZE_RE.get_or_init(|| {
            regex::Regex::new(r"(?is)<(?:d:)?getcontentlength>\s*(\d+)\s*</(?:d:)?getcontentlength>")
                .unwrap()
        });
        let lastmod_re = LASTMOD_RE.get_or_init(|| {
            regex::Regex::new(r"(?is)<(?:d:)?getlastmodified>\s*(.*?)\s*</(?:d:)?getlastmodified>")
                .unwrap()
        });
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

    /// DELETE：2xx 视为成功。
    pub async fn delete(&self, path: &str) -> Result<(), String> {
        let resp = self.raw("DELETE", &join_url(&self.base, path), None, None).await?;
        let code = resp.status().as_u16();
        if (200..=299).contains(&code) {
            Ok(())
        } else {
            Err(format!("DELETE -> {code}"))
        }
    }

    /// COPY：src → dst（Destination 用绝对 URL），Overwrite: F 不覆盖同名项；2xx 视为成功。
    pub async fn copy(&self, src: &str, dst: &str) -> Result<(), String> {
        let dest = join_url(&self.base, dst);
        let resp = self
            .raw_ex(
                "COPY",
                &join_url(&self.base, src),
                None,
                None,
                &[("Destination", &dest), ("Overwrite", "F")],
            )
            .await?;
        let code = resp.status().as_u16();
        if (200..=299).contains(&code) {
            Ok(())
        } else {
            Err(format!("COPY -> {code}"))
        }
    }

    /// MOVE：同 COPY，用 MOVE 方法（`move` 为 Rust 关键字，故命名 move_path）。
    pub async fn move_path(&self, src: &str, dst: &str) -> Result<(), String> {
        let dest = join_url(&self.base, dst);
        let resp = self
            .raw_ex(
                "MOVE",
                &join_url(&self.base, src),
                None,
                None,
                &[("Destination", &dest), ("Overwrite", "F")],
            )
            .await?;
        let code = resp.status().as_u16();
        if (200..=299).contains(&code) {
            Ok(())
        } else {
            Err(format!("MOVE -> {code}"))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_path() {
        // 空格 → %20
        assert_eq!(encode_path("a b"), "a%20b");
        // '+' → %2B（不被当作空格）
        assert_eq!(encode_path("a+b"), "a%2Bb");
        // 中文按 UTF-8 逐字节编码："中" = E4 B8 AD
        assert_eq!(encode_path("中"), "%E4%B8%AD");
        // 多字中文
        assert_eq!(encode_path("中文"), "%E4%B8%AD%E6%96%87");
        // '/' 分隔保留，段内空格照常编码
        assert_eq!(encode_path("a/b c"), "a/b%20c");
        // 字母数字与 -_.~ 保持不变
        assert_eq!(encode_path("aZ9-_.~"), "aZ9-_.~");
    }

    #[test]
    fn test_join_url() {
        // base 带尾斜杠：去掉尾斜杠后拼接
        assert_eq!(join_url("http://h/dav/", "a/b"), "http://h/dav/a/b");
        // base 不带尾斜杠
        assert_eq!(join_url("http://h/dav", "a"), "http://h/dav/a");
        // path 带前导斜杠：编码后去掉前导斜杠
        assert_eq!(join_url("http://h/dav", "/a"), "http://h/dav/a");
        // path 为空 → base + "/"
        assert_eq!(join_url("http://h/dav", ""), "http://h/dav/");
        assert_eq!(join_url("http://h/dav/", ""), "http://h/dav/");
    }

    #[test]
    fn test_percent_decode_path() {
        // "%20" → 空格
        assert_eq!(percent_decode_path("a%20b"), "a b");
        // 连续中文 %XX 还原为原中文
        assert_eq!(percent_decode_path("%E4%B8%AD%E6%96%87"), "中文");
        // 混合路径
        assert_eq!(percent_decode_path("/dav/%E4%B8%AD/x"), "/dav/中/x");
        // 结尾残缺 '%' 不 panic，原样保留
        assert_eq!(percent_decode_path("abc%"), "abc%");
        assert_eq!(percent_decode_path("ab%2"), "ab%2");
    }

    #[test]
    fn test_parse_rfc822() {
        // "Sun, 06 Sep 2026 15:01:53 GMT" → 按函数公式自算的确定 epoch 秒。
        // days(1970..=2025)=20454，2026 前 8 个月(Jan..Aug)=243 天，+ (6-1)=5 天 → 20702 天；
        // 20702*86400 + 15*3600 + 1*60 + 53 = 1_788_706_913
        assert_eq!(parse_rfc822("Sun, 06 Sep 2026 15:01:53 GMT"), 1_788_706_913);
        // 非法：字段不足
        assert_eq!(parse_rfc822("Sun, 06 Sep"), 0);
        // 非法：月名错误
        assert_eq!(parse_rfc822("Sun, 06 Xxx 2026 15:01:53 GMT"), 0);
        // 非法：星期错误
        assert_eq!(parse_rfc822("Xxx, 06 Sep 2026 15:01:53 GMT"), 0);
        // 非法：空串
        assert_eq!(parse_rfc822(""), 0);
    }
}
