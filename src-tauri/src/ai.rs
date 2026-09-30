use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::Emitter;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub struct AiConfig {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// azure-openai 专用：部署名（{base}/openai/deployments/{deployment}/...）
    pub deployment: String,
    /// azure-openai 专用：api-version
    pub api_version: String,
}

/// 结构化错误：kind 供前端故障转移判定，message 供展示。
/// 绝不含 apiKey；kind 取值：network / timeout / http_4xx / http_5xx /
/// protocol / model_id / config / unknown。
#[derive(serde::Serialize, Clone, Debug)]
pub struct AiError {
    pub kind: String,
    pub message: String,
}

impl AiError {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self { kind: kind.to_string(), message: message.into() }
    }
    pub fn network(message: impl Into<String>) -> Self {
        Self::new("network", message)
    }
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("timeout", message)
    }
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new("protocol", message)
    }
    pub fn model_id(message: impl Into<String>) -> Self {
        Self::new("model_id", message)
    }
    pub fn config(message: impl Into<String>) -> Self {
        Self::new("config", message)
    }
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        let kind = if (400..500).contains(&status) {
            "http_4xx"
        } else if (500..600).contains(&status) {
            "http_5xx"
        } else {
            "unknown"
        };
        Self::new(kind, message)
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// 去掉文本中 URL 的查询串：reqwest 错误 Display 会带完整 URL，而 Gemini 等历史实现
/// 曾把 api_key 放进 query。改造后不再回显 query，避免任何密钥经错误信息外泄。
fn redact_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '?' {
            // 抹掉 query 直到空白或右括号（reqwest 的 "...for url (http://h/p?x)" 结构）。
            while let Some(&n) = it.peek() {
                if n.is_whitespace() || n == ')' {
                    break;
                }
                it.next();
            }
            out.push_str("?<已隐藏>");
        } else {
            out.push(c);
        }
    }
    out
}

fn classify_req(e: &reqwest::Error) -> AiError {
    let msg = redact_query(&e.to_string());
    if e.is_timeout() {
        AiError::timeout(format!("请求超时: {msg}"))
    } else {
        AiError::network(format!("网络失败: {msg}"))
    }
}

/// 把代理/URL 里的 `user:pass@` 凭据打码，供测试连接展示（不改变可用性判断）。
fn mask_creds(s: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // 可选 scheme + 含冒号的 userinfo + '@'：仅命中真正的凭据段，普通 URL 不受影响。
        regex::Regex::new(r"(?i)([a-z][a-z0-9+.\-]*://)?([^/@\s]+:[^/@\s]+)@").unwrap()
    });
    re.replace_all(s, "${1}***:***@").into_owned()
}

/// 模型ID形状守卫（与前端 isPlainModelId 对齐）：只允许字母数字与 . _ / : -，
/// 拒空/前导/ / :// ；防止把 JSON 片段、?query、-H 请求头、curl 整行塞进 model。
pub fn valid_model_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && !s.starts_with('/')
        && !s.contains("://")
        && s.chars().next().map_or(false, |c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | ':' | '-'))
}

/* ============================ 代理 ============================ */

fn normalize_proxy_url(raw: &str) -> Result<String, String> {
    let mut u = raw.trim().trim_end_matches('/').to_string();
    if u.is_empty() {
        return Err("代理地址不能为空".into());
    }
    if !u.contains("://") {
        u = format!("http://{u}");
    }
    Ok(u)
}

fn proxy_with_bypass(u: &str) -> Result<reqwest::Proxy, String> {
    reqwest::Proxy::all(u)
        .map(|p| p.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1")))
        .map_err(|e| format!("代理地址无效: {u} ({e})"))
}

/// 解析 `ProxyServer`，兼容 `host:port` 与 `http=host:port;https=host:port` 两种格式。
fn parse_proxy_server(s: &str) -> String {
    let s = s.trim();
    if !s.contains('=') {
        return s.to_string();
    }
    let (mut https, mut http, mut other) = (None, None, None);
    for part in s.split(';') {
        if let Some((k, v)) = part.split_once('=') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            match k.as_str() {
                "https" => https = Some(v.to_string()),
                "http" => http = Some(v.to_string()),
                _ => {
                    if other.is_none() {
                        other = Some(v.to_string());
                    }
                }
            }
        }
    }
    https.or(http).or(other).unwrap_or_else(|| s.to_string())
}

#[derive(Clone, Default)]
pub struct WinProxyInfo {
    /// 手动配置的系统代理（已解析为 host:port）
    pub manual: Option<String>,
    /// PAC 自动配置脚本地址（存在则说明系统用 PAC，本程序不解析）
    pub pac: Option<String>,
}

pub fn windows_proxy_info() -> WinProxyInfo {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let mut info = WinProxyInfo::default();
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let key = match hk.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings") {
        Ok(k) => k,
        Err(_) => return info,
    };
    let proxy_type: u32 = key.get_value("ProxyType").unwrap_or(0);
    let auto_url: String = key.get_value("AutoConfigURL").unwrap_or_default();
    if proxy_type == 2 || !auto_url.trim().is_empty() {
        info.pac = Some(auto_url);
    }
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled != 0 {
        if let Ok(server) = key.get_value::<String, _>("ProxyServer") {
            let s = server.trim();
            if !s.is_empty() {
                info.manual = Some(parse_proxy_server(s));
            }
        }
    }
    info
}

#[derive(Clone)]
pub struct ProxyResolution {
    pub url: Option<String>,
    pub source: String,
    pub pac: Option<String>,
}

/// auto：优先环境变量，其次 Windows 注册表；PAC 仅诊断提示。
pub fn resolve_auto_proxy() -> ProxyResolution {
    for k in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim();
            if !v.is_empty() {
                return ProxyResolution {
                    url: Some(v.to_string()),
                    source: format!("环境变量 {k}"),
                    pac: None,
                };
            }
        }
    }
    let info = windows_proxy_info();
    if let Some(addr) = info.manual {
        return ProxyResolution {
            url: Some(addr),
            source: "Windows 系统代理".into(),
            pac: info.pac,
        };
    }
    let source = if info.pac.is_some() {
        "未检测到环境变量，且系统使用 PAC 自动配置脚本（本程序不解析 PAC）".to_string()
    } else {
        "未检测到环境变量或系统代理".to_string()
    };
    ProxyResolution { url: None, source, pac: info.pac }
}

/// 目标主机是否为本机/私有网段（loopback / 10/8 / 172.16/12 / 192.168/16 / 169.254/16）。
/// 仅这些目标允许放宽 TLS 校验，以支持本地自签名服务（如 TTS 网关、局域网 NAS）。
pub fn is_local_or_private_host(host: &str) -> bool {
    use std::net::IpAddr;
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    let h = h.split('%').next().unwrap_or(h); // 去掉 IPv6 zone id（fe80::1%eth0）
    if h.eq_ignore_ascii_case("localhost") || h == "::1" {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) => {
            let seg = v6.segments();
            v6.is_loopback()
                || (seg[0] & 0xfe00) == 0xfc00 // ULA fc00::/7（取首段高 7 位）
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10（取首段高 10 位）
        }
        Err(_) => false,
    }
}

/// 从目标 URL 判断是否本机/私有；无 scheme 时按 http 解析（用户常填 `127.0.0.1:3900`）。
pub fn is_local_or_private_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() {
        return false;
    }
    let with_scheme = if u.contains("://") {
        u.to_string()
    } else {
        format!("http://{u}")
    };
    reqwest::Url::parse(&with_scheme)
        .ok()
        .and_then(|p| p.host_str().map(is_local_or_private_host))
        .unwrap_or(false)
}

/// client 缓存：key = (代理模式, 代理地址, 是否放宽 TLS)。
type ClientCache = Mutex<HashMap<(String, String, bool), reqwest::Client>>;

/// 按「目标是否本机/私有」缓存 client（key 含代理配置，避免跨配置串用）。
/// `danger_accept_invalid_certs` 是 Client 级选项，无法逐请求切换，故分别构建并复用。
fn client_cache() -> &'static ClientCache {
    static CACHE: OnceLock<ClientCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `target_url`：即将访问的目标地址。仅当它是 loopback / 私有网段时才放宽 TLS 校验，
/// 公网目标一律严格校验证书；本地自签名 TTS/NAS 因此仍可用。
pub fn build_client(
    proxy_mode: &str,
    proxy_url: &str,
    target_url: &str,
) -> Result<reqwest::Client, String> {
    let accept_invalid = is_local_or_private_url(target_url);
    let key = (proxy_mode.to_string(), proxy_url.to_string(), accept_invalid);
    if let Ok(cache) = client_cache().lock() {
        if let Some(c) = cache.get(&key) {
            return Ok(c.clone());
        }
    }
    let client = build_client_uncached(proxy_mode, proxy_url, accept_invalid)?;
    if let Ok(mut cache) = client_cache().lock() {
        cache.insert(key, client.clone());
    }
    Ok(client)
}

fn build_client_uncached(
    proxy_mode: &str,
    proxy_url: &str,
    accept_invalid_certs: bool,
) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(20))
        .danger_accept_invalid_certs(accept_invalid_certs);
    match proxy_mode {
        // 彻底直连：显式关闭 reqwest 对 HTTP(S)_PROXY 等环境变量的自动读取
        "direct" => {
            b = b.no_proxy();
        }
        "custom" => {
            let u = normalize_proxy_url(proxy_url)?;
            b = b.proxy(proxy_with_bypass(&u)?);
        }
        // auto（默认）与环境变量/系统代理一致；同时兼容历史遗留的 "system"
        "auto" | "system" | "" => {
            let r = resolve_auto_proxy();
            if let Some(addr) = r.url {
                if let Ok(p) = proxy_with_bypass(&addr) {
                    b = b.proxy(p);
                }
            } else {
                b = b.no_proxy();
            }
        }
        _ => {
            b = b.no_proxy();
        }
    }
    b.build().map_err(|e| e.to_string())
}

/* ============================ 服务商适配 ============================ */

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    OpenAi,
    Ollama,
    Anthropic,
    Gemini,
    Azure,
    /// 阿里云百炼 / DashScope（通义千问 Qwen-TTS 多模态合成端点）
    DashScope,
}

