//! Provider-neutral OpenAI-compatible request/response contract.
//!
//! The core crate deliberately does not choose an HTTP runtime. Applications inject a
//! `JsonTransport` implementation for their chosen networking stack.

use crate::base::{BaseLLMAdapter, ChatResponse, LLMError, Message, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiCompatibleConfig {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
    pub timeout_ms: u64,
}

impl OpenAiCompatibleConfig {
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into(), api_key: None, model: model.into(), timeout_ms: 30_000 }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiCompatibleRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
}

impl OpenAiCompatibleRequest {
    pub fn chat(config: &OpenAiCompatibleConfig, messages: &[Message]) -> Self {
        Self { model: config.model.clone(), messages: messages.to_vec(), stream: false }
    }

    pub fn stream(config: &OpenAiCompatibleConfig, messages: &[Message]) -> Self {
        Self { model: config.model.clone(), messages: messages.to_vec(), stream: true }
    }
}

/// Injected network boundary. Applications can implement this with any HTTP stack.
pub trait JsonTransport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
    ) -> Result<(u16, Value), LLMError>;

    /// Optional Server-Sent Events transport. JSON-only transports remain safely unsupported.
    fn post_sse(
        &self,
        _url: &str,
        _headers: &BTreeMap<String, String>,
        _body: &Value,
        _timeout_ms: u64,
    ) -> Result<(u16, String), LLMError> {
        Err(LLMError::NotImplemented)
    }
}

pub fn authorization_headers(config: &OpenAiCompatibleConfig) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    if let Some(key) = &config.api_key {
        headers.insert("authorization".into(), format!("Bearer {key}"));
    }
    headers
}

pub fn parse_chat_response(config: &OpenAiCompatibleConfig, status: u16, raw: Value) -> Result<ChatResponse, LLMError> {
    if status == 401 || status == 403 { return Err(LLMError::Auth); }
    if status == 429 { return Err(LLMError::RateLimit { retry_after: 1.0 }); }
    if !(200..300).contains(&status) {
        return Err(LLMError::ApiError { status, message: raw.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("provider error").to_string() });
    }
    let choice = raw.get("choices").and_then(Value::as_array).and_then(|items| items.first()).ok_or_else(|| LLMError::ApiError { status, message: "missing choices".into() })?;
    let content = choice.get("message").and_then(|m| m.get("content")).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| LLMError::ApiError { status, message: "missing non-empty message.content".into() })?;
    let model = raw.get("model").and_then(Value::as_str).unwrap_or(&config.model).to_string();
    let finish_reason = choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop").to_string();
    let usage = raw.get("usage").map(|u| Usage {
        prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        total_tokens: u.get("total_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
    }).unwrap_or_default();
    Ok(ChatResponse {
        content: content.to_string(),
        thinking: String::new(),
        model,
        usage,
        raw,
        finish_reason,
        thinking_tokens: 0,
        tool_calls: Vec::new(),
    })
}



/// Parse OpenAI-compatible text/event-stream deltas into content chunks.
pub fn parse_sse_response(
    config: &OpenAiCompatibleConfig,
    status: u16,
    raw: &str,
) -> Result<Vec<String>, LLMError> {
    if status == 401 || status == 403 { return Err(LLMError::Auth); }
    if status == 429 { return Err(LLMError::RateLimit { retry_after: 1.0 }); }
    if !(200..300).contains(&status) {
        let message = serde_json::from_str::<Value>(raw).ok()
            .and_then(|v| v.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| raw.trim().chars().take(500).collect());
        return Err(LLMError::ApiError { status, message });
    }

    let mut chunks = Vec::new();
    for line in raw.lines() {
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" { break; }
        let value: Value = serde_json::from_str(data)
            .map_err(|e| LLMError::ApiError { status, message: format!("malformed SSE data: {e}") })?;
        if let Some(error) = value.get("error") {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("provider stream error");
            return Err(LLMError::ApiError { status, message: message.to_string() });
        }
        if let Some(text) = value.get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            chunks.push(text.to_string());
        }
    }
    if chunks.is_empty() {
        return Err(LLMError::ApiError {
            status,
            message: format!("provider stream returned no content chunks for model {}", config.model),
        });
    }
    Ok(chunks)
}

