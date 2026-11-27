//! 激活引擎：直接向供应商 API 发送最小请求（max_tokens=1 的 "hi"）。
//! 注意：只有上一窗口已过期时，请求才会点燃新的五小时窗口；窗口内的请求
//! 只会被算进旧窗口（HTTP 200 但不点亮新窗口），定时触发据此推迟发送
//! （scheduler::scheduled_delay_ms）。额度查询在 quota.rs。

use crate::net_err;
use crate::store::ProviderConfig;
use std::sync::OnceLock;
use std::time::Duration;

/// 激活点燃的滚动窗口时长：5 小时（毫秒）。调度推迟与激活后的窗口确认均以此为基准。
pub(crate) const WINDOW_MS: i64 = 5 * 3600 * 1000;

pub(crate) fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build http client")
    })
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

fn trim_url(u: &str) -> String {
    u.trim().trim_end_matches('/').to_string()
}

/// Anthropic 兼容端点：base_url + /v1/messages。
/// 兼容三种写法：.../anthropic、.../v1、.../v1/messages（含尾部斜杠）。
pub fn messages_url(base_url: &str) -> String {
    let b = trim_url(base_url);
    if b.ends_with("/messages") {
        b
    } else if b.ends_with("/v1") {
        format!("{b}/messages")
    } else {
        format!("{b}/v1/messages")
    }
}

/// OpenAI 兼容端点：base_url + /v1/chat/completions。
pub fn chat_url(base_url: &str) -> String {
    let b = trim_url(base_url);
    if b.ends_with("/chat/completions") {
        b
    } else if b.ends_with("/v1") {
        format!("{b}/chat/completions")
    } else {
        format!("{b}/v1/chat/completions")
    }
}

/// 剥掉 cc-switch 的 1M 上下文标记后缀（"glm-5.3-flash[1M]" → "glm-5.3-flash"）。
/// 该标记只用于声明 1M 上下文档位，不是真实模型名，发给上游会报"模型不存在"。
fn strip_context_marker(model: &str) -> String {
    let m = model.trim();
    match m.get(m.len().saturating_sub(4)..) {
        Some(tail) if tail.eq_ignore_ascii_case("[1m]") => m[..m.len() - 4].trim().to_string(),
        _ => m.to_string(),
    }
}

/// 发送一次最小激活请求。HTTP 2xx 即视为窗口已激活。
pub async fn activate(p: &ProviderConfig) -> Result<String, String> {
    if p.api_key.trim().is_empty() {
        return Err("未配置 API Key".into());
    }
    let model = strip_context_marker(&p.model);
    if model.is_empty() {
        return Err("未配置模型名".into());
    }
    let (url, body) = match p.format.as_str() {
        "openai" => (
            chat_url(&p.base_url),
            serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "stream": false,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        ),
        _ => (
            messages_url(&p.base_url),
            serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        ),
    };

    let mut req = client().post(&url).json(&body);
    req = match p.format.as_str() {
        "openai" => req.bearer_auth(&p.api_key),
        // 同时携带 Bearer 与 x-api-key，兼容 AUTH_TOKEN 与 API_KEY 两种供应商
        _ => req
            .bearer_auth(&p.api_key)
            .header("x-api-key", &p.api_key)
            .header("anthropic-version", "2023-06-01"),
    };

    let started = std::time::Instant::now();
    let resp = req.send().await.map_err(net_err::describe)?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let ms = started.elapsed().as_millis();

    if status.is_success() {
        Ok(format!("HTTP {status}，耗时 {ms}ms，窗口已激活"))
    } else {
        Err(format!(
            "HTTP {status}，耗时 {ms}ms，响应：{}",
            truncate(&text, 300)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_context_marker() {
        assert_eq!(strip_context_marker("glm-5.3-flash[1M]"), "glm-5.3-flash");
        assert_eq!(strip_context_marker("glm-5.3-flash[1m]"), "glm-5.3-flash");
        assert_eq!(strip_context_marker("glm-5.3-flash"), "glm-5.3-flash");
        assert_eq!(
            strip_context_marker("  glm-5.3-flash [1M] "),
            "glm-5.3-flash"
        );
        assert_eq!(strip_context_marker("[1m]"), "");
        // 多字节字符结尾不能 panic
        assert_eq!(strip_context_marker("模型名上下"), "模型名上下");
    }

    #[test]
    fn test_messages_url() {
        assert_eq!(
            messages_url("https://open.bigmodel.cn/api/anthropic"),
            "https://open.bigmodel.cn/api/anthropic/v1/messages"
        );
        assert_eq!(
            messages_url("https://api.kimi.com/coding/"),
            "https://api.kimi.com/coding/v1/messages"
        );
        assert_eq!(
            messages_url("https://x.example.com/v1/"),
            "https://x.example.com/v1/messages"
        );
        assert_eq!(
            messages_url("https://x.example.com/anthropic/v1/messages"),
            "https://x.example.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_chat_url() {
        assert_eq!(
            chat_url("https://api.deepseek.com/anthropic"),
            "https://api.deepseek.com/anthropic/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }
}