impl Provider {
    pub fn parse(s: &str) -> Provider {
        match s.to_ascii_lowercase().as_str() {
            "ollama" => Provider::Ollama,
            "anthropic" | "claude" => Provider::Anthropic,
            "gemini" | "google" => Provider::Gemini,
            "azure-openai" | "azure" | "azure_openai" => Provider::Azure,
            "dashscope" | "qwen" | "qwen-tts" | "qwen3-tts" | "tongyi" | "bailian" => {
                Provider::DashScope
            }
            _ => Provider::OpenAi,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Provider::OpenAi => "OpenAI 兼容",
            Provider::Ollama => "Ollama",
            Provider::Anthropic => "Anthropic",
            Provider::Gemini => "Gemini",
            Provider::Azure => "Azure OpenAI",
            Provider::DashScope => "通义千问 TTS",
        }
    }
}

fn strip_trailing_slash(s: &str) -> &str {
    s.trim().trim_end_matches('/')
}

fn ensure_v1(base: &str) -> String {
    if base.ends_with("/v1") {
        base.to_string()
    } else {
        format!("{base}/v1")
    }
}

// 出站请求体用结构体声明字段顺序，保证 Object.keys(body) 稳定为约定值，且绝不夹带额外字段。
#[derive(serde::Serialize, Clone)]
struct OllamaChatBody {
    messages: Vec<serde_json::Value>,
    model: String,
    options: serde_json::Value,
    stream: bool,
}

#[derive(serde::Serialize, Clone)]
struct OpenAiChatBody {
    model: String,
    messages: Vec<serde_json::Value>,
    stream: bool,
    temperature: f64,
}

#[derive(serde::Serialize, Clone)]
struct AnthropicChatBody {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<serde_json::Value>,
    stream: bool,
}

#[derive(serde::Serialize, Clone)]
struct GeminiChatBody {
    contents: Vec<serde_json::Value>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<serde_json::Value>,
}

#[derive(serde::Serialize, Clone)]
#[serde(untagged)]
enum ChatBody {
    Ollama(OllamaChatBody),
    OpenAi(OpenAiChatBody),
    Anthropic(AnthropicChatBody),
    Gemini(GeminiChatBody),
}

struct Prepared {
    url: String,
    headers: Vec<(String, String)>,
    body: ChatBody,
}

/// 判定整行是否为「budget/token 预算」注入行（v5 收紧版）。
/// 只在**整行**呈"字段名/占位符形态"时命中；正文自然语句一律保留。
fn is_budget_line(line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() {
        return false;
    }
    let low = l.to_ascii_lowercase();
    // 分词：同时按 `_`、`:`、`=`、空白切分（关键：token_budget 需能被切开）。
    let words: Vec<&str> = low
        .split(|c: char| c.is_whitespace() || matches!(c, '_' | ':' | '=' | ',' | ';' | '\t' | '|'))
        .filter(|w| !w.is_empty())
        .collect();
    let all_noise = |s: &str| -> bool {
        // 仅由数字或 o/O（形近 0）组成 → 计数字噪声
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == 'o' || c == 'O')
    };
    let is_kw = |w: &str| w == "budget" || w == "token" || w == "tokens";

    // 逐词判断：任一词以 token_budget / budget / token / tokens 开头，
    // 且其后紧接的剩余部分全是数字/形近 0 → 元数据（token_budget200000、budget200000 等）。
    // 注意 `_` 已被切分，故 `token_budget200000` 会变成 `token` + `budget200000` 两个词。
    for w in &words {
        for base in ["token_budget", "tokens_budget", "budget", "tokens", "token"] {
            if let Some(rest) = w.strip_prefix(base) {
                if all_noise(rest) {
                    return true;
                }
            }
        }
    }

    // 独立字段名行：整行由「关键词（可两词 token budget）+ 计数字/纯数字」构成。
    // 覆盖 `budget 200000` / `tokens: 20oooo` / `token budget:999` 等。
    {
        let mut rest = words.as_slice();
        let mut consumed = 0usize;
        while consumed < 2 && !rest.is_empty() && is_kw(rest[0]) {
            rest = &rest[1..];
            consumed += 1;
        }
        if consumed >= 1 && !rest.is_empty() && rest.iter().all(|w| all_noise(w)) {
            return true;
        }
    }

    // `budget:` / `budget=` 前缀，且其后非英文字母（数字/符号/空）→ 元数据
    for pfx in ["budget:", "budget =", "budget="] {
        if let Some(rest) = low.strip_prefix(pfx) {
            let rest = rest.trim();
            if rest.is_empty() || rest.chars().all(|c| !c.is_ascii_alphabetic()) {
                return true;
            }
        }
    }

    // 标签包裹：<...> / [...] 内部含独立 token/budget 词
    let tag = (l.starts_with('<') && l.ends_with('>'))
        || (l.starts_with('[') && l.ends_with(']'));
    if tag {
        let inner = &low[1..low.len().saturating_sub(1)];
        let inner_words: Vec<&str> = inner
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty())
            .collect();
        if inner_words.iter().any(|w| is_kw(w) || w.starts_with("token_budget")) {
            return true;
        }
    }

    false
}

/// 清洗 messages（v5）：
///   - 不误杀正文（分词版判定）；
///   - 相邻完全重复的消息并成 1 条；
///   - 清洗后为空的消息给非空兜底（保证请求仍能发出）。
pub fn sanitize_messages(messages: &[(String, String)]) -> Vec<(String, String)> {
    const FALLBACK: &str = ".";
    let mut cleaned: Vec<(String, String)> = messages
        .iter()
        .map(|(role, content)| {
            let kept: Vec<&str> = content
                .split('\n')
                .filter(|line| !is_budget_line(line))
                .collect();
            let joined = kept.join("\n");
            let joined = joined.trim_matches('\n').to_string();
            let body = if joined.trim().is_empty() {
                FALLBACK.to_string()
            } else {
                joined
            };
            (role.clone(), body)
        })
        .collect();

    let mut deduped: Vec<(String, String)> = Vec::with_capacity(cleaned.len());
    for item in cleaned.drain(..) {
        if deduped
            .last()
            .map(|(r, c)| *r == item.0 && *c == item.1)
            .unwrap_or(false)
        {
            continue;
        }
        deduped.push(item);
    }
    deduped
}

/// 溯源（独立宽松版）：**不复用 is_budget_line**，用子串宽松匹配捕捉任何漏删残留。
/// `FLOWREAD_AI_TRACE=1` 时逐条打印 idx/role/len；命中打印 head= 残留原文。
pub fn trace_budget(messages: &[(String, String)]) {
    if std::env::var("FLOWREAD_AI_TRACE").as_deref() != Ok("1") {
        return;
    }
    let total: usize = messages.iter().map(|(_, c)| c.chars().count()).sum();
    eprintln!("[ai-trace] total={} messages={}", total, messages.len());
    let mut hits = 0usize;
    for (i, (role, content)) in messages.iter().enumerate() {
        let low = content.to_ascii_lowercase();
        let hit = low.contains("budget") || low.contains("token_budget");
        if hit {
            hits += 1;
            let head: String = content.chars().take(120).collect();
            eprintln!("[ai-trace] idx={} role={} len={} HIT head={:?}", i, role, content.chars().count(), head);
        } else {
            eprintln!("[ai-trace] idx={} role={} len={}", i, role, content.chars().count());
        }
    }
    eprintln!("[ai-trace] hits={} (0 = 干净)", hits);
}

fn msgs_json(messages: &[(String, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|(role, content)| json!({"role": role, "content": content}))
        .collect()
}

fn anthropic_messages_json(messages: &[(String, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|(role, content)| {
            let role = if role == "assistant" { "assistant" } else { "user" };
            json!({"role": role, "content": content})
        })
        .collect()
}

fn gemini_contents_json(messages: &[(String, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|(role, content)| {
            let role = if role == "assistant" { "model" } else { "user" };
            json!({"role": role, "parts": [{"text": content}]})
        })
        .collect()
}

/// 统一构造请求：URL / 认证头 / 固定字段顺序的 body。
fn build_request(
    p: Provider,
    cfg: &AiConfig,
    system: &str,
    messages: &[(String, String)],
    stream: bool,
) -> Result<Prepared, AiError> {
    let base = strip_trailing_slash(&cfg.base_url);
    if base.is_empty() {
        return Err(AiError::config("未填写 API 地址"));
    }
    if !valid_model_id(&cfg.model) {
        return Err(AiError::model_id(
            "模型ID含非法字符（JSON/URL/query/请求头/curl 不能作为模型ID），请求已拦截。",
        ));
    }
    // 出站前清洗：摘除客户端/网关/代理拼接的 budget/token 元数据行（含相邻去重）。
    let cleaned: Vec<(String, String)> = sanitize_messages(messages);
    trace_budget(&cleaned);
    let messages: &[(String, String)] = &cleaned;
    match p {
        Provider::Ollama => Ok(Prepared {
            url: format!("{base}/api/chat"),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: ChatBody::Ollama(OllamaChatBody {
                messages: msgs_json(messages),
                model: cfg.model.clone(),
                options: json!({"num_ctx": 8192}),
                stream,
            }),
        }),
        Provider::OpenAi | Provider::DashScope => Ok(Prepared {
            url: format!("{}/chat/completions", ensure_v1(base)),
            headers: bearer_headers(&cfg.api_key),
            body: ChatBody::OpenAi(OpenAiChatBody {
                model: cfg.model.clone(),
                messages: msgs_json(messages),
                stream,
                temperature: 0.7,
            }),
        }),
        Provider::Azure => {
            if cfg.deployment.trim().is_empty() {
                return Err(AiError::config("Azure OpenAI 需要填写部署名（deployment）"));
            }
            if !valid_model_id(cfg.deployment.trim()) {
                return Err(AiError::model_id("Azure 部署名含非法字符，请求已拦截。"));
            }
            let api_version = if cfg.api_version.trim().is_empty() {
                "2024-06-01"
            } else {
                cfg.api_version.trim()
            };
            Ok(Prepared {
                url: format!(
                    "{base}/openai/deployments/{}/chat/completions?api-version={api_version}",
                    cfg.deployment.trim()
                ),
                headers: vec![
                    ("api-key".into(), cfg.api_key.clone()),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body: ChatBody::OpenAi(OpenAiChatBody {
                    model: cfg.model.clone(),
                    messages: msgs_json(messages),
                    stream,
                    temperature: 0.7,
                }),
            })
        }
        Provider::Anthropic => {
            let url = if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            };
            Ok(Prepared {
                url,
                headers: vec![
                    ("x-api-key".into(), cfg.api_key.clone()),
                    ("anthropic-version".into(), "2023-06-01".into()),
                    ("Content-Type".into(), "application/json".into()),
                ],
                body: ChatBody::Anthropic(AnthropicChatBody {
                    model: cfg.model.clone(),
                    max_tokens: 4096,
                    system: system.to_string(),
                    messages: anthropic_messages_json(messages),
                    stream,
                }),
            })
        }
        Provider::Gemini => {
            let model = cfg.model.strip_prefix("models/").unwrap_or(&cfg.model);
            let method = if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            };
            // 不再把 apiKey 放进 URL query（会经错误信息/日志泄漏），改用官方请求头 x-goog-api-key。
            let url = format!("{base}/v1beta/models/{model}:{method}");
            let sys = if system.trim().is_empty() {
                None
            } else {
                Some(json!({"parts": [{"text": system}]}))
            };
            let mut headers = vec![("Content-Type".into(), "application/json".into())];
            if !cfg.api_key.is_empty() {
                headers.push(("x-goog-api-key".into(), cfg.api_key.clone()));
            }
            Ok(Prepared {
                url,
                headers,
                body: ChatBody::Gemini(GeminiChatBody {
                    contents: gemini_contents_json(messages),
                    system_instruction: sys,
                }),
            })
        }
    }
}

fn bearer_headers(api_key: &str) -> Vec<(String, String)> {
    let mut h = vec![("Content-Type".into(), "application/json".into())];
    if !api_key.is_empty() {
        h.push(("Authorization".into(), format!("Bearer {api_key}")));
    }
    h
}

/// 流式分片取正文
fn stream_delta(p: Provider, v: &serde_json::Value) -> Option<String> {
    match p {
        Provider::Ollama => v.pointer("/message/content").and_then(|c| c.as_str()).map(str::to_string),
        Provider::OpenAi | Provider::Azure => {
            v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()).map(str::to_string)
        }
        Provider::DashScope => {
            v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()).map(str::to_string)
        }
        Provider::Anthropic => {
            if v.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
                v.pointer("/delta/text").and_then(|c| c.as_str()).map(str::to_string)
            } else {
                None
            }
        }
        Provider::Gemini => gemini_parts_text(v),
    }
}