/// OpenAI-compatible chat adapter over an application-provided JSON transport.
///
/// The adapter owns request shaping and response/error normalization. The embedding
/// application owns the actual HTTP client, TLS, connection pooling, and network policy.
pub struct OpenAiCompatibleAdapter<T: JsonTransport> {
    config: OpenAiCompatibleConfig,
    transport: T,
}

impl<T: JsonTransport> OpenAiCompatibleAdapter<T> {
    pub fn new(config: OpenAiCompatibleConfig, transport: T) -> Self {
        Self { config, transport }
    }

    pub fn config(&self) -> &OpenAiCompatibleConfig {
        &self.config
    }

    fn chat_completions_url(&self) -> String {
        let endpoint = self.config.endpoint.trim_end_matches('/');
        if endpoint.ends_with("/chat/completions") {
            endpoint.to_string()
        } else {
            format!("{endpoint}/chat/completions")
        }
    }
}

impl<T: JsonTransport> BaseLLMAdapter for OpenAiCompatibleAdapter<T> {
    fn name(&self) -> &str {
        &self.config.model
    }

    fn chat(&self, messages: &[Message]) -> Result<ChatResponse, LLMError> {
        let request = OpenAiCompatibleRequest::chat(&self.config, messages);
        let body = serde_json::to_value(request).map_err(|error| LLMError::ApiError {
            status: 400,
            message: format!("failed to serialize chat request: {error}"),
        })?;
        let (status, raw) = self.transport.post_json(
            &self.chat_completions_url(),
            &authorization_headers(&self.config),
            &body,
            self.config.timeout_ms,
        )?;
        parse_chat_response(&self.config, status, raw)
    }

