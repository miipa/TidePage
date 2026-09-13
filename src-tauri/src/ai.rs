use futures_util::StreamExt;
use serde_json::json;
use tauri::Emitter;
use std::time::Duration;

pub struct AiConfig {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// "auto" | "off"；仅 Ollama /api/chat 协议明确支持 think:false，
    /// 其他适配器收到 off 必须显式报错，不得静默按 auto 或注入未知字段
    pub reasoning_mode: String,
    /// off 时 OpenAI 兼容端点的关闭策略：switch-model(默认,无注入) / json-think-false /
    /// query-think-false / header-think-false。任何策略都不得把参数并入 model 字段。
    pub reasoning_strategy: String,
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

pub fn build_client(proxy_mode: &str, proxy_url: &str) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(20))
        .danger_accept_invalid_certs(true);
    let with_bypass = |u: String| -> Option<reqwest::Proxy> {
        reqwest::Proxy::all(&u)
            .ok()
            .map(|p| p.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1")))
    };
    match proxy_mode {
        "custom" => {
            let mut u = proxy_url.trim().trim_end_matches('/').to_string();
            if u.is_empty() {
                return Err("自定义代理地址不能为空".into());
            }
            if !u.contains("://") {
                u = format!("http://{u}");
            }
            let p = with_bypass(u.clone()).ok_or_else(|| format!("代理地址无效: {u}"))?;
            b = b.proxy(p);
        }
        "direct" => {
            b = b.no_proxy();
        }
        "auto" => {
            let env_proxy = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
                .iter()
                .find_map(|k| std::env::var(k).ok())
                .filter(|v| !v.is_empty());
            let addr = env_proxy.or_else(windows_system_proxy);
            if let Some(addr) = addr {
                let u = if addr.contains("://") { addr } else { format!("http://{addr}") };
                if let Some(p) = with_bypass(u) {
                    b = b.proxy(p);
                }
            }
        }
        _ => {
            if let Some(addr) = windows_system_proxy() {
                let u = if addr.contains("://") { addr } else { format!("http://{addr}") };
                if let Some(p) = with_bypass(u) {
                    b = b.proxy(p);
                }
            }
        }
    }
    b.build().map_err(|e| e.to_string())
}

fn windows_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let key = hk
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let server: String = key.get_value("ProxyServer").ok()?;
    if server.is_empty() {
        return None;
    }
    if let Some(pos) = server.find('=') {
        let (_, rest) = server.split_at(pos);
        let rest = rest.trim_start_matches('=');
        let pick = rest.split(';').find(|s| s.contains(":")).unwrap_or(rest);
        return Some(pick.to_string());
    }
    Some(server)
}