/// 非流式响应取正文
fn full_content(p: Provider, v: &serde_json::Value) -> Option<String> {
    match p {
        Provider::Ollama => v
            .pointer("/message/content")
            .or_else(|| v.pointer("/response"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Provider::OpenAi | Provider::Azure => v
            .pointer("/choices/0/message/content")
            .or_else(|| v.pointer("/choices/0/delta/content"))
            .or_else(|| v.pointer("/choices/0/text"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Provider::DashScope => v
            .pointer("/choices/0/message/content")
            .or_else(|| v.pointer("/choices/0/delta/content"))
            .or_else(|| v.pointer("/choices/0/text"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Provider::Anthropic => v.pointer("/content").and_then(|c| c.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<String>()
        }),
        Provider::Gemini => gemini_parts_text(v),
    }
}

fn gemini_parts_text(v: &serde_json::Value) -> Option<String> {
    let arr = v.pointer("/candidates/0/content/parts")?.as_array()?;
    let s: String = arr
        .iter()
        .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
        .collect();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 从错误响应体提取可读信息（各 provider 结构不同）。
fn error_message_from_body(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.pointer("/error/status"))
                .or_else(|| v.get("error"))
                .and_then(|m| {
                    m.as_str()
                        .map(str::to_string)
                        .or_else(|| Some(m.to_string()))
                })
        })
        .unwrap_or_else(|| text.chars().take(300).collect())
}

/* ============================ 连接测试 ============================ */

pub async fn test_connection(
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
    deployment: String,
    api_version: String,
    proxy_mode: String,
    proxy_url: String,
) -> Result<String, String> {
    let p = Provider::parse(&provider);
    let cfg = AiConfig {
        provider: provider.clone(),
        base_url,
        api_key,
        model,
        deployment,
        api_version,
    };
    let prepared = build_request(
        p,
        &cfg,
        "",
        &[("user".to_string(), "回复 OK".to_string())],
        false,
    )
    .map_err(|e| format!("✗ 请求配置错误：{}", e.message))?;

    let mode = if proxy_mode.is_empty() { "auto" } else { proxy_mode.as_str() };
    let target = prepared.url.as_str();
    let mut candidates: Vec<(String, reqwest::Client)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    match mode {
        "custom" => {
            let shown = mask_creds(&normalize_proxy_url(&proxy_url).unwrap_or_else(|_| proxy_url.clone()));
            match build_client("custom", &proxy_url, target) {
                Ok(c) => candidates.push((format!("自定义代理 {shown}"), c)),
                Err(e) => notes.push(format!("自定义代理不可用：{}", mask_creds(&e))),
            }
        }
        "direct" => match build_client("direct", "", target) {
            Ok(c) => candidates.push(("直连".into(), c)),
            Err(e) => notes.push(format!("直连客户端构建失败：{}", mask_creds(&e))),
        },
        _ => {
            let r = resolve_auto_proxy();
            match &r.url {
                Some(addr) => match build_client("custom", addr, target) {
                    Ok(c) => candidates.push((format!("自动代理（{}）", r.source), c)),
                    Err(e) => notes.push(format!("自动代理（{}）不可用：{}", r.source, mask_creds(&e))),
                },
                None => {
                    notes.push(r.source.clone());
                    if let Ok(c) = build_client("direct", "", target) {
                        candidates.push(("直连（自动模式未检测到代理）".into(), c));
                    }
                }
            }
            if let Some(pac) = &r.pac {
                notes.push(format!(
                    "检测到系统 PAC 自动配置脚本（{pac}），本程序不解析 PAC，自动模式不会使用该代理"
                ));
            }
        }
    }
    // 非直连模式再补一条直连回退，便于诊断
    if mode != "direct" {
        if let Ok(c) = build_client("direct", "", target) {
            candidates.push(("直连（回退）".into(), c));
        }
    }

    let mut errs: Vec<String> = Vec::new();
    for (name, http) in &candidates {
        let mut req = http.clone().post(&prepared.url).timeout(Duration::from_secs(25)).json(&prepared.body);
        for (k, v) in &prepared.headers {
            if k.eq_ignore_ascii_case("content-type") {
                continue;
            }
            req = req.header(k.as_str(), v.as_str());
        }
        let outcome = async {
            let resp = req.send().await.map_err(|e| classify_req(&e).message)?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                let brief = error_message_from_body(&text);
                return Err(format!("HTTP {status}: {brief}"));
            }
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let reply = full_content(p, &parsed).unwrap_or_default();
            let reply = reply.trim();
            Ok::<String, String>(if reply.is_empty() {
                format!("✓ 连接成功（{name}｜{}）", p.label())
            } else {
                format!("✓ 连接成功（{name}｜{}），模型回复：{reply}", p.label())
            })
        }
        .await;
        match outcome {
            Ok(msg) => {
                if notes.is_empty() {
                    return Ok(msg);
                }
                return Ok(format!("{msg}；诊断：{}", notes.join("；")));
            }
            Err(e) => errs.push(format!("【{name}】{e}")),
        }
    }
    let mut all: Vec<String> = errs;
    all.extend(notes);
    Err(format!("✗ 全部尝试失败 → {}", mask_creds(&all.join(" ； "))))
}

/* ============================ 流式聊天 ============================ */

pub async fn stream_chat(
    app: tauri::AppHandle,
    http: reqwest::Client,
    id: String,
    cfg: AiConfig,
    system: String,
    messages: Vec<(String, String)>,
) -> Result<(), AiError> {
    // 溯源（清洗前）：显示进入本层的原始 messages 是否已含 budget 残留。
    // FLOWREAD_AI_TRACE=1 时，先打印一块 [ai-trace]（pre），再由 build_request 打印清洗后的（post）。
    // 若 pre hits>0 而 post hits=0 → 注入来自上游（客户端配置/网关/代理），清洗已摘除。
    trace_budget(&messages);
    let p = Provider::parse(&cfg.provider);
    let prepared = build_request(p, &cfg, &system, &messages, true)?;

    let emit_err = |app: &tauri::AppHandle, id: &str, e: &AiError| {
        let _ = app.emit(
            &format!("ai-token:{id}"),
            serde_json::json!({"error": e.message, "kind": e.kind, "done": true}),
        );
    };
    let emit_delta = |app: &tauri::AppHandle, id: &str, content: &str, done: bool| {
        let _ = app.emit(
            &format!("ai-token:{id}"),
            serde_json::json!({"delta": content, "done": done}),
        );
    };

    let mut req = http
        .post(&prepared.url)
        .timeout(Duration::from_secs(300))
        .json(&prepared.body);
    for (k, v) in &prepared.headers {
        if k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        match reqwest::header::HeaderValue::from_str(v) {
            Ok(hv) => {
                req = req.header(k.as_str(), hv);
            }
            Err(_) => return Err(AiError::config(format!("无效的请求头值：{k}"))),
        }
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let err = classify_req(&e);
            emit_err(&app, &id, &err);
            return Err(err);
        }
    };
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let err = AiError::http(status, format!("HTTP {status}: {}", error_message_from_body(&text)));
        emit_err(&app, &id, &err);
        return Err(err);
    }

    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let is_ollama = p == Provider::Ollama;
    let is_sse = ct.contains("text/event-stream") || (is_ollama && ct.contains("ndjson"));

    if !is_sse {
        let text = resp.text().await.map_err(|e| classify_req(&e))?;
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if let Some(content) = full_content(p, &parsed) {
            if !content.is_empty() {
                emit_delta(&app, &id, &content, true);
                return Ok(());
            }
        }
        let brief: String = text.chars().take(300).collect();
        let err = AiError::protocol(format!(
            "响应协议不匹配：端点返回非流式响应（Content-Type: {ct}），且未包含可识别的正文字段。响应片段：{brief}"
        ));
        emit_err(&app, &id, &err);
        return Err(err);
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut got_any = false;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let err = classify_req(&e);
                emit_err(&app, &id, &err);
                return Err(err);
            }
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            if is_ollama {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(content) = stream_delta(Provider::Ollama, &v) {
                        got_any = true;
                        let done = v.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
                        emit_delta(&app, &id, &content, done);
                        if done {
                            return Ok(());
                        }
                    }
                }
                continue;
            }
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                if !got_any {
                    let err = AiError::protocol(format!(
                        "响应协议不匹配：流结束前未找到正文字段。响应片段：{}",
                        buf.chars().take(200).collect::<String>()
                    ));
                    emit_err(&app, &id, &err);
                    return Err(err);
                }
                emit_delta(&app, &id, "", true);
                return Ok(());
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(content) = stream_delta(p, &v) {
                    if !content.is_empty() {
                        got_any = true;
                    }
                    emit_delta(&app, &id, &content, false);
                }
            }
        }
    }

    if !got_any {
        let ct2 = if ct.is_empty() { "未知" } else { &ct };
        let brief: String = buf.chars().take(300).collect();
        let parsed: serde_json::Value = serde_json::from_str(buf.trim()).unwrap_or_default();
        if let Some(content) = full_content(p, &parsed) {
            if !content.is_empty() {
                emit_delta(&app, &id, &content, true);
                return Ok(());
            }
        }
        let err = AiError::protocol(format!(
            "响应协议不匹配：流式响应中未找到该协议的正文字段，拒绝空白回复（Content-Type: {ct2}）。响应片段：{brief}"
        ));
        emit_err(&app, &id, &err);
        return Err(err);
    }
    emit_delta(&app, &id, "", true);
    Ok(())
}

