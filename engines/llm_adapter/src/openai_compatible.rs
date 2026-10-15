//! OpenAI-compatible provider contract without binding the core crate to a network runtime.
//!
//! The adapter owns request/response semantics and delegates HTTP to a small injected
//! transport. This makes provider behavior testable in CI while allowing applications to
//! supply reqwest, ureq, hyper, or another HTTP implementation without changing the LLM API.

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
        Self {
            endpoint: endpoint.into(),
            api_key: None,
            model: model.into(),
            timeout_ms: 30_000,
        }
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

/// Minimal HTTP boundary required by the provider adapter.
pub trait JsonTransport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
    ) -> Result<(u16, Value), LLMError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
}

/// Adapter for OpenAI-compatible `/chat/completions` APIs.
pub struct OpenAiCompatibleAdapter<T> {
    config: OpenAiCompatibleConfig,
    transport: T,
}

impl<T> OpenAiCompatibleAdapter<T> {
    pub fn new(config: OpenAiCompatibleConfig, transport: T) -> Self {
        Self { config, transport }
    }

    pub fn config(&self) -> &OpenAiCompatibleConfig {
        &self.config
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        if let Some(key) = &self.config.api_key {
            headers.insert("authorization".into(), format!("Bearer {key}"));
        }
        headers
    }

    fn parse_chat_response(&self, status: u16, raw: Value) -> Result<ChatResponse, LLMError> {
        if !(200..300).contains(&status) {
            if status == 401 || status == 403 {
                return Err(LLMError::Auth);
            }
            if status == 429 {
                return Err(LLMError::RateLimit { retry_after: 1.0 });
            }
            return Err(LLMError::ApiError {
                status,
                message: raw
                    .get("error")
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("provider returned an error")
                    .to_string(),
            });
        }

        let choice = raw
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .ok_or_else(|| LLMError::ApiError { status, message: "response has no choices".into() })?;
        let content = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| LLMError::ApiError { status, message: "first choice has no textual message.content".into() })?;
        if content.trim().is_empty() {
            return Err(LLMError::ApiError { status, message: "provider returned empty content".into() });
        }

        let usage = raw.get("usage").map(|u| Usage {
            prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            total_tokens: u.get("total_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        }).unwrap_or_default();
        let finish_reason = choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop");

        Ok(ChatResponse {
            content: content.to_string(),
            thinking: String::new(),
            model: raw.get("model").and_then(Value::as_str).unwrap_or(&self.config.model).to_string(),
            usage,
            raw,
            finish_reason: finish_reason.to_string(),
            thinking_tokens: 0,
            tool_calls: Vec::new(),
        })
    }
}

impl<T: JsonTransport> BaseLLMAdapter for OpenAiCompatibleAdapter<T> {
    fn name(&self) -> &str {
        &self.config.model
    }

    fn chat(&self, messages: &[Message]) -> Result<ChatResponse, LLMError> {
        let request = ChatRequest { model: &self.config.model, messages, stream: false };
        let body = serde_json::to_value(request)
            .map_err(|e| LLMError::ApiError { status: 0, message: format!("request serialization failed: {e}") })?;
        let (status, raw) = self.transport.post_json(&self.config.endpoint, &self.headers(), &body, self.config.timeout_ms)?;
        self.parse_chat_response(status, raw)
    }

    fn stream(&self, messages: &[Message]) -> Result<Vec<String>, LLMError> {
        let response = self.chat(messages)?;
        Ok(response.content.split_whitespace().map(str::to_string).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct RecordingTransport {
        calls: Arc<Mutex<Vec<(String, BTreeMap<String, String>, Value, u64)>>>,
        status: u16,
        response: Value,
    }

    impl JsonTransport for RecordingTransport {
        fn post_json(&self, url: &str, headers: &BTreeMap<String, String>, body: &Value, timeout_ms: u64) -> Result<(u16, Value), LLMError> {
            self.calls.lock().unwrap().push((url.into(), headers.clone(), body.clone(), timeout_ms));
            Ok((self.status, self.response.clone()))
        }
    }

    #[test]
    fn parses_openai_compatible_response_and_auth_header() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = RecordingTransport {
            calls: Arc::clone(&calls),
            status: 200,
            response: serde_json::json!({
                "model": "test-model",
                "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
            }),
        };
        let adapter = OpenAiCompatibleAdapter::new(
            OpenAiCompatibleConfig::new("https://example.test/v1/chat/completions", "test-model").with_api_key("secret"),
            transport,
        );
        let response = adapter.chat(&[Message::user("hi")]).unwrap();
        assert_eq!(response.content, "hello");
        assert_eq!(response.usage.total_tokens, 5);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.get("authorization").unwrap(), "Bearer secret");
        assert_eq!(calls[0].2["model"], "test-model");
        assert_eq!(calls[0].2["stream"], false);
    }

    #[test]
    fn maps_auth_and_rate_limit_statuses() {
        for (status, expected) in [(401, "auth"), (429, "rate")].iter() {
            let transport = RecordingTransport {
                calls: Arc::new(Mutex::new(Vec::new())),
                status: *status,
                response: serde_json::json!({"error": {"message": "blocked"}}),
            };
            let adapter = OpenAiCompatibleAdapter::new(OpenAiCompatibleConfig::new("http://test", "m"), transport);
            let err = adapter.chat(&[Message::user("hi")]).unwrap_err();
            match (expected, err) {
                ("auth", LLMError::Auth) | ("rate", LLMError::RateLimit { .. }) => {}
                _ => panic!("unexpected error mapping"),
            }
        }
    }
}