pub async fn test_connection(
    provider: String,
    base_url: String,
    api_key: String,
    model: String,
    proxy_mode: String,
    proxy_url: String,
) -> Result<String, String> {
    let mut clients: Vec<(&'static str, reqwest::Client)> = Vec::new();
    let primary_name: &'static str = match proxy_mode.as_str() {
        "custom" => "自定义代理",
        "system" => "系统代理",
        _ => "直连",
    };
    if let Ok(c) = build_client(&proxy_mode, &proxy_url) {
        clients.push((primary_name, c));
    }
    if proxy_mode != "direct" {
        if let Ok(c) = build_client("direct", "") {
            clients.push(("直连(自动回退)", c));
        }
    } else if let Some(addr) = windows_system_proxy() {
        if let Ok(c) = build_client("custom", &addr) {
            clients.push(("系统代理(自动尝试)", c));
        }
    }
    if clients.is_empty() {
        clients.push(("直连", build_client("direct", "")?));
    }

    let is_ollama = provider == "ollama";
    let (url, body) = if is_ollama {
        let base = base_url.trim_end_matches('/');
        (
            format!("{base}/api/chat"),
            json!({
                "model": model,
                "messages": [{"role": "user", "content": "回复 OK"}],
                "stream": false,
                "options": {"num_predict": 8}
            }),
        )
    } else {
        let base = base_url.trim_end_matches('/');
        let base = if base.ends_with("/v1") {
            base.to_string()
        } else {
            format!("{base}/v1")
        };
        (
            format!("{base}/chat/completions"),
            json!({
                "model": model,
                "messages": [{"role": "user", "content": "回复 OK"}],
                "max_tokens": 8,
                "stream": false
            }),
        )
    };

    let mut errs = Vec::new();
    for (name, http) in &clients {
        let mut req = http.clone().post(&url).timeout(Duration::from_secs(25)).json(&body);
        if !is_ollama && !api_key.is_empty() {
            req = req.bearer_auth(&api_key);
        }
        let outcome = async {
            let resp = req.send().await.map_err(|e| format!("网络失败: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                let brief: String = text.chars().take(220).collect();
                return Err(format!("HTTP {status}: {brief}"));
            }
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let reply = parsed
                .pointer("/choices/0/message/content")
                .or_else(|| parsed.pointer("/message/content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            Ok::<String, String>(if reply.is_empty() {
                "✓ 连接成功".into()
            } else {
                format!("✓ 连接成功（{name}），模型回复：{reply}")
            })
        }
        .await;
        match outcome {
            Ok(msg) => return Ok(msg),
            Err(e) => errs.push(format!("【{name}】{e}")),
        }
    }
    Err(format!("全部尝试失败 → {}", errs.join(" ； ")))
}

pub async fn stream_chat(
    app: tauri::AppHandle,
    http: reqwest::Client,
    id: String,
    cfg: AiConfig,
    system: String,
    messages: Vec<(String, String)>,
) -> Result<(), String> {
    let is_ollama = cfg.provider == "ollama";

    let mut msgs = vec![json!({"role": "system", "content": system.clone()})];
    for (role, content) in &messages {
        msgs.push(json!({"role": role, "content": content}));
    }

    // 策略决定完整协议包（URL + body 结构 + query/header + 解析器）。auto 一律走 chat 协议且零注入。
    let (final_url, protocol, body, use_auth, header_think) = if is_ollama {
        let url = {
            let base = cfg.base_url.trim_end_matches('/');
            format!("{base}/api/chat")
        };
        let mut b = json!({
            "model": cfg.model,
            "messages": msgs,
            "stream": true,
            "options": {"num_ctx": 8192}
        });
        // think 是 Ollama /api/chat 的协议级字段（官方 API 文档），off 时显式下发
        if cfg.reasoning_mode == "off" {
            b["think"] = json!(false);
        }
        (url, OffProtocol::Chat, b, false, false)
    } else {
        if !valid_model_id(&cfg.model) {
            let msg = "模型ID含非法字符（JSON/URL/query/请求头/curl 不能作为模型ID），请求已拦截。".to_string();
            let _ = app.emit(&format!("ai-token:{id}"), serde_json::json!({"error": &msg, "done": true}));
            return Err(msg);
        }
        let plan = openai_off_plan(&cfg.base_url, &cfg.model, &cfg.reasoning_mode, &cfg.reasoning_strategy);
        // model-complete 响应字段未经真实探针确认 → 解析器未实现，发送前显式拒绝（不猜解析、不发空白）
        if plan.protocol == OffProtocol::ModelCompleteUnsupported {
            let msg = "model-complete 协议的响应字段未经「仅回复OK」探针确认，解析器未实现：拒绝发送（不猜响应格式）。".to_string();
            let _ = app.emit(&format!("ai-token:{id}"), serde_json::json!({"error": &msg, "done": true}));
            return Err(msg);
        }
        let b = match plan.protocol {
            OffProtocol::Completions => {
                // /v1/completions 无 messages 结构：system+user 按顺序合并为 prompt（显式协议映射，非暗改）
                let mut parts: Vec<String> = Vec::new();
                if !system.is_empty() { parts.push(system.clone()); }
                for (_, content) in &messages { parts.push(content.clone()); }
                let mut j = json!({
                    "model": cfg.model,
                    "prompt": parts.join("\n\n"),
                    "stream": true,
                    "temperature": 0.7
                });
                if plan.body_think { j["think"] = json!(false); }
                j
            }
            _ => {
                let mut j = json!({
                    "model": cfg.model,
                    "messages": msgs,
                    "stream": true,
                    "temperature": 0.7
                });
                if plan.body_think { j["think"] = json!(false); }
                j
            }
        };
        (plan.url, plan.protocol, b, !cfg.api_key.is_empty(), plan.header_think)
    };

    let mut req = http.post(&final_url).timeout(std::time::Duration::from_secs(300));
    if header_think {
        req = req.header("X-Think-Enabled", "false");
    }
    if use_auth {
        use base64::Engine;
        let cred = format!("Bearer {}", cfg.api_key);
        req = req.header(
            "Authorization",
            reqwest::header::HeaderValue::from_str(&cred).map_err(|_| "无效的 API Key")?,
        );
    }

    let resp = req.json(&body).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let msg: String = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.get("message"))
                    .and_then(|m| m.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_else(|| {
                format!("HTTP {status}: {}", text.chars().take(300).collect::<String>())
            });
        let _ = app.emit(&format!("ai-token:{id}"), serde_json::json!({"error": msg, "done": true}));
        return Err(msg);
    }

    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let is_sse = ct.contains("text/event-stream") || (is_ollama && ct.contains("ndjson"));

    let emit_delta = |app: &tauri::AppHandle, id: &str, content: &str, done: bool| {
        let _ = app.emit(
            &format!("ai-token:{id}"),
            serde_json::json!({"delta": content, "done": done}),
        );
    };
    let emit_err = |app: &tauri::AppHandle, id: &str, msg: &str| {
        let _ = app.emit(
            &format!("ai-token:{id}"),
            serde_json::json!({"error": msg, "done": true}),
        );
    };

    if !is_sse {
        let text = resp.text().await.map_err(|e| e.to_string())?;
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let content = parsed
            .pointer("/choices/0/message/content")
            .or_else(|| parsed.pointer("/choices/0/delta/content"))
            .or_else(|| parsed.pointer("/choices/0/text"))
            .or_else(|| parsed.pointer("/message/content"))
            .or_else(|| parsed.pointer("/content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if content.is_empty() {
            let brief: String = text.chars().take(300).collect();
            let msg = format!("响应协议不匹配：端点返回非流式响应（Content-Type: {ct}），且无该协议正文字段。响应片段：{brief}");
            emit_err(&app, &id, &msg);
            return Err(msg);
        }
        emit_delta(&app, &id, content, true);
        return Ok(());
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut got_any = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }
            if is_ollama {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    // off 模式收到非空 thinking = 服务/模型未真正执行关闭，显式报错而非静默丢弃
                    if cfg.reasoning_mode == "off" {
                        if let Some(think) = v.pointer("/message/thinking").and_then(|t| t.as_str()) {
                            if !think.is_empty() {
                                let msg = "服务或模型未执行关闭思考（off 模式仍返回 thinking 内容）".to_string();
                                emit_err(&app, &id, &msg);
                                return Err(msg);
                            }
                        }
                    }
                    if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_str()) {
                        got_any = true;
                        let done = v.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
                        emit_delta(&app, &id, content, done);
                    }
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    if !got_any {
                        // 200 但全程无该协议正文字段 → 显式报错，绝不静默完成出空白回复
                        let msg = format!("响应协议不匹配：流结束前未找到该协议的正文字段（chat=choices[].delta.content / completions=choices[].text）。响应片段：{}", buf.chars().take(200).collect::<String>());
                        emit_err(&app, &id, &msg);
                        return Err(msg);
                    }
                    emit_delta(&app, &id, "", true);
                    return Ok(());
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = extract_stream_content(protocol, &v) {
                        if !content.is_empty() {
                            got_any = true;
                        }
                        emit_delta(&app, &id, &content, false);
                    }
                }
            }
        }
    }
    if !got_any {
        let ct2 = if ct.is_empty() { "未知" } else { &ct };
        let brief: String = buf.chars().take(300).collect();
        let parsed: serde_json::Value = serde_json::from_str(buf.trim()).unwrap_or_default();
        let content = parsed
            .pointer("/choices/0/message/content")
            .or_else(|| parsed.pointer("/message/content"))
            .or_else(|| parsed.pointer("/content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if !content.is_empty() {
            emit_delta(&app, &id, content, true);
            return Ok(());
        }
        let msg = format!(
            "响应协议不匹配：流式响应中未找到该协议的正文字段（chat=choices[].delta.content / completions=choices[].text），拒绝空白回复（Content-Type: {ct2}）。响应片段：{brief}"
        );
        emit_err(&app, &id, &msg);
        return Err(msg);
    }
    emit_delta(&app, &id, "", true);
    Ok(())
}

