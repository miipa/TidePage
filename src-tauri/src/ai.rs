use futures_util::StreamExt;
use serde_json::json;
use tauri::Emitter;
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

fn classify_req(e: &reqwest::Error) -> AiError {
    if e.is_timeout() {
        AiError::timeout(format!("请求超时: {e}"))
    } else {
        AiError::network(format!("网络失败: {e}"))
    }
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

pub fn build_client(proxy_mode: &str, proxy_url: &str) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(20))
        .danger_accept_invalid_certs(true);
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
}

impl Provider {
    pub fn parse(s: &str) -> Provider {
        match s.to_ascii_lowercase().as_str() {
            "ollama" => Provider::Ollama,
            "anthropic" | "claude" => Provider::Anthropic,
            "gemini" | "google" => Provider::Gemini,
            "azure-openai" | "azure" | "azure_openai" => Provider::Azure,
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
        Provider::OpenAi => Ok(Prepared {
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
            let url = format!("{base}/v1beta/models/{model}:{method}");
            let sep = if url.contains('?') { '&' } else { '?' };
            let url = format!("{url}{sep}key={}", cfg.api_key);
            let sys = if system.trim().is_empty() {
                None
            } else {
                Some(json!({"parts": [{"text": system}]}))
            };
            Ok(Prepared {
                url,
                headers: vec![("Content-Type".into(), "application/json".into())],
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
    let mut candidates: Vec<(String, reqwest::Client)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    match mode {
        "custom" => {
            let shown = normalize_proxy_url(&proxy_url).unwrap_or_else(|_| proxy_url.clone());
            match build_client("custom", &proxy_url) {
                Ok(c) => candidates.push((format!("自定义代理 {shown}"), c)),
                Err(e) => notes.push(format!("自定义代理不可用：{e}")),
            }
        }
        "direct" => match build_client("direct", "") {
            Ok(c) => candidates.push(("直连".into(), c)),
            Err(e) => notes.push(format!("直连客户端构建失败：{e}")),
        },
        _ => {
            let r = resolve_auto_proxy();
            match &r.url {
                Some(addr) => match build_client("custom", addr) {
                    Ok(c) => candidates.push((format!("自动代理（{}）", r.source), c)),
                    Err(e) => notes.push(format!("自动代理（{}）不可用：{e}", r.source)),
                },
                None => {
                    notes.push(r.source.clone());
                    if let Ok(c) = build_client("direct", "") {
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
        if let Ok(c) = build_client("direct", "") {
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
    Err(format!("✗ 全部尝试失败 → {}", all.join(" ； ")))
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

#[derive(serde::Serialize)]
struct TtsBody {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

/// 调用 OpenAI 兼容 / Azure OpenAI 的语音合成接口，返回音频字节（如 mp3）。
pub async fn synthesize(
    http: reqwest::Client,
    cfg: AiConfig,
    voice: &str,
    format: &str,
    text: &str,
) -> Result<Vec<u8>, AiError> {
    let p = Provider::parse(&cfg.provider);
    let base = strip_trailing_slash(&cfg.base_url);
    if base.is_empty() {
        return Err(AiError::config("未配置语音端点地址"));
    }
    let (url, headers) = match p {
        Provider::OpenAi | Provider::Ollama => {
            // Ollama 无 /audio/speech，最终会返回 4xx，这里统一按 OpenAI 兼容路径尝试
            let b = ensure_v1(base);
            (format!("{b}/audio/speech"), bearer_headers(&cfg.api_key))
        }
        Provider::Azure => {
            let dep = cfg.deployment.trim();
            if dep.is_empty() {
                return Err(AiError::config("Azure OpenAI 语音需要填写部署名（deployment）"));
            }
            let ver = if cfg.api_version.trim().is_empty() {
                "2024-06-01"
            } else {
                cfg.api_version.trim()
            };
            let mut h: Vec<(String, String)> = Vec::new();
            if !cfg.api_key.trim().is_empty() {
                h.push(("api-key".into(), cfg.api_key.clone()));
            }
            (
                format!("{base}/openai/deployments/{dep}/audio/speech?api-version={ver}"),
                h,
            )
        }
        _ => {
            return Err(AiError::config(
                "在线语音仅支持 OpenAI 兼容或 Azure OpenAI 端点；请改用系统语音",
            ))
        }
    };
    let fmt = if format.trim().is_empty() { "mp3" } else { format.trim() };
    let voice = if voice.trim().is_empty() { "alloy" } else { voice.trim() };
    let body = TtsBody {
        model: cfg.model.clone(),
        input: text.to_string(),
        voice: voice.to_string(),
        response_format: fmt.to_string(),
    };
    let mut req = http.post(&url).json(&body);
    for (k, v) in headers {
        req = req.header(k, v);
    }
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
        assert_eq!(Provider::parse("whatever"), Provider::OpenAi);
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
        assert_eq!(
            g.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:streamGenerateContent?alt=sse&key=gk"
        );

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