    fn stream(&self, messages: &[Message]) -> Result<Vec<String>, LLMError> {
        let request = OpenAiCompatibleRequest::stream(&self.config, messages);
        let body = serde_json::to_value(request).map_err(|error| LLMError::ApiError {
            status: 400,
            message: format!("failed to serialize stream request: {error}"),
        })?;
        let (status, raw) = self.transport.post_sse(
            &self.chat_completions_url(),
            &authorization_headers(&self.config),
            &body,
            self.config.timeout_ms,
        )?;
        parse_sse_response(&self.config, status, &raw)
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeTransport {
        calls: Mutex<Vec<(String, BTreeMap<String, String>, Value, u64)>>,
        response: Mutex<(u16, Value)>,
    }

    impl FakeTransport {
        fn new(status: u16, response: Value) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                response: Mutex::new((status, response)),
            }
        }
    }

    impl JsonTransport for FakeTransport {
        fn post_json(
            &self,
            url: &str,
            headers: &BTreeMap<String, String>,
            body: &Value,
            timeout_ms: u64,
        ) -> Result<(u16, Value), LLMError> {
            self.calls.lock().unwrap().push((
                url.to_string(),
                headers.clone(),
                body.clone(),
                timeout_ms,
            ));
            Ok(self.response.lock().unwrap().clone())
        }
    }

    fn success_body() -> Value {
        serde_json::json!({
            "model": "served-model",
            "choices": [{
                "message": {"role": "assistant", "content": "provider response"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
        })
    }

    #[test]
    fn adapter_builds_authenticated_request_and_normalizes_response() {
        let config = OpenAiCompatibleConfig::new(
            "https://provider.example/v1/",
            "requested-model",
        )
        .with_api_key("unit-test-key");
        let transport = FakeTransport::new(200, success_body());
        let adapter = OpenAiCompatibleAdapter::new(config, transport);

        let response = adapter.chat(&[Message::user("hello")]).unwrap();

        assert_eq!(adapter.name(), "requested-model");
        assert_eq!(response.content, "provider response");
        assert_eq!(response.model, "served-model");
        assert_eq!(response.usage.total_tokens, 5);
        let calls = adapter.transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://provider.example/v1/chat/completions");
        assert_eq!(calls[0].1.get("authorization").unwrap(), "Bearer unit-test-key");
        assert_eq!(calls[0].1.get("content-type").unwrap(), "application/json");
        assert_eq!(calls[0].2["model"], "requested-model");
        assert_eq!(calls[0].2["stream"], false);
        assert_eq!(calls[0].2["messages"][0]["content"], "hello");
        assert_eq!(calls[0].3, 30_000);
    }

    #[test]
    fn adapter_accepts_an_endpoint_that_already_contains_chat_completions() {
        let config = OpenAiCompatibleConfig::new(
            "https://provider.example/v1/chat/completions",
            "test-model",
        );
        let transport = FakeTransport::new(200, success_body());
        let adapter = OpenAiCompatibleAdapter::new(config, transport);
        adapter.chat(&[Message::user("hello")]).unwrap();
        let calls = adapter.transport.calls.lock().unwrap();
        assert_eq!(calls[0].0, "https://provider.example/v1/chat/completions");
    }

    #[test]
    fn provider_auth_rate_limit_and_malformed_payload_fail_closed() {
        let config = OpenAiCompatibleConfig::new("https://provider.example/v1", "model");
        assert!(matches!(
            parse_chat_response(&config, 401, serde_json::json!({"error":{"message":"bad key"}})),
            Err(LLMError::Auth)
        ));
        assert!(matches!(
            parse_chat_response(&config, 429, serde_json::json!({"error":{"message":"slow down"}})),
            Err(LLMError::RateLimit { .. })
        ));
        assert!(matches!(
            parse_chat_response(&config, 200, serde_json::json!({"choices":[]})),
            Err(LLMError::ApiError { status: 200, .. })
        ));
    }

    struct FakeSseTransport {
        response: Mutex<(u16, String)>,
        calls: Mutex<Vec<(String, BTreeMap<String, String>, Value, u64)>>,
    }

    impl JsonTransport for FakeSseTransport {
        fn post_json(&self, _: &str, _: &BTreeMap<String, String>, _: &Value, _: u64)
            -> Result<(u16, Value), LLMError> {
            Err(LLMError::NotImplemented)
        }

        fn post_sse(&self, url: &str, headers: &BTreeMap<String, String>, body: &Value, timeout_ms: u64)
            -> Result<(u16, String), LLMError> {
            self.calls.lock().unwrap().push((url.to_string(), headers.clone(), body.clone(), timeout_ms));
            Ok(self.response.lock().unwrap().clone())
        }
    }

    #[test]
    fn adapter_parses_sse_content_deltas_and_requests_stream_mode() {
        let raw = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let transport = FakeSseTransport {
            response: Mutex::new((200, raw.to_string())),
            calls: Mutex::new(Vec::new()),
        };
        let adapter = OpenAiCompatibleAdapter::new(
            OpenAiCompatibleConfig::new("https://provider.example/v1", "stream-model"),
            transport,
        );
        assert_eq!(adapter.stream(&[Message::user("say hello")]).unwrap(), vec!["hello ", "world"]);
        let calls = adapter.transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://provider.example/v1/chat/completions");
        assert_eq!(calls[0].2["stream"], true);
        assert_eq!(calls[0].2["messages"][0]["content"], "say hello");
    }

    #[test]
    fn sse_parser_rejects_auth_rate_limit_and_malformed_payloads() {
        let config = OpenAiCompatibleConfig::new("https://provider.example/v1", "model");
        assert!(matches!(parse_sse_response(&config, 401, "{}"), Err(LLMError::Auth)));
        assert!(matches!(parse_sse_response(&config, 429, "{}"), Err(LLMError::RateLimit { .. })));
        assert!(matches!(parse_sse_response(&config, 200, "data: not-json\n"), Err(LLMError::ApiError { .. })));
        assert!(matches!(parse_sse_response(&config, 200, "data: [DONE]\n"), Err(LLMError::ApiError { .. })));
    }
}