/// 响应正文解析协议。model-complete 在真实探针确认响应字段前不实现（不猜），命中即发送前拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffProtocol { Chat, Completions, ModelCompleteUnsupported }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffPlan {
    pub url: String,
    pub protocol: OffProtocol,
    pub body_think: bool,
    pub header_think: bool,
}

/// 每个策略绑定完整协议：URL + body 结构（由 protocol 决定 messages/prompt）+ query/header + 解析器。
/// 关键：query/header 策略命中 /v1/models/{enc(model)}/complete，绝不落回 chat 路径。
/// auto 或未知/switch-model 策略：走 chat 协议且零注入（model 由上层已切换）。
pub fn openai_off_plan(base_url: &str, model: &str, mode: &str, strategy: &str) -> OffPlan {
    let base = base_url.trim_end_matches('/');
    let v1 = if base.ends_with("/v1") { base.to_string() } else { format!("{base}/v1") };
    let chat = || OffPlan { url: format!("{v1}/chat/completions"), protocol: OffProtocol::Chat, body_think: false, header_think: false };
    if mode != "off" {
        return chat();
    }
    match strategy {
        "chat-json-think-false" => OffPlan { url: format!("{v1}/chat/completions"), protocol: OffProtocol::Chat, body_think: true, header_think: false },
        "completions-json-think-false" => OffPlan { url: format!("{v1}/completions"), protocol: OffProtocol::Completions, body_think: true, header_think: false },
        "model-complete-query-think-false" => OffPlan { url: format!("{v1}/models/{}/complete?think=false", percent_encode(model)), protocol: OffProtocol::ModelCompleteUnsupported, body_think: false, header_think: false },
        "model-complete-header-think-false" => OffPlan { url: format!("{v1}/models/{}/complete", percent_encode(model)), protocol: OffProtocol::ModelCompleteUnsupported, body_think: false, header_think: true },
        _ => chat(),
    }
}

