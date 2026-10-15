//! Provider-neutral OpenAI-compatible chat contract.
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

pub trait JsonTransport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
    ) -> Result<(u16, Value), LLMError>;
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
}

pub struct OpenAiCompatibleAdapter<T> {
    config: OpenAiCompatibleConfig,
    transport: T,
}

impl<T> OpenAiCompatibleAdapter<T> {
    pub fn new(config: OpenAiCompatibleConfig, transport: T) -> Self { Self { config, transport } }
    pub fn config(&self) -> &OpenAiCompatibleConfig { &self.config }

    fn headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        if let Some(key) = &self.config.api_key {
            headers.insert("authorization".into(), format!("Bearer {key}"));
        }
        headers
    }

    fn parse_response(&self, status: u16, raw: Value) -> Result<ChatResponse, LLMError> {
        if status == 401 || status == 403 { return Err(LLMError::Auth); }
        if status == 429 { return Err(LLMError::RateLimit { retry_after: 1.0 }); }
        if !(200..300).contains(&status) {
            return Err(LLMError::ApiError { status, message: raw.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("provider error").to_string() });
        }
        let choice = raw.get("choices").and_then(Value::as_array).and_then(|items| items.first()).ok_or_else(|| LLMError::ApiError { status, message: "missing choices".into() })?;
        let content = choice.get("message").and_then(|m| m.get("content")).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| LLMError::ApiError { status, message: "missing non-empty message.content".into() })?;
        let usage = raw.get("usage").map(|u| Usage {
            prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            total_tokens: u.get("total_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        }).unwrap_or_default();
        Ok(ChatResponse {
            content: content.to_string(),
            thinking: String::new(),
            model: raw.get("model").and_then(Value::as_str).unwrap_or(&self.config.model).to_string(),
            usage,
            raw,
            finish_reason: choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop").to_string(),
            thinking_tokens: 0,
            tool_calls: Vec::new(),
        })
    }
}

impl<T: JsonTransport> BaseLLMAdapter for OpenAiCompatibleAdapter<T> {
    fn name(&self) -> &str { &self.config.model }

    fn chat(&self, messages: &[Message]) -> Result<ChatResponse, LLMError> {
        let body = serde_json::to_value(ChatRequest { model: &self.config.model, messages, stream: false })
            .map_err(|error| LLMError::ApiError { status: 0, message: error.to_string() })?;
        let (status, raw) = self.transport.post_json(&self.config.endpoint, &self.headers(), &body, self.config.timeout_ms)?;
        self.parse_response(status, raw)
    }

    fn stream(&self, _messages: &[Message]) -> Result<Vec<String>, LLMError> {
        Err(LLMError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Transport { calls: Arc<Mutex<usize>>, status: u16, response: Value }

    impl JsonTransport for Transport {
        fn post_json(&self, _url: &str, _headers: &BTreeMap<String, String>, _body: &Value, _timeout_ms: u64) -> Result<(u16, Value), LLMError> {
            *self.calls.lock().unwrap() += 1;
            Ok((self.status, self.response.clone()))
        }
    }

    #[test]
    fn parses_chat_response() {
        let transport = Transport { calls: Arc::new(Mutex::new(0)), status: 200, response: serde_json::json!({"model":"m","choices":[{"message":{"content":"ok"}}]}) };
        let adapter = OpenAiCompatibleAdapter::new(OpenAiCompatibleConfig::new("https://example.test/chat", "m"), transport);
        assert_eq!(adapter.chat(&[Message::user("hello")]).unwrap().content, "ok");
    }

    #[test]
    fn maps_auth_and_rate_limit() {
        for status in [401, 429] {
            let transport = Transport { calls: Arc::new(Mutex::new(0)), status, response: serde_json::json!({}) };
            let adapter = OpenAiCompatibleAdapter::new(OpenAiCompatibleConfig::new("https://example.test/chat", "m"), transport);
            let error = adapter.chat(&[Message::user("hello")]).unwrap_err();
            match (status, error) {
                (401, LLMError::Auth) | (429, LLMError::RateLimit { .. }) => {}
                _ => panic!("unexpected provider error mapping"),
            }
        }
    }
}