/* ============================ 在线语音合成（TTS） ============================ */

/// 发起合成请求并按音频字节返回（非 2xx 转结构化错误）。
async fn post_audio(req: reqwest::RequestBuilder) -> Result<Vec<u8>, AiError> {
    let resp = req.send().await.map_err(|e| classify_req(&e))?;
    let status = resp.status();
    if !status.is_success() {
        let msg = resp.text().await.unwrap_or_default();
        let brief: String = msg.chars().take(300).collect();
        return Err(AiError::http(status.as_u16(), brief));
    }
    let bytes = resp.bytes().await.map_err(|e| classify_req(&e))?;
    if bytes.is_empty() {
        return Err(AiError::protocol("语音接口返回空音频"));
    }
    Ok(bytes.to_vec())
}

#[derive(serde::Serialize)]
struct TtsBody {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

/// 由 DashScope 的 base_url 推导出千问 TTS 多模态合成端点。
/// 兼容用户填写 `https://dashscope.aliyuncs.com`、`.../api/v1`
/// 或完整的 `.../generation` 三种写法。
fn dashscope_tts_url(base: &str) -> String {
    let b = strip_trailing_slash(base);
    if b.ends_with("/generation") {
        b.to_string()
    } else if b.ends_with("/api/v1") {
        format!("{b}/services/aigc/multimodal-generation/generation")
    } else {
        format!("{b}/api/v1/services/aigc/multimodal-generation/generation")
    }
}

/// 调用阿里云百炼 / DashScope 的通义千问 Qwen-TTS 合成接口。
/// 该端点返回 JSON（`output.audio.url` 或 base64 `output.audio.data`），
/// 与 OpenAI `/audio/speech` 直接回传音频字节不同，需在此解析后二次下载。
async fn synthesize_dashscope(
    http: reqwest::Client,
    cfg: &AiConfig,
    base: &str,
    voice: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if cfg.api_key.trim().is_empty() {
        return Err(AiError::config("通义千问语音需要填写 DashScope API Key"));
    }
    let model = if cfg.model.trim().is_empty() {
        "qwen3-tts-flash"
    } else {
        cfg.model.trim()
    };
    let voice = if voice.trim().is_empty() { "Cherry" } else { voice.trim() };
    // language_type：复用 api_version 字段承载（前端可留空），默认 Auto（自动识别语种）。
    let lang = if cfg.api_version.trim().is_empty() {
        "Auto"
    } else {
        cfg.api_version.trim()
    };
    let url = dashscope_tts_url(base);
    let body = json!({
        "model": model,
        "input": { "text": text, "voice": voice, "language_type": lang },
    });
    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {}", cfg.api_key.trim()))
        .json(&body)
        .send()
        .await
        .map_err(|e| classify_req(&e))?;
    let status = resp.status();
    if !status.is_success() {
        let msg = resp.text().await.unwrap_or_default();
        let brief: String = msg.chars().take(300).collect();
        return Err(AiError::http(status.as_u16(), brief));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| classify_req(&e))?;
    // DashScope 可能以 HTTP 200 携带业务错误码返回。
    let code = v.get("code").and_then(|x| x.as_str()).unwrap_or("");
    if !code.is_empty() {
        let m = v.get("message").and_then(|x| x.as_str()).unwrap_or("");
        return Err(AiError::protocol(format!("DashScope 返回错误 {code}：{m}")));
    }
    let audio = v.get("output").and_then(|o| o.get("audio"));
    // 优先取内联 base64；否则回落到二次下载 url。
    if let Some(data) = audio.and_then(|a| a.get("data")).and_then(|x| x.as_str()) {
        if !data.is_empty() {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| AiError::protocol(format!("音频 Base64 解码失败：{e}")))?;
            if !bytes.is_empty() {
                return Ok(bytes);
            }
        }
    }
    if let Some(audio_url) = audio.and_then(|a| a.get("url")).and_then(|x| x.as_str()) {
        if !audio_url.is_empty() {
            let ar = http.get(audio_url).send().await.map_err(|e| classify_req(&e))?;
            if !ar.status().is_success() {
                return Err(AiError::http(ar.status().as_u16(), "下载合成音频失败"));
            }
            let bytes = ar.bytes().await.map_err(|e| classify_req(&e))?;
            if bytes.is_empty() {
                return Err(AiError::protocol("下载的合成音频为空"));
            }
            return Ok(bytes.to_vec());
        }
    }
    Err(AiError::protocol("DashScope 响应中未找到音频数据"))
}

/* ------------------------ 本地 Breeze-TTS-2 网关 ------------------------ */

/// 判定是否走本地 Breeze-TTS-2 HTTP 网关（自建协议，非 OpenAI 兼容）。
pub fn is_breeze_provider(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "breeze" | "breeze-tts" | "breeze-tts-2" | "breeze_tts" | "breeze_tts_2"
    )
}

/// 归一化 Breeze 网关根地址：容忍用户填写 `.../v1`、`.../api`、
/// `.../api/voices`、`.../api/tts/clone` 等任意常见后缀。
pub fn breeze_root(base: &str) -> String {
    let mut b = strip_trailing_slash(base).to_string();
    loop {
        let stripped: Option<String> = ["/api/tts/clone", "/api/voices", "/api", "/v1"]
            .iter()
            .find_map(|suf| b.strip_suffix(suf).map(|s| s.trim_end_matches('/').to_string()));
        match stripped {
            Some(s) if !s.is_empty() => b = s,
            _ => break,
        }
    }
    b
}

/// 由网关根地址推导克隆合成端点。
pub fn breeze_clone_url(base: &str) -> String {
    format!("{}/api/tts/clone", breeze_root(base))
}

/// 由网关根地址推导音色列表端点。
pub fn breeze_voices_url(base: &str) -> String {
    format!("{}/api/voices", breeze_root(base))
}

/// Breeze 网关标准监听端口（app.py 启动 uvicorn 的默认端口）。
/// 上游 C++ 推理服务 audiocpp_server 监听 8080，仅供网关内部调用，不提供
/// /api/voices、/api/tts/clone 等对外接口；用户常误把 8080 当成网关地址。
const BREEZE_GATEWAY_PORT: &str = "7863";

/// 把根地址的端口替换为网关标准端口 7863；已是 7863 或无法解析则返回 None。
/// 仅做纯字符串解析（scheme://authority[/...]，authority=host[:port]），
/// 不引入 url 依赖；IPv6 字面量（含 `[`）不处理。
fn swap_to_gateway_port(root: &str) -> Option<String> {
    let (scheme, rest) = root.split_once("://")?;
    // authority 到首个 '/' 为止（breeze_root 已去尾部路径，通常无 '/'）。
    let (authority, tail) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.contains('[') {
        return None; // IPv6，保守不处理
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p),
        None => (authority, ""), // 无端口：http 默认 80，仍追加 7863 候选
    };
    if host.is_empty() || port == BREEZE_GATEWAY_PORT {
        return None;
    }
    Some(format!("{scheme}://{host}:{BREEZE_GATEWAY_PORT}{tail}"))
}

/// 返回按优先级排列的候选网关根地址：先用用户配置的地址，若其端口不是网关
/// 标准端口，再追加「同主机 + 7863」作为回退（修正误填上游 8080 端口等情况）。
pub fn breeze_root_candidates(base: &str) -> Vec<String> {
    let root = breeze_root(base);
    let mut out = vec![root.clone()];
    if let Some(alt) = swap_to_gateway_port(&root) {
        if !out.contains(&alt) {
            out.push(alt);
        }
    }
    out
}

/// Breeze 网关为本机 / 局域网服务，请求一律直连（绕过系统代理）。
/// 否则 192.168.x.x 等局域网地址会被系统代理拦截而连不通。
/// `base` 为网关地址，用于判定是否放宽 TLS 校验（仅本机/私有网段）。
pub fn breeze_client(base: &str) -> Result<reqwest::Client, AiError> {
    build_client("direct", "", base).map_err(AiError::config)
}

/// 音色参考音频缓存：`{base}\x1f{voice_id}` -> (voice_ref_b64, reference_text)。
/// 整章逐句合成时避免重复拉取同一音色的参考音频。
fn breeze_voice_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, (String, String)>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (String, String)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 按正文判定合成语种：含汉字取 Chinese，否则 English（网关仅支持这两种）。
fn detect_tts_language(text: &str) -> &'static str {
    if text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        "Chinese"
    } else {
        "English"
    }
}

