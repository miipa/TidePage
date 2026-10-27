use std::sync::Arc;

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

/// S7：远端路径段校验。防「..」穿越/越权访问父目录：
/// - 空段（连续斜杠、前导/尾随斜杠）跳过；
/// - 段等于 `.` 或 `..` 直接拒绝；
/// - percent-encoded 变体（%2e%2e、%2E%2E）解码后再判；解码结果含 / 或反斜杠也拒绝，
///   避免 %2e%2e%2f 这类编码绕过。
/// 正常中文/空格/加号/以点开头的普通文件名（..foo、.hidden）不受影响。
fn validate_remote_path(path: &str) -> Result<(), String> {
    for seg in path.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg == "." || seg == ".." {
            return Err(format!("非法路径段：{seg}"));
        }
        let decoded = percent_decode_path(seg);
        if decoded == "." || decoded == ".." || decoded.contains('/') || decoded.contains('\\') {
            return Err("非法路径段（疑似目录穿越）".to_string());
        }
    }
    Ok(())
}

/// S2：32 位平台单文件上限。32 位进程地址空间只有 2GiB 量级，超过它的单文件不应继续
/// 尝试（即使流式写盘也会在缓冲/映射等环节失败）。64 位保持原有 8GiB 语义不变。
const WEBDAV_32BIT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 32 位且超过 2GiB 时返回可读错误文案；其余返回 None（纯函数便于测试）。
pub(crate) fn too_large_for_arch(len: u64, ptr_width: u32) -> Option<&'static str> {
    if ptr_width == 32 && len > WEBDAV_32BIT_MAX_BYTES {
        Some("32 位版本单文件上限 2GiB，请改用 64 位版本")
    } else {
        None
    }
}

/// 同目录唯一 tmp 名：并发下载同一本书各自独立，rename 后互不覆盖。
fn download_tmp_path(target: &std::path::Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    let dir = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    dir.join(format!(".{name}.{}.{nanos}.tmp", std::process::id()))
}

impl DavClient {
    pub fn new(base: &str, user: &str, pass: &str) -> Result<Self, String> {
        // 仅本机/私有网段（局域网 NAS 常用自签名证书）放宽校验，公网 WebDAV 严格验证。
        let accept_invalid = crate::ai::is_local_or_private_url(base);
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(accept_invalid)
            // 不设 client 级总超时：总超时会把整段请求（含 body 传输）一并计时，
            // 大文件流式下载/上传会中途 abort。改用 read_timeout（两次收到字节的最大间隔）
            // 捕获死连接而不限制总时长；连接超时与连接池策略保持不变。
            .read_timeout(std::time::Duration::from_secs(45))
            .connect_timeout(std::time::Duration::from_secs(20))
            // 后端边缘机 keep-alive 不稳，每次请求新建连接更稳
            .pool_max_idle_per_host(0)
            .build()
            // P1-7：构建失败不能静默退化成默认客户端（会丢超时/TLS/代理策略），
            // 返回可读错误让前端提示检查代理/证书配置；不回显凭据。
            .map_err(|e| format!("创建 WebDAV 客户端失败（请检查代理与证书配置）：{e}"))?;
        Ok(DavClient {
            base: base.trim_end_matches('/').to_string(),
            user: user.into(),
            pass: pass.into(),
            http,
        })
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
        body: Option<Arc<Vec<u8>>>,
    ) -> Result<reqwest::Response, String> {
        self.raw_ex(method, url, depth, body, &[]).await
    }

