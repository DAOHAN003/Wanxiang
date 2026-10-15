//! Provider-neutral OpenAI-compatible chat contract.
//!
//! The core crate deliberately does not choose an HTTP runtime. Applications inject a
//! `JsonTransport` implementation and can map the normalized response into `BaseLLMAdapter`.

use crate::base::{ChatResponse, LLMError, Message, Usage};
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

/// Injected network boundary. A concrete application can implement this with any HTTP stack.
pub trait JsonTransport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
    ) -> Result<(u16, Value), LLMError>;
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
        return Err(LLMError::ApiError {
            status,
            message: raw.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("provider error").to_string(),
        });
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
        model: raw.get("model").and_then(Value::as_str).unwrap_or(&config.model).to_string(),
        usage,
        raw,
        finish_reason: choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop").to_string(),
        thinking_tokens: 0,
        tool_calls: Vec::new(),
    })
}