/// 读取指定音色的参考音频（Base64）与参考文本；voice 为空时取列表首个音色。
/// `root` 为已归一化（无 /api、/v1 等后缀）的网关根地址。
/// 结果写入进程内缓存，供后续逐句合成复用。
async fn fetch_breeze_voice_at(
    http: &reqwest::Client,
    root: &str,
    voice: &str,
) -> Result<(String, String), AiError> {
    let key = format!("{}\u{1f}{}", root, voice.trim());
    if let Ok(cache) = breeze_voice_cache().lock() {
        if let Some(hit) = cache.get(&key) {
            return Ok(hit.clone());
        }
    }
    let voices_url = format!("{root}/api/voices");
    let resp = http
        .get(&voices_url)
        .send()
        .await
        .map_err(|e| classify_req(&e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let msg = resp.text().await.unwrap_or_default();
        let brief: String = msg.chars().take(300).collect();
        return Err(AiError::http(status.as_u16(), brief));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| classify_req(&e))?;
    let empty: Vec<serde_json::Value> = Vec::new();
    let arr = v
        .get("voices")
        .and_then(|x| x.as_array())
        .or_else(|| v.get("data").and_then(|x| x.as_array()))
        .unwrap_or(&empty);
    let want = voice.trim();
    let picked = if want.is_empty() {
        arr.first()
    } else {
        arr.iter()
            .find(|it| {
                it.get("id")
                    .and_then(|x| x.as_str())
                    .map(|s| s == want)
                    .unwrap_or(false)
            })
            .or_else(|| arr.first())
    };
    let Some(item) = picked else {
        return Err(AiError::config("该服务未保存音色，请先在服务端创建克隆音色"));
    };
    let id = item
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if id.is_empty() {
        return Err(AiError::protocol("音色缺少 id，无法合成"));
    }
    let reference_text = item
        .get("reference_text")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let audio_url = format!("{root}/api/voices/{id}/audio");
    let aref = http
        .get(&audio_url)
        .send()
        .await
        .map_err(|e| classify_req(&e))?;
    if !aref.status().is_success() {
        return Err(AiError::http(aref.status().as_u16(), "读取音色参考音频失败"));
    }
    let bytes = aref.bytes().await.map_err(|e| classify_req(&e))?;
    if bytes.is_empty() {
        return Err(AiError::protocol("音色参考音频为空"));
    }
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let pair = (b64, reference_text);
    if let Ok(mut cache) = breeze_voice_cache().lock() {
        cache.insert(key, pair.clone());
    }
    Ok(pair)
}

/// 判定模型名是否属于 Breeze 系列（用于决定是否下发 model 字段）。
/// 网关仅内置一个模型，填其它协议遗留的模型名会导致 500，故非 Breeze 模型一律省略。
fn is_breeze_model(model: &str) -> bool {
    matches!(
        model.trim().to_ascii_lowercase().as_str(),
        "" | "breeze" | "breeze-tts" | "breeze-tts-2" | "breeze_tts" | "breeze_tts_2"
            | "q8" | "q8_0" | "q4" | "q4_k" | "base" | "instruct"
    )
}

/// 调用本地 Breeze-TTS-2 网关的克隆合成接口，返回音频字节（WAV）。
/// 依次尝试候选网关根地址（配置地址 + 7863 回退），首个能取到音色的根即用于克隆。
async fn synthesize_breeze(
    http: reqwest::Client,
    cfg: &AiConfig,
    base: &str,
    voice: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    let candidates = breeze_root_candidates(base);
    let mut last_err: Option<AiError> = None;
    for root in &candidates {
        let (voice_ref_b64, reference_text) =
            match fetch_breeze_voice_at(&http, root, voice).await {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
        let mut body = json!({
            "model": "breeze-tts-2",
            "text": text,
            "voice_ref_b64": voice_ref_b64,
            "reference_text": reference_text,
            "language": detect_tts_language(text),
            "do_split": true,
        });
        // 省略非 Breeze 模型名，交由网关使用其内置模型，避免 500。
        if !is_breeze_model(cfg.model.trim()) {
            body.as_object_mut().map(|o| o.remove("model"));
        }
        let clone_url = format!("{root}/api/tts/clone");
        let resp = match http.post(&clone_url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(classify_req(&e));
                continue;
            }
        };
        let status = resp.status();
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !status.is_success() {
            let msg = resp.text().await.unwrap_or_default();
            let brief: String = msg.chars().take(300).collect();
            last_err = Some(AiError::http(status.as_u16(), brief));
            continue;
        }
        let bytes = resp.bytes().await.map_err(|e| classify_req(&e))?;
        if ctype.contains("application/json") {
            // 兼容以 JSON 携带 Base64 音频的实现。
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let data = v
                    .get("audio")
                    .and_then(|a| a.as_str().or_else(|| a.get("data").and_then(|d| d.as_str())));
                if let Some(data) = data {
                    if !data.is_empty() {
                        use base64::Engine as _;
                        let raw = base64::engine::general_purpose::STANDARD
                            .decode(data)
                            .map_err(|e| AiError::protocol(format!("音频 Base64 解码失败：{e}")))?;
                        if !raw.is_empty() {
                            return Ok(raw);
                        }
                    }
                }
            }
            last_err = Some(AiError::protocol("网关返回 JSON 但未包含音频数据"));
            continue;
        }
        if bytes.is_empty() {
            last_err = Some(AiError::protocol("语音接口返回空音频"));
            continue;
        }
        return Ok(bytes.to_vec());
    }
    Err(last_err.unwrap_or_else(|| AiError::config("未配置 Breeze 网关地址")))
}

/// 一次在线语音合成的全部参数（按 provider 显式分派到具体协议实现）。
pub struct TtsParams<'a> {
    pub provider: &'a str,
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub secret_key: &'a str,
    pub app_id: &'a str,
    pub cluster: &'a str,
    pub model: &'a str,
    pub voice: &'a str,
    pub format: &'a str,
    /// 正文语言标签（config.tts.lang），仅 GPT-SoVITS 等协议使用。
    pub lang: &'a str,
}

/// 火山引擎固定公网端点（忽略用户 base_url）。
const VOLCANO_TTS_URL: &str = "https://openspeech.bytedance.com/api/v1/tts";
/// 百度语音固定公网端点（忽略用户 base_url）：取 token 与短文本合成分属两个公网 host。
const BAIDU_TOKEN_URL: &str = "https://aip.baidubce.com/oauth/2.0/token";
const BAIDU_TTS_URL: &str = "https://tsn.baidu.com/text2audio";

/// 按用户显式选择的接口协议分派合成；不做事先的地址探测。
/// 协议：openai / azure / dashscope / minimax / fish / gptsovits / breeze /
/// volcano / baidu / edge，未知一律按 OpenAI 兼容处理。
///
/// `proxy_mode`/`proxy_url` 用于给「固定公网端点」的 provider 按其**真实请求 URL**
/// 重新构建 client：这些 provider 忽略用户 base_url，若复用按 base_url 构建的 client，
/// 用户把 base_url 误填成本地/私网地址时会错误地对公网端点放宽证书校验。
pub async fn synthesize_tts(
    http: reqwest::Client,
    proxy_mode: &str,
    proxy_url: &str,
    p: &TtsParams<'_>,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    let base = strip_trailing_slash(p.base_url);
    match p.provider.trim().to_ascii_lowercase().as_str() {
        "azure" => synth_azure(&http, p, base, text).await,
        "dashscope" => synth_dashscope(&http, p, base, text).await,
        "minimax" => synth_minimax(&http, p, base, text).await,
        "fish" => synth_fish(&http, p, base, text).await,
        "gptsovits" => synth_gptsovits(&http, p, base, text).await,
        "breeze" => synth_breeze_dispatch(&http, p, base, text).await,
        "volcano" => {
            let c = build_client(proxy_mode, proxy_url, VOLCANO_TTS_URL).unwrap_or_else(|_| http.clone());
            synth_volcano(&c, p, text).await
        }
        "baidu" => {
            let c = build_client(proxy_mode, proxy_url, BAIDU_TTS_URL).unwrap_or_else(|_| http.clone());
            synth_baidu(&c, p, text).await
        }
        // Edge 走 wss（tokio-tungstenite），不使用 reqwest client。
        "edge" => synth_edge(p, text).await,
        // openai / voicestudio 均为 OpenAI 兼容（VoiceStudio 本地 /v1/audio/speech）。
        _ => synth_openai(&http, p, base, text).await,
    }
}

/// OpenAI 兼容：POST {ensure_v1(base)}/audio/speech，Bearer 鉴权。
async fn synth_openai(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let url = format!("{}/audio/speech", ensure_v1(base));
    let model = if p.model.trim().is_empty() { "tts-1" } else { p.model.trim() };
    let voice = if p.voice.trim().is_empty() { "alloy" } else { p.voice.trim() };
    let fmt = if p.format.trim().is_empty() { "mp3" } else { p.format.trim() };
    let body = TtsBody {
        model: model.to_string(),
        input: text.to_string(),
        voice: voice.to_string(),
        response_format: fmt.to_string(),
    };
    let mut req = http.post(&url).json(&body);
    for (k, v) in bearer_headers(p.api_key) {
        req = req.header(k, v);
    }
    post_audio(req).await
}

/// Azure OpenAI：地址含 deployments/{dep}/ 时直用该 URL；否则用 model 当部署名。
async fn synth_azure(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let url = if base.contains("/deployments/") {
        base.to_string()
    } else {
        if p.model.trim().is_empty() {
            return Err(AiError::config("Azure OpenAI 语音需要填写部署名（模型）"));
        }
        format!("{base}/openai/v1/audio/speech?api-version=preview")
    };
    let voice = if p.voice.trim().is_empty() { "alloy" } else { p.voice.trim() };
    let fmt = if p.format.trim().is_empty() { "mp3" } else { p.format.trim() };
    let body = TtsBody {
        model: p.model.trim().to_string(),
        input: text.to_string(),
        voice: voice.to_string(),
        response_format: fmt.to_string(),
    };
    let mut req = http.post(&url).json(&body);
    if !p.api_key.trim().is_empty() {
        req = req.header("api-key", p.api_key.trim());
    }
    post_audio(req).await
}

/// DashScope 通义千问：复用 synthesize_dashscope（JSON 回包 + 二次下载）。
async fn synth_dashscope(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let cfg = AiConfig {
        provider: "dashscope".into(),
        base_url: base.to_string(),
        api_key: p.api_key.to_string(),
        model: p.model.to_string(),
        deployment: String::new(),
        api_version: p.lang.to_string(),
    };
    synthesize_dashscope(http.clone(), &cfg, base, p.voice, text).await
}

/// MiniMax：POST {base}/v1/t2a_v2；音频在 data.audio（base64 或 hex）。
async fn synth_minimax(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let model = if p.model.trim().is_empty() { "speech-2.6-hd" } else { p.model.trim() };
    let voice = if p.voice.trim().is_empty() { "male-qn-qingse" } else { p.voice.trim() };
    let fmt = if p.format.trim().is_empty() { "mp3" } else { p.format.trim() };
    let url = format!("{base}/v1/t2a_v2");
    let body = json!({
        "model": model,
        "text": text,
        "voice_setting": { "voice_id": voice },
        "audio_setting": { "format": fmt },
    });
    let mut req = http.post(&url).json(&body);
    for (k, v) in bearer_headers(p.api_key) {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| classify_req(&e))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let brief: String = txt.chars().take(300).collect();
        return Err(AiError::http(status.as_u16(), brief));
    }
    let v: serde_json::Value =
        serde_json::from_str(&txt).map_err(|e| AiError::protocol(format!("响应解析失败：{e}")))?;
    let data = v
        .get("data")
        .and_then(|d| d.get("audio"))
        .and_then(|a| a.as_str())
        .unwrap_or("");
    if data.is_empty() {
        let msg = v
            .get("base_resp")
            .and_then(|b| b.get("status_msg"))
            .and_then(|m| m.as_str())
            .unwrap_or("响应中未找到音频数据");
        return Err(AiError::protocol(format!("MiniMax 合成失败：{msg}")));
    }
    decode_audio_b64_or_hex(data)
}

