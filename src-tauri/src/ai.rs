use futures_util::StreamExt;
use serde_json::json;
use tauri::Emitter;
use std::time::Duration;

pub struct AiConfig {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
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

// 出站请求体用结构体声明字段顺序，保证 Object.keys(body) 稳定为约定值，且绝不夹带额外字段。
#[derive(serde::Serialize)]
struct OllamaChatBody {
    messages: Vec<serde_json::Value>,
    model: String,
    options: serde_json::Value,
    stream: bool,
}

#[derive(serde::Serialize)]
struct OpenAiChatBody {
    model: String,
    messages: Vec<serde_json::Value>,
    stream: bool,
    temperature: f64,
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum ChatBody {
    Ollama(OllamaChatBody),
    OpenAi(OpenAiChatBody),
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

    let mut msgs = vec![json!({"role": "system", "content": system})];
    for (role, content) in &messages {
        msgs.push(json!({"role": role, "content": content}));
    }

    // “关闭思考”功能已彻底移除：请求构造恒为 auto 行为（零额外注入：不改 body 字段/query/请求头），
    // Ollama → /api/chat，OpenAI 兼容 → /v1/chat/completions，与功能引入前一致。
    // 保留模型ID形状守卫：防止把 JSON 片段/URL/请求头/curl 塞进 model。
    let (final_url, body, use_auth) = if is_ollama {
        let base = cfg.base_url.trim_end_matches('/');
        let url = format!("{base}/api/chat");
        let b = ChatBody::Ollama(OllamaChatBody {
            messages: msgs,
            model: cfg.model.clone(),
            options: json!({"num_ctx": 8192}),
            stream: true,
        });
        (url, b, false)
    } else {
        if !valid_model_id(&cfg.model) {
            let msg = "模型ID含非法字符（JSON/URL/query/请求头/curl 不能作为模型ID），请求已拦截。".to_string();
            let _ = app.emit(&format!("ai-token:{id}"), serde_json::json!({"error": &msg, "done": true}));
            return Err(msg);
        }
        let base = cfg.base_url.trim_end_matches('/');
        let base = if base.ends_with("/v1") { base.to_string() } else { format!("{base}/v1") };
        let url = format!("{base}/chat/completions");
        let b = ChatBody::OpenAi(OpenAiChatBody {
            model: cfg.model.clone(),
            messages: msgs,
            stream: true,
            temperature: 0.7,
        });
        (url, b, !cfg.api_key.is_empty())
    };

    let mut req = http.post(&final_url).timeout(std::time::Duration::from_secs(300));
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
            .or_else(|| parsed.pointer("/message/content"))
            .or_else(|| parsed.pointer("/content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if content.is_empty() {
            let brief: String = text.chars().take(300).collect();
            let msg = format!("响应协议不匹配：端点返回非流式响应（Content-Type: {ct}），且未包含可识别的正文字段。响应片段：{brief}");
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
                        // 200 但全程无正文字段 → 显式报错，绝不静默完成出空白回复
                        let msg = format!("响应协议不匹配：流结束前未找到正文字段 choices[].delta.content。响应片段：{}", buf.chars().take(200).collect::<String>());
                        emit_err(&app, &id, &msg);
                        return Err(msg);
                    }
                    emit_delta(&app, &id, "", true);
                    return Ok(());
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) =
                        v.pointer("/choices/0/delta/content").and_then(|c| c.as_str())
                    {
                        if !content.is_empty() {
                            got_any = true;
                        }
                        emit_delta(&app, &id, content, false);
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
}