/// 按协议提取流式增量正文。completions 取 choices[].text；chat 取 choices[].delta.content。
pub fn extract_stream_content(protocol: OffProtocol, v: &serde_json::Value) -> Option<String> {
    match protocol {
        OffProtocol::Chat => v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()).map(String::from),
        OffProtocol::Completions => v.pointer("/choices/0/text").and_then(|c| c.as_str()).map(String::from),
        OffProtocol::ModelCompleteUnsupported => None,
    }
}

/// 极简 percent-encode（对齐 JS encodeURIComponent）：仅保留 A-Za-z0-9 与 -_.~，其余按 UTF-8 字节 %XX 大写。
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn strategy_binds_full_protocol() {
        let b = "https://h/v1";
        // chat：URL chat，body think
        let p = openai_off_plan(b, "TARGET", "off", "chat-json-think-false");
        assert_eq!(p.url, "https://h/v1/chat/completions");
        assert_eq!(p.protocol, OffProtocol::Chat);
        assert!(p.body_think);
        assert!(!p.header_think);
        // completions：URL /v1/completions，非 chat 路径
        let p = openai_off_plan(b, "TARGET", "off", "completions-json-think-false");
        assert_eq!(p.url, "https://h/v1/completions");
        assert_eq!(p.protocol, OffProtocol::Completions);
        assert!(p.body_think);
        // model-complete query：命中 models/{enc}/complete?think=false，绝不落 chat
        let p = openai_off_plan(b, "org/mod el", "off", "model-complete-query-think-false");
        assert_eq!(p.url, "https://h/v1/models/org%2Fmod%20el/complete?think=false");
        assert_eq!(p.protocol, OffProtocol::ModelCompleteUnsupported);
        // model-complete header：URL 无 query，header 注入
        let p = openai_off_plan(b, "TARGET", "off", "model-complete-header-think-false");
        assert_eq!(p.url, "https://h/v1/models/TARGET/complete");
        assert!(p.header_think);
        assert!(!p.url.contains("chat/completions"));
    }

    #[test]
    fn auto_has_no_injection_and_switch_model_uses_chat() {
        for s in ["chat-json-think-false", "completions-json-think-false", "model-complete-query-think-false", "model-complete-header-think-false", "switch-model"] {
            let p = openai_off_plan("https://h", "TARGET", "auto", s);
            assert_eq!(p.protocol, OffProtocol::Chat, "auto must stay chat for {s}");
            assert!(!p.body_think && !p.header_think, "auto must not inject for {s}");
            assert!(!p.url.contains("think=false"), "auto url clean");
        }
        let p = openai_off_plan("https://h", "TARGET", "off", "switch-model");
        assert_eq!(p.url, "https://h/v1/chat/completions");
        assert!(!p.body_think && !p.header_think); // switch-model 不改协议参数，model 由上层已切换
    }

    #[test]
    fn base_url_without_v1_appended() {
        assert_eq!(openai_off_plan("https://h/", "M", "off", "completions-json-think-false").url, "https://h/v1/completions");
        assert_eq!(openai_off_plan("https://h/v1", "M", "off", "chat-json-think-false").url, "https://h/v1/chat/completions");
    }

    #[test]
    fn parsers_read_correct_fields() {
        let chat = json!({ "choices": [{ "delta": { "content": "OK" } }] });
        assert_eq!(extract_stream_content(OffProtocol::Chat, &chat).as_deref(), Some("OK"));
        assert_eq!(extract_stream_content(OffProtocol::Completions, &chat), None); // delta 不属于 completions
        let comp = json!({ "choices": [{ "text": "OK" }] });
        assert_eq!(extract_stream_content(OffProtocol::Completions, &comp).as_deref(), Some("OK"));
        assert_eq!(extract_stream_content(OffProtocol::Chat, &comp), None);
    }

    #[test]
    fn percent_encode_matches_uri_component() {
        assert_eq!(percent_encode("TARGET"), "TARGET");
        assert_eq!(percent_encode("org/model"), "org%2Fmodel");
        assert_eq!(percent_encode("gpt 4:o.x-y"), "gpt%204%3Ao.x-y");
        assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
    }
}