/// Fish Audio：POST {base}/v1/tts，请求头带 model，返回音频字节。
async fn synth_fish(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let model = if p.model.trim().is_empty() { "s1" } else { p.model.trim() };
    let fmt = if p.format.trim().is_empty() { "mp3" } else { p.format.trim() };
    let url = format!("{base}/v1/tts");
    let body = json!({ "text": text, "reference_id": p.voice, "format": fmt });
    let mut req = http.post(&url).header("model", model).json(&body);
    for (k, v) in bearer_headers(p.api_key) {
        req = req.header(k, v);
    }
    post_audio(req).await
}

/// GPT-SoVITS 原生 /tts：参考音频路径由 voice 给出。
async fn synth_gptsovits(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let lang = map_gptsovits_lang(p.lang);
    let fmt = if p.format.trim().is_empty() { "wav" } else { p.format.trim() };
    let url = format!("{base}/tts");
    let body = json!({
        "text": text,
        "text_lang": lang,
        "ref_audio_path": p.voice,
        "prompt_text": "",
        "prompt_lang": lang,
        "media_type": fmt,
    });
    post_audio(http.post(&url).json(&body)).await
}

/// 把 config.tts.lang 映射为 GPT-SoVITS 的语种代码。
fn map_gptsovits_lang(lang: &str) -> &'static str {
    let l = lang.trim().to_ascii_lowercase();
    if l.starts_with("ja") || l.starts_with("jp") {
        "ja"
    } else if l.starts_with("ko") {
        "ko"
    } else if l.starts_with("yue") || l.starts_with("cantonese") {
        "yue"
    } else if l.starts_with("en") {
        "en"
    } else {
        "zh"
    }
}

/// Breeze-TTS-2 本地网关：复用 synthesize_breeze（直连）。
async fn synth_breeze_dispatch(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    base: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if base.is_empty() {
        return Err(AiError::config("未填写服务地址"));
    }
    let local = breeze_client(base).unwrap_or_else(|_| http.clone());
    let cfg = AiConfig {
        provider: "breeze".into(),
        base_url: base.to_string(),
        api_key: p.api_key.to_string(),
        model: p.model.to_string(),
        deployment: String::new(),
        api_version: String::new(),
    };
    synthesize_breeze(local, &cfg, base, p.voice, text).await
}

/// 火山引擎（豆包）V1 HTTP：一次性 JSON + base64 返回。
async fn synth_volcano(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    const URL: &str = VOLCANO_TTS_URL;
    if p.app_id.trim().is_empty() {
        return Err(AiError::config("火山引擎语音需要填写 AppID"));
    }
    if p.api_key.trim().is_empty() {
        return Err(AiError::config("火山引擎语音需要填写 Access Token"));
    }
    let voice = if p.voice.trim().is_empty() {
        "zh_female_cancan_mars_bigtts"
    } else {
        p.voice.trim()
    };
    let fmt = if p.format.trim().is_empty() { "mp3" } else { p.format.trim() };
    let cluster = if p.cluster.trim().is_empty() { "volcano_tts" } else { p.cluster.trim() };
    let body = json!({
        "app": { "appid": p.app_id.trim(), "token": p.api_key.trim(), "cluster": cluster },
        "user": { "uid": "flowread" },
        "audio": { "voice_type": voice, "encoding": fmt, "rate": 24000 },
        "request": { "reqid": unique_reqid(), "text": text, "text_type": "plain", "operation": "query" },
    });
    let resp = http
        .post(URL)
        .header("Authorization", format!("Bearer;{}", p.api_key.trim()))
        .json(&body)
        .send()
        .await
        .map_err(|e| classify_req(&e))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let brief: String = txt.chars().take(300).collect();
        return Err(AiError::http(status.as_u16(), brief));
    }
    let v: serde_json::Value =
        serde_json::from_str(&txt).map_err(|e| AiError::protocol(format!("响应解析失败：{e}")))?;
    let code = v.get("code").map(json_i64).unwrap_or(0);
    if code != 3000 {
        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("合成失败");
        return Err(AiError::protocol(format!("火山引擎返回错误 {code}：{msg}")));
    }
    let data = v.get("data").and_then(|d| d.as_str()).unwrap_or("");
    if data.is_empty() {
        return Err(AiError::protocol("火山引擎响应中未找到音频数据"));
    }
    decode_audio_b64_or_hex(data)
}

/// 百度语音：先取 token（进程内缓存），再调短文本在线合成。
async fn synth_baidu(
    http: &reqwest::Client,
    p: &TtsParams<'_>,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    if p.api_key.trim().is_empty() {
        return Err(AiError::config("百度语音需要填写 API Key"));
    }
    if p.secret_key.trim().is_empty() {
        return Err(AiError::config("百度语音需要填写 Secret Key"));
    }
    let token = baidu_token(http, p.api_key.trim(), p.secret_key.trim()).await?;
    let fmt = if p.format.trim().is_empty() { "mp3" } else { p.format.trim() };
    let aue = match fmt.to_ascii_lowercase().as_str() {
        "wav" => "6",
        "pcm" => "4",
        _ => "3",
    };
    let per = if p.voice.trim().is_empty() { "0" } else { p.voice.trim() };
    let form = [
        ("tex", text),
        ("tok", token.as_str()),
        ("cuid", "flowread"),
        ("ctp", "1"),
        ("lan", "zh"),
        ("spd", "5"),
        ("pit", "5"),
        ("vol", "5"),
        ("per", per),
        ("aue", aue),
    ];
    let resp = http
        .post(BAIDU_TTS_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| classify_req(&e))?;
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = resp.bytes().await.map_err(|e| classify_req(&e))?;
    if ctype.starts_with("audio") {
        if bytes.is_empty() {
            return Err(AiError::protocol("百度语音返回空音频"));
        }
        return Ok(bytes.to_vec());
    }
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    let err_no = v.get("err_no").map(json_i64).unwrap_or(-1);
    let err_msg = v.get("err_msg").and_then(|x| x.as_str()).unwrap_or("未知错误");
    Err(AiError::protocol(format!("百度语音合成失败：err_no={err_no} {err_msg}")))
}

/// 百度语音 token 缓存：`{apiKey}\x1f{secretKey}` -> (token, 过期时间戳秒)。
fn baidu_token_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, (String, u64)>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (String, u64)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 获取百度语音 access_token；过期前 60 秒复用缓存。
async fn baidu_token(http: &reqwest::Client, api_key: &str, secret_key: &str) -> Result<String, AiError> {
    let key = format!("{api_key}\u{1f}{secret_key}");
    let now = now_unix_secs();
    if let Ok(m) = baidu_token_cache().lock() {
        if let Some((tok, exp)) = m.get(&key) {
            if *exp > now + 60 {
                return Ok(tok.clone());
            }
        }
    }
    let resp = http
        .post(BAIDU_TOKEN_URL)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", api_key),
            ("client_secret", secret_key),
        ])
        .send()
        .await
        .map_err(|e| classify_req(&e))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let brief: String = txt.chars().take(300).collect();
        return Err(AiError::http(status.as_u16(), brief));
    }
    let v: serde_json::Value =
        serde_json::from_str(&txt).map_err(|e| AiError::protocol(format!("响应解析失败：{e}")))?;
    let tok = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            let desc = v
                .get("error_description")
                .and_then(|x| x.as_str())
                .unwrap_or("未返回 access_token");
            AiError::protocol(format!("百度语音获取 token 失败：{desc}"))
        })?
        .to_string();
    let expires = v.get("expires_in").map(json_i64).unwrap_or(2_592_000).max(600) as u64;
    if let Ok(mut m) = baidu_token_cache().lock() {
        m.insert(key, (tok.clone(), now + expires));
    }
    Ok(tok)
}

/* ------------------------------ Edge TTS ------------------------------ */

const EDGE_TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
/// 伪装成较新的 Edge 稳定版；与 Sec-MS-GEC-Version 后缀保持一致。
const EDGE_CHROMIUM_VERSION: &str = "143.0.3650.75";

/// Edge TTS 免费接口：走 WebSocket（wss），直连，无需密钥与地址。
async fn synth_edge(p: &TtsParams<'_>, text: &str) -> Result<Vec<u8>, AiError> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let voice = if p.voice.trim().is_empty() {
        "zh-CN-XiaoxiaoNeural"
    } else {
        p.voice.trim()
    };
    let connection_id = uuid_v4();
    let gec = edge_sec_ms_gec();
    let url = format!(
        "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken={EDGE_TRUSTED_CLIENT_TOKEN}&ConnectionId={connection_id}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version=1-{EDGE_CHROMIUM_VERSION}"
    );
    let mut request = url
        .into_client_request()
        .map_err(|e| AiError::protocol(format!("Edge 请求构建失败：{e}")))?;
    {
        let headers = request.headers_mut();
        headers.insert(
            "Origin",
            "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".parse().unwrap(),
        );
        headers.insert(
            "User-Agent",
            format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{EDGE_CHROMIUM_VERSION} Safari/537.36 Edg/{EDGE_CHROMIUM_VERSION}"
            )
            .parse()
            .unwrap(),
        );
        headers.insert("Pragma", "no-cache".parse().unwrap());
        headers.insert("Cache-Control", "no-cache".parse().unwrap());
    }
    let (mut ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| AiError::network(format!("Edge 连接失败：{e}")))?;

    let config_msg = "Content-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{\"context\":{\"synthesis\":{\"audio\":{\"metadataoptions\":{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"true\"},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}";
    ws.send(Message::text(config_msg))
        .await
        .map_err(|e| AiError::network(format!("Edge 发送失败：{e}")))?;

    let request_id = uuid_v4();
    let timestamp = rfc3339_utc();
    let ssml = format!(
        "<speak version='1.0' xml:lang='zh-CN'><voice name='{voice}'><prosody rate='+0%' volume='+0%' pitch='+0Hz'>{}</prosody></voice></speak>",
        escape_xml(text)
    );
    let ssml_msg = format!(
        "X-RequestId:{request_id}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{timestamp}\r\nPath:ssml\r\n\r\n{ssml}"
    );
    ws.send(Message::text(ssml_msg))
        .await
        .map_err(|e| AiError::network(format!("Edge 发送失败：{e}")))?;

    let mut audio: Vec<u8> = Vec::new();
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => {
                if t.contains("Path:turn.end") {
                    break;
                }
            }
            Some(Ok(Message::Binary(b))) => {
                // 二进制帧：前 2 字节（大端）为 header 长度，其后 header，再其后为音频。
                if b.len() < 2 {
                    continue;
                }
                let header_len = ((b[0] as usize) << 8) | (b[1] as usize);
                let start = 2 + header_len;
                if start <= b.len() {
                    audio.extend_from_slice(&b[start..]);
                }
            }
            Some(Ok(Message::Close(_))) => break,
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(AiError::network(format!("Edge 接收失败：{e}"))),
            None => break,
        }
    }
    if audio.is_empty() {
        return Err(AiError::protocol("Edge TTS 未返回音频"));
    }
    Ok(audio)
}