    /// 带重试的原始请求（覆盖传输错误与瞬时 429/5xx）；extra_headers 承载 Destination 等自定义头。
    ///
    /// body 用 Arc 承载：构造请求时若只此一份则零拷贝交给 reqwest，
    /// 重试通过 try_clone 复用其 Bytes（只克隆句柄，不重复复制整块内存）。
    async fn raw_ex(
        &self,
        method: &str,
        url: &str,
        depth: Option<&str>,
        body: Option<Arc<Vec<u8>>>,
        extra_headers: &[(&str, &str)],
    ) -> Result<reqwest::Response, String> {
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "bad method")?;
        let mut builder = self.http.request(method, url);
        if let Some(a) = self.auth() {
            builder = builder.header("Authorization", a);
        }
        if let Some(d) = depth {
            builder = builder.header("Depth", d);
        }
        for (k, v) in extra_headers {
            builder = builder.header(*k, *v);
        }
        if let Some(data) = body {
            // Arc 唯一持有者时直接取出 Vec（零拷贝）；否则只拷一份交给 reqwest。
            let vec = match Arc::try_unwrap(data) {
                Ok(v) => v,
                Err(shared) => (*shared).clone(),
            };
            builder = builder.body(vec);
        }
        let mut last: Option<String> = None;
        for attempt in 0..3 {
            // 可复用 Body（Vec→Bytes）的 try_clone 只克隆 Bytes 句柄，不复制整块数据。
            let req = match builder.try_clone() {
                Some(r) => r,
                None => return Err("请求体不可重试".into()),
            };
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
        validate_remote_path(path)?;
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

    pub async fn put(&self, path: &str, data: Arc<Vec<u8>>) -> Result<(), String> {
        validate_remote_path(path)?;
        let resp = self
            .raw("PUT", &join_url(&self.base, path), None, Some(data))
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("PUT {path} -> {}", resp.status()))
        }
    }

    /// S7：PROPFIND 响应体上限（与 GET 的 16 MiB 口径一致）。
    /// 目录列表远小于此；恶意/异常的 WebDAV 服务器不能靠超长 XML 让客户端整段读入。
    const PROPFIND_MAX_BYTES: u64 = 16 * 1024 * 1024;

    /// 普通 GET：16 MiB 上限（进度/设置备份等小文件）。
    pub async fn get(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        self.get_limited(path, 16 * 1024 * 1024).await
    }

    /// 带上限的 GET：边流式读取边累计字节，超过 max_bytes 立即返回 Err（不信任远端 Content-Length）。
    /// 返回 Ok(None) = 404；其它非 2xx 返回 Err。
    pub async fn get_limited(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        validate_remote_path(path)?;
        use futures_util::StreamExt;
        let resp = self.raw("GET", &join_url(&self.base, path), None, None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("GET {path} -> {}", resp.status()));
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut received: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            received = received.saturating_add(chunk.len() as u64);
            if received > max_bytes {
                return Err(format!("GET {path} 超过上限 {max_bytes} 字节"));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(Some(buf))
    }

    pub async fn get_stream(
        &self,
        path: &str,
    ) -> Result<Option<reqwest::Response>, String> {
        validate_remote_path(path)?;
        let resp = self.raw("GET", &join_url(&self.base, path), None, None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("GET {path} -> {}", resp.status()));
        }
        Ok(Some(resp))
    }

    /// S2：流式上传本地文件（PUT）。请求体按块读取，内存与文件大小无关。
    /// 流式 body 不可重试，故单次尝试；错误按状态码给可读原因。
    pub async fn put_file(&self, path: &str, mut file: std::fs::File) -> Result<(), String> {
        use std::io::Read;
        validate_remote_path(path)?;
        let url = join_url(&self.base, path);
        // reqwest 0.12 的 Body::from 只接受 tokio::fs::File（需 tokio "fs" feature，本轮不允许改
        // Cargo.toml）；这里用「阻塞线程分块读 + mpsc → wrap_stream」实现等价流式 body，
        // 内存固定为 4 个 64KiB 块，与文件大小无关。
        let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(4);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                            break; // 请求已结束/失败，停止读盘
                        }
                    }
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        break;
                    }
                }
            }
        });
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        let mut builder = self.http.put(url).body(reqwest::Body::wrap_stream(stream));
        if let Some(a) = self.auth() {
            builder = builder.header("Authorization", a);
        }
        let resp = builder.send().await.map_err(|e| format!("PUT {path} 上传失败：{e}"))?;
        let code = resp.status().as_u16();
        if (200..=299).contains(&code) {
            Ok(())
        } else {
            Err(match code {
                401 | 403 => format!("PUT {path} 失败：账号密码错误或无权限 (HTTP {code})"),
                409 => format!("PUT {path} 失败：目录不存在或冲突 (HTTP {code})"),
                500..=599 => format!("PUT {path} 失败：服务器错误 (HTTP {code})"),
                _ => format!("PUT {path} -> HTTP {code}"),
            })
        }
    }

    /// S2：流式下载到文件：边读边写，超 max_bytes 立即中止并删 tmp；任何失败清理 tmp。
    /// 先写同目录唯一 tmp 再 rename，并发下载同一本书各自有独立 tmp，不互相覆盖。
    /// 404 返回 Ok(false)；其它非 2xx 返回 Err。
    pub async fn download_to_file(
        &self,
        path: &str,
        target: &std::path::Path,
        max_bytes: u64,
    ) -> Result<bool, String> {
        use futures_util::StreamExt;
        use std::io::Write;
        validate_remote_path(path)?;
        let resp = self.raw("GET", &join_url(&self.base, path), None, None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(false);
        }
        if !resp.status().is_success() {
            return Err(format!("GET {path} -> {}", resp.status()));
        }
        let tmp = download_tmp_path(target);
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("创建下载临时文件失败：{e}"))?;
        let mut received: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    drop(f);
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("下载 {path} 失败：{e}"));
                }
            };
            received = received.saturating_add(chunk.len() as u64);
            if received > max_bytes {
                drop(f);
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("文件超过上限 {max_bytes} 字节，已取消下载"));
            }
            if let Err(e) = f.write_all(&chunk) {
                drop(f);
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("写入下载临时文件失败：{e}"));
            }
        }
        if let Err(e) = f.sync_all() {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("同步下载临时文件失败：{e}"));
        }
        drop(f);
        // Windows：rename 不覆盖已存在目标；调用方通常已判不存在，这里兜底重试一次。
        if std::fs::rename(&tmp, target).is_err() {
            let _ = std::fs::remove_file(target);
            if let Err(e) = std::fs::rename(&tmp, target) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("保存下载文件失败：{e}"));
            }
        }
        Ok(true)
    }

    /// S3：读远端 + ETag（404 返回 None）。用于同步冲突检测。
    pub async fn get_with_etag(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<(Vec<u8>, Option<String>)>, String> {
        use futures_util::StreamExt;
        validate_remote_path(path)?;
        let resp = self.raw("GET", &join_url(&self.base, path), None, None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("GET {path} -> {}", resp.status()));
        }
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_length = resp.content_length();
        if let Some(cl) = content_length {
            if cl > max_bytes {
                return Err(format!("GET {path} 超过上限 {max_bytes} 字节"));
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut received: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            received = received.saturating_add(chunk.len() as u64);
            if body_limit_exceeded(received, max_bytes) {
                return Err(format!("GET {path} 超过上限 {max_bytes} 字节"));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(Some((buf, etag)))
    }

    /// S3：带条件上传。etag 存在加 If-Match；create_only 加 If-None-Match: *。
    /// 返回状态码（2xx/412/409 由调用方判定）。
    pub async fn put_if_match(
        &self,
        path: &str,
        data: Arc<Vec<u8>>,
        etag: Option<&str>,
        create_only: bool,
    ) -> Result<u16, String> {
        validate_remote_path(path)?;
        let url = join_url(&self.base, path);
        let body = (*data).clone();
        let mut builder = self.http.put(url).body(body);
        if let Some(a) = self.auth() {
            builder = builder.header("Authorization", a);
        }
        if let Some(tag) = etag {
            builder = builder.header(reqwest::header::IF_MATCH, tag);
        }
        if create_only {
            builder = builder.header(reqwest::header::IF_NONE_MATCH, "*");
        }
        let resp = builder.send().await.map_err(|e| format!("PUT {path} 上传失败：{e}"))?;
        Ok(resp.status().as_u16())
    }

    pub async fn list(&self, path: &str) -> Result<Vec<(String, bool, i64, i64)>, String> {
        validate_remote_path(path)?;
        let resp = self.raw("PROPFIND", &join_url(&self.base, path), Some("1"), None).await?;
        if resp.status().as_u16() == 404 {
            return Ok(vec![]);
        }
        if !(resp.status().is_success() || resp.status().as_u16() == 207) {
            return Err(format!("PROPFIND {path} -> {}", resp.status()));
        }
        // S7：流式累计 + 上限，不信任 Content-Length；超限立即报错。
        use futures_util::StreamExt;
        let mut raw: Vec<u8> = Vec::new();
        let mut received: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            received = received.saturating_add(chunk.len() as u64);
            if body_limit_exceeded(received, Self::PROPFIND_MAX_BYTES) {
                return Err(format!(
                    "PROPFIND {path} 响应超过上限 {} 字节",
                    Self::PROPFIND_MAX_BYTES
                ));
            }
            raw.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&raw).into_owned();
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
        validate_remote_path(path)?;
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
        validate_remote_path(src)?;
        validate_remote_path(dst)?;
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
        validate_remote_path(src)?;
        validate_remote_path(dst)?;
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

/// S7：流式累计字节是否超上限（抽成纯函数便于边界测试；与 GET 的判定口径一致）。
fn body_limit_exceeded(received: u64, max: u64) -> bool {
    received > max
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
    // S7：年份限定 1970..=9999。旧实现用 while y > 1970 逐年递减，超大年份会长时间挂死；
    // 早于 1970 的日期对 mtime 没有意义，一并按非法处理返回 0。
    if !(1970..=9999).contains(&yy) {
        return 0;
    }
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

/// 逐字节解码 %XX：不按字符边界切片，非 ASCII 或残缺的 % 序列都不会 panic。
/// 整段解出的字节不是合法 UTF-8（远端可控）时丢弃该段返回空串，调用方按无效条目跳过。
fn percent_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
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
        // 非 ASCII 与非法 UTF-8 字节序列不 panic：解不出合法 UTF-8 时整段丢弃为空串。
        assert_eq!(percent_decode_path("%中"), "%中");
        assert_eq!(percent_decode_path("%FF"), "");
        assert_eq!(percent_decode_path("%E4"), "");
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
        // S7：年份越界一律 0（旧实现会对巨大年份做逐年递减，可能长时间挂死）。
        assert_eq!(parse_rfc822("Sun, 06 Sep 0000 00:00:00 GMT"), 0);
        assert_eq!(parse_rfc822("Sun, 06 Sep 1969 00:00:00 GMT"), 0);
        assert_eq!(parse_rfc822("Sun, 06 Sep 10000 00:00:00 GMT"), 0);
        assert_eq!(parse_rfc822("Sun, 06 Sep 999999999 00:00:00 GMT"), 0);
        // 边界：9999 年仍可解析（>0），且不因递减循环挂住。
        assert!(parse_rfc822("Sun, 06 Sep 9999 00:00:00 GMT") > 0);
    }

    /// S7：PROPFIND 响应体上限判定的边界（max-1 / max / max+1）。
    #[test]
    fn test_body_limit_exceeded() {
        const MAX: u64 = 16 * 1024 * 1024;
        assert!(!body_limit_exceeded(MAX - 1, MAX));
        assert!(!body_limit_exceeded(MAX, MAX));
        assert!(body_limit_exceeded(MAX + 1, MAX));
        assert!(!body_limit_exceeded(0, MAX));
    }

    /// S7：远端路径段校验：拒绝 . / .. 及其 percent-encoded 变体；普通文件名不误伤。
    #[test]
    fn test_validate_remote_path() {
        // 允许：空/根、普通多段、中文/空格/加号、以点开头的普通文件名。
        for ok in ["", "/", "a/b", "书 名/第一 章.txt", "a+b/c.txt", "..foo", ".hidden", "a/..foo/b"] {
            assert!(validate_remote_path(ok).is_ok(), "应允许：{ok:?}");
        }
        // 拒绝：点段、parent 穿越、percent-encoded 变体、解码后含分隔符。
        for bad in [
            "..",
            ".",
            "a/../b",
            "a/./b",
            "/../x",
            "%2e%2e",
            "a/%2E%2E/b",
            "%2e%2e%2F..",
            "a/%2e%b/../x",
            "a/..%5c/b",
        ] {
            assert!(validate_remote_path(bad).is_err(), "应拒绝：{bad:?}");
        }
        // 错误信息可读（含「非法路径段」）。
        let err = validate_remote_path("a/../b").unwrap_err();
        assert!(err.contains("非法路径段"), "错误信息应可读：{err}");
    }
    /// S2：32 位 >2GiB 拒绝；恰好 2GiB 与 64 位同值都不拒绝（纯函数边界）。
    #[test]
    fn too_large_for_arch_boundaries() {
        const TWO_GIB: u64 = 2 * 1024 * 1024 * 1024;
        assert!(too_large_for_arch(TWO_GIB, 32).is_none(), "恰好 2GiB 允许");
        assert!(too_large_for_arch(TWO_GIB + 1, 32).is_some(), "32 位超 2GiB 必须拒绝");
        assert!(too_large_for_arch(TWO_GIB, 64).is_none());
        assert!(too_large_for_arch(TWO_GIB + 1, 64).is_none(), "64 位保留 8GiB 语义");
        assert!(too_large_for_arch(u64::MAX - 1, 32).is_some());
    }

}

#[cfg(test)]
mod r11_dav_tests {
    use super::*;

    /// P1-7：构建成功返回 Ok 且 base 归一化；客户端字段可见（失败路径需要真实构建错误，见交付说明）。
    #[test]
    fn dav_client_new_returns_result_and_trims_base() {
        let c = DavClient::new("https://example.com/dav/", "user", "pass").unwrap();
        assert_eq!(c.base, "https://example.com/dav");
        assert_eq!(c.user, "user");
        // 非本机公网地址不得放宽证书校验（构造参数不可直接断言，走代码路径保证）。
    }
}