/// 计算 Edge TTS 的 Sec-MS-GEC：Windows filetime ticks 向下取整到 5 分钟后，
/// 拼接可信客户端令牌做 SHA-256，取大写十六进制。
fn edge_sec_ms_gec() -> String {
    let unix = now_unix_secs() as i64;
    let ticks = (unix + 11_644_473_600).saturating_mul(10_000_000);
    let rounded = ticks / 3_000_000_000 * 3_000_000_000;
    let mut h = Sha256::new();
    h.update(format!("{rounded}{EDGE_TRUSTED_CLIENT_TOKEN}").as_bytes());
    h.finalize().iter().map(|b| format!("{b:02X}")).collect()
}

/// XML 转义（SSML 文本节点）。
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

/// 生成一个 UUID v4 形态的字符串（时间 + 计数器混合，仅用于请求 id）。
fn uuid_v4() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut bytes = [0u8; 16];
    let mut x = nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for b in bytes.iter_mut() {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *b = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 24) as u8;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// 唯一请求 id：时间戳 + 计数器做 SHA-256（十六进制）。
fn unique_reqid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(nanos.to_le_bytes());
    h.update(n.to_le_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 当前 Unix 秒（UTC）。
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC RFC3339 时间戳（秒精度），用于 Edge 的 X-Timestamp。
fn rfc3339_utc() -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(now_unix_secs() as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Unix 秒 -> (年, 月, 日, 时, 分, 秒)（Howard Hinnant 算法）。
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, mi, s)
}

/// JSON 数值/字符串统一取 i64。
fn json_i64(v: &serde_json::Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        .unwrap_or(0)
}

/// 解码服务返回的音频字段：先按 base64；若仅含十六进制字符且长度为偶数则按 hex。
fn decode_audio_b64_or_hex(s: &str) -> Result<Vec<u8>, AiError> {
    let t = s.trim();
    if !t.is_empty() && t.len() % 2 == 0 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes = t.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for i in (0..bytes.len()).step_by(2) {
            let hi = (bytes[i] as char).to_digit(16).unwrap_or(0);
            let lo = (bytes[i + 1] as char).to_digit(16).unwrap_or(0);
            out.push((hi * 16 + lo) as u8);
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(t)
        .map_err(|e| AiError::protocol(format!("音频 Base64 解码失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_id_shape() {
        assert!(valid_model_id("TARGET"));
        assert!(valid_model_id("deepseek-chat"));
        assert!(valid_model_id("openai/gpt-4o"));
        assert!(valid_model_id("llama3:latest"));
        assert!(!valid_model_id(""));
        assert!(!valid_model_id("TARGET\" , \"think\": false"));
        assert!(!valid_model_id("TARGET?think=false"));
        assert!(!valid_model_id("-H \"X-Think: 1\""));
        assert!(!valid_model_id("http://evil/v1/completions"));
        assert!(!valid_model_id("/v1/completions"));
        assert!(!valid_model_id("X-Think-Enabled: false"));
    }

    #[test]
    fn provider_parse() {
        assert_eq!(Provider::parse("openai"), Provider::OpenAi);
        assert_eq!(Provider::parse("ollama"), Provider::Ollama);
        assert_eq!(Provider::parse("anthropic"), Provider::Anthropic);
        assert_eq!(Provider::parse("claude"), Provider::Anthropic);
        assert_eq!(Provider::parse("gemini"), Provider::Gemini);
        assert_eq!(Provider::parse("azure-openai"), Provider::Azure);
        assert_eq!(Provider::parse("dashscope"), Provider::DashScope);
        assert_eq!(Provider::parse("qwen"), Provider::DashScope);
        assert_eq!(Provider::parse("whatever"), Provider::OpenAi);
    }

    #[test]
    fn tls_scope_local_and_private_hosts() {
        // loopback / 私网 / link-local（含 IPv6 ULA 与 fe80）允许放宽；公网严格校验。
        for h in [
            "127.0.0.1",
            "::1",
            "localhost",
            "10.0.0.5",
            "172.16.3.4",
            "172.31.255.1",
            "192.168.1.20",
            "169.254.10.1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
        ] {
            assert!(is_local_or_private_host(h), "{h} 应视为本地/私有");
        }
        for h in [
            "8.8.8.8",
            "1.1.1.1",
            "172.15.0.1",
            "172.32.0.1",
            "203.0.113.5",
            "2001:4860:4860::8888",
            "example.com",
            "evil.example",
        ] {
            assert!(!is_local_or_private_host(h), "{h} 应视为公网");
        }
        assert!(is_local_or_private_url("127.0.0.1:3900"));
        assert!(is_local_or_private_url("http://192.168.1.10:7863/api"));
        assert!(!is_local_or_private_url("https://openspeech.bytedance.com/api/v1/tts"));
        assert!(!is_local_or_private_url(""));
    }

    #[test]
    fn dashscope_tts_url_shape() {
        let full = "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
        assert_eq!(dashscope_tts_url("https://dashscope.aliyuncs.com"), full);
        assert_eq!(dashscope_tts_url("https://dashscope.aliyuncs.com/"), full);
        assert_eq!(dashscope_tts_url("https://dashscope.aliyuncs.com/api/v1"), full);
        assert_eq!(dashscope_tts_url(full), full);
    }

    #[test]
    fn breeze_provider_and_urls() {
        assert!(is_breeze_provider("breeze"));
        assert!(is_breeze_provider("Breeze-TTS-2"));
        assert!(is_breeze_provider("breeze_tts"));
        assert!(!is_breeze_provider("openai"));
        assert!(!is_breeze_provider("dashscope"));

        assert_eq!(
            breeze_clone_url("http://127.0.0.1:7863"),
            "http://127.0.0.1:7863/api/tts/clone"
        );
        assert_eq!(
            breeze_clone_url("http://127.0.0.1:7863/"),
            "http://127.0.0.1:7863/api/tts/clone"
        );
        assert_eq!(
            breeze_clone_url("http://127.0.0.1:7863/api"),
            "http://127.0.0.1:7863/api/tts/clone"
        );
        assert_eq!(
            breeze_clone_url("http://127.0.0.1:7863/v1"),
            "http://127.0.0.1:7863/api/tts/clone"
        );
        assert_eq!(
            breeze_clone_url("http://127.0.0.1:7863/api/tts/clone"),
            "http://127.0.0.1:7863/api/tts/clone"
        );
        assert_eq!(
            breeze_voices_url("http://127.0.0.1:7863"),
            "http://127.0.0.1:7863/api/voices"
        );
        assert_eq!(
            breeze_voices_url("http://127.0.0.1:7863/api"),
            "http://127.0.0.1:7863/api/voices"
        );
        assert_eq!(
            breeze_voices_url("http://127.0.0.1:7863/v1"),
            "http://127.0.0.1:7863/api/voices"
        );
        assert_eq!(
            breeze_voices_url("http://127.0.0.1:7863/v1/"),
            "http://127.0.0.1:7863/api/voices"
        );
        assert_eq!(
            breeze_voices_url("http://127.0.0.1:7863/api/voices"),
            "http://127.0.0.1:7863/api/voices"
        );

        assert_eq!(detect_tts_language("你好，世界"), "Chinese");
        assert_eq!(detect_tts_language("hello world"), "English");

        assert!(is_breeze_model("breeze-tts-2"));
        assert!(is_breeze_model("Breeze"));
        assert!(is_breeze_model(""));
        assert!(!is_breeze_model("omnivoice"));
        assert!(!is_breeze_model("tts-1"));
    }

    #[test]
    fn breeze_gateway_port_fallback() {
        // 端口就是网关端口：无回退候选。
        assert_eq!(swap_to_gateway_port("http://127.0.0.1:7863"), None);
        // 误填上游 8080：回退到同主机 7863。
        assert_eq!(
            swap_to_gateway_port("http://127.0.0.1:8080"),
            Some("http://127.0.0.1:7863".to_string())
        );
        // 局域网地址同样回退。
        assert_eq!(
            swap_to_gateway_port("http://192.168.1.20:8080"),
            Some("http://192.168.1.20:7863".to_string())
        );
        // 无端口：追加 7863 候选（http 默认 80 视为非网关端口）。
        assert_eq!(
            swap_to_gateway_port("http://127.0.0.1"),
            Some("http://127.0.0.1:7863".to_string())
        );
        // IPv6 字面量：保守不处理。
        assert_eq!(swap_to_gateway_port("http://[::1]:8080"), None);

        // 候选列表：配置地址在前，7863 回退在后；含 /v1、/api 等后缀先归一化。
        assert_eq!(
            breeze_root_candidates("http://127.0.0.1:8080/v1"),
            vec![
                "http://127.0.0.1:8080".to_string(),
                "http://127.0.0.1:7863".to_string()
            ]
        );
        assert_eq!(
            breeze_root_candidates("http://127.0.0.1:7863/api/voices"),
            vec!["http://127.0.0.1:7863".to_string()]
        );
        assert_eq!(
            breeze_root_candidates("http://192.168.1.20:8080"),
            vec![
                "http://192.168.1.20:8080".to_string(),
                "http://192.168.1.20:7863".to_string()
            ]
        );
    }

    #[test]
    fn proxy_server_parsing() {
        assert_eq!(parse_proxy_server("127.0.0.1:7897"), "127.0.0.1:7897");
        assert_eq!(
            parse_proxy_server("http=127.0.0.1:7897;https=127.0.0.1:7898"),
            "127.0.0.1:7898"
        );
        assert_eq!(
            parse_proxy_server("http=10.0.0.1:8080;https=10.0.0.1:8443;ftp=1.1.1.1:21"),
            "10.0.0.1:8443"
        );
        assert_eq!(parse_proxy_server("http=onlyhttp:3128"), "onlyhttp:3128");
    }

    #[test]
    fn request_urls_and_bodies() {
        let base = AiConfig {
            provider: "openai".into(),
            base_url: "https://api.deepseek.com".into(),
            api_key: "k".into(),
            model: "deepseek-chat".into(),
            deployment: "".into(),
            api_version: "".into(),
        };
        let openai = build_request(Provider::OpenAi, &base, "sys", &[("user".into(), "hi".into())], true).unwrap();
        assert_eq!(openai.url, "https://api.deepseek.com/v1/chat/completions");
        assert!(openai.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer k"));

        let anthropic_cfg = AiConfig {
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key: "ak".into(),
            model: "claude-3-5-sonnet-latest".into(),
            deployment: "".into(),
            api_version: "".into(),
        };
        let a = build_request(Provider::Anthropic, &anthropic_cfg, "sys", &[("user".into(), "hi".into())], true).unwrap();
        assert_eq!(a.url, "https://api.anthropic.com/v1/messages");
        assert!(a.headers.iter().any(|(k, v)| k == "x-api-key" && v == "ak"));
        assert!(a.headers.iter().any(|(k, v)| k == "anthropic-version" && v == "2023-06-01"));

        let gemini_cfg = AiConfig {
            provider: "gemini".into(),
            base_url: "https://generativelanguage.googleapis.com".into(),
            api_key: "gk".into(),
            model: "models/gemini-1.5-flash".into(),
            deployment: "".into(),
            api_version: "".into(),
        };
        let g = build_request(Provider::Gemini, &gemini_cfg, "", &[("user".into(), "hi".into())], true).unwrap();
        // 密钥不再进 URL query，改用请求头 x-goog-api-key。
        assert_eq!(
            g.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:streamGenerateContent?alt=sse"
        );
        assert!(!g.url.contains("key="), "URL 不得包含密钥");
        assert!(g.headers.iter().any(|(k, v)| k == "x-goog-api-key" && v == "gk"));

        let azure_cfg = AiConfig {
            provider: "azure-openai".into(),
            base_url: "https://x.openai.azure.com".into(),
            api_key: "zk".into(),
            model: "gpt-4o".into(),
            deployment: "my-deploy".into(),
            api_version: "2024-06-01".into(),
        };
        let z = build_request(Provider::Azure, &azure_cfg, "", &[("user".into(), "hi".into())], false).unwrap();
        assert_eq!(
            z.url,
            "https://x.openai.azure.com/openai/deployments/my-deploy/chat/completions?api-version=2024-06-01"
        );
        assert!(z.headers.iter().any(|(k, v)| k == "api-key" && v == "zk"));
    }

    #[test]
    fn proxy_credentials_are_masked() {
        assert_eq!(mask_creds("http://user:pass@127.0.0.1:7897"), "http://***:***@127.0.0.1:7897");
        assert_eq!(mask_creds("socks5://u:p@h:1080"), "socks5://***:***@h:1080");
        assert_eq!(mask_creds("user:pass@host:8080"), "***:***@host:8080");
        // 无凭据的普通地址保持不变
        assert_eq!(mask_creds("http://127.0.0.1:7897"), "http://127.0.0.1:7897");
    }

    #[test]
    fn error_url_query_is_redacted() {
        let s = "error sending request for url (https://generativelanguage.googleapis.com/v1beta/models/x:generateContent?key=SECRET)";
        let out = redact_query(s);
        assert!(!out.contains("SECRET"), "查询串中的密钥不应外泄: {out}");
        assert!(out.contains("?<已隐藏>"));
    }

    #[test]
    fn streaming_parsers() {
        let oai: serde_json::Value =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":"你好"}}]}"#).unwrap();
        assert_eq!(stream_delta(Provider::OpenAi, &oai).as_deref(), Some("你好"));

        let claude: serde_json::Value = serde_json::from_str(
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}"#,
        )
        .unwrap();
        assert_eq!(stream_delta(Provider::Anthropic, &claude).as_deref(), Some("Hi"));

        let gem: serde_json::Value = serde_json::from_str(
            r#"{"candidates":[{"content":{"parts":[{"text":"a"},{"text":"b"}]}}]}"#,
        )
        .unwrap();
        assert_eq!(stream_delta(Provider::Gemini, &gem).as_deref(), Some("ab"));

        let oll: serde_json::Value =
            serde_json::from_str(r#"{"message":{"content":"x"},"done":false}"#).unwrap();
        assert_eq!(stream_delta(Provider::Ollama, &oll).as_deref(), Some("x"));

        let claude_full: serde_json::Value =
            serde_json::from_str(r#"{"content":[{"type":"text","text":"全"}]}"#).unwrap();
        assert_eq!(full_content(Provider::Anthropic, &claude_full).as_deref(), Some("全"));
    }

    #[test]
    fn error_kind_classification() {
        assert_eq!(AiError::http(401, "x").kind, "http_4xx");
        assert_eq!(AiError::http(429, "x").kind, "http_4xx");
        assert_eq!(AiError::http(502, "x").kind, "http_5xx");
        assert_eq!(AiError::model_id("x").kind, "model_id");
        assert_eq!(AiError::protocol("x").kind, "protocol");
    }
}

#[cfg(test)]
mod budget_sanitize {
    use super::*;

    #[test]
    fn strips_all_budget_variants() {
        let msgs = vec![("user".to_string(), "正文第一段\nbudget:token_budget200000\n正文结尾".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, "正文第一段\n正文结尾");
        assert!(!out[0].1.to_ascii_lowercase().contains("budget"));
    }

    #[test]
    fn strips_spaced_and_noisy_variant() {
        let msgs = vec![("user".to_string(), "开头\nbudget:token budget2o0000\n尾部".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out[0].1, "开头\n尾部");
    }

    #[test]
    fn strips_xml_tag_variant() {
        let msgs = vec![("user".to_string(), "A\n<budget:token_budget>20oooo</budget:token_budget>\nB".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out[0].1, "A\nB");
    }

    #[test]
    fn strips_bare_count_line() {
        let msgs = vec![("user".to_string(), "X\nbudget: 200000\nY".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out[0].1, "X\nY");
    }

    #[test]
    fn keeps_prose_with_budget_word() {
        let msgs = vec![("user".to_string(),
            "The quarterly budget review is scheduled.\nWe discussed the token budget for Q3.\nEnd.".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out[0].1,
            "The quarterly budget review is scheduled.\nWe discussed the token budget for Q3.\nEnd.");
    }

    #[test]
    fn keeps_budget_followed_by_year() {
        let msgs = vec![("user".to_string(), "Our budget 2024 plan\n正文".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out[0].1, "Our budget 2024 plan\n正文");
    }

    #[test]
    fn never_empty_on_all_budget() {
        let msgs = vec![("user".to_string(), "budget:token_budget200000\ntoken budget:999".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out.len(), 1);
        assert!(!out[0].1.trim().is_empty());
    }

    #[test]
    fn wraps_are_not_empty_either() {
        let msgs = vec![("user".to_string(), "<budget:token_budget>20oooo</budget:token_budget>".to_string())];
        let out = sanitize_messages(&msgs);
        assert!(!out[0].1.trim().is_empty());
    }

    #[test]
    fn dedupes_adjacent_identical_messages() {
        let m = ("user".to_string(), "重复正文".to_string());
        let msgs = vec![m.clone(), m.clone(), ("assistant".to_string(), "ok".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1, "重复正文");
        assert_eq!(out[1].1, "ok");
    }

    #[test]
    fn strips_two_word_colon_count() {
        let msgs = vec![("user".to_string(), "token budget:999".to_string())];
        let out = sanitize_messages(&msgs);
        assert_eq!(out.len(), 1);
        assert!(
            !out[0].1.to_ascii_lowercase().contains("budget"),
            "budget 字样未清除: {:?}",
            out[0].1
        );
    }

    /// 走真实出站构造路径（build_request → sanitize_messages → trace_budget）。
    /// 外部以 `FLOWREAD_AI_TRACE=1 cargo test trace_real_path -- --nocapture` 运行时，
    /// 会打印 `[ai-trace] total=/hits=`。断言：出站 body 不含 budget 字样。
    #[test]
    fn trace_real_path() {
        let cfg = AiConfig {
            provider: "openai".to_string(),
            base_url: "http://127.0.0.1:1/v1".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            deployment: String::new(),
            api_version: String::new(),
        };
        // 一条被污染的（第2行是注入元数据）+ 一条干净正文。
        let msgs: Vec<(String, String)> = vec![
            ("user".to_string(), "请总结这一章。\nbudget:token_budget200000\n以上。".to_string()),
            ("assistant".to_string(), "好的。".to_string()),
        ];
        let prepared = build_request(Provider::parse("openai"), &cfg, "", &msgs, false).unwrap();
        let body = serde_json::to_string(&prepared.body).unwrap();
        assert!(
            !body.to_ascii_lowercase().contains("budget"),
            "出站 body 仍含 budget: {body}"
        );
    }

    /// 真实 HTTP 往返（本地 socket），走生产出站构造路径：
    ///   1) trace_budget(&raw)  —— pre（原始 messages）
    ///   2) build_request(...)   —— 内部 sanitize + trace_budget(cleaned) post
    ///   3) 真发 TCP HTTP POST 到本地服务并读回 200
    /// 以 `FLOWREAD_AI_TRACE=1 cargo test real_http_roundtrip_trace -- --nocapture` 运行。
    #[test]
    fn real_http_roundtrip_trace() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 16384];
                let _ = sock.read(&mut buf);
                let body = "{\"choices\":[{\"message\":{\"content\":\"ok\"}}]}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });

        let cfg = AiConfig {
            provider: "openai".to_string(),
            base_url: format!("http://{addr}/v1"),
            api_key: "k".to_string(),
            model: "m".to_string(),
            deployment: String::new(),
            api_version: String::new(),
        };
        // 真实消息：正文 + 一行注入元数据。
        let raw: Vec<(String, String)> =
            vec![("user".to_string(), "正文\nbudget:token_budget200000\n尾部".to_string())];

        trace_budget(&raw); // pre
        let prepared = build_request(Provider::parse("openai"), &cfg, "", &raw, false).unwrap(); // 内部 post
        let body = serde_json::to_string(&prepared.body).unwrap();
        assert!(!body.to_ascii_lowercase().contains("budget"), "出站 body 含 budget");

        // 真发 TCP HTTP POST。
        let mut stream = TcpStream::connect(addr).unwrap();
        let req = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            prepared.url, addr, body.len(), body
        );
        stream.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "未拿到 200: {}", &resp.chars().take(80).collect::<String>());
        srv.join().ok();
    }

    /// 证明 trace 的独立宽松检测器**能捕获残留**（不是同源 no-op）：
    /// 直接对未清洗的污染文本调用 trace_budget，应打印 HIT head=。
    #[test]
    fn trace_detects_residue() {
        let polluted: Vec<(String, String)> = vec![
            ("user".to_string(), "正常正文\n<budget:token_budget>20oooo</budget:token_budget>".to_string()),
        ];
        trace_budget(&polluted); // FLOWREAD_AI_TRACE=1 时打印 hits=1 + head=
    }
}
