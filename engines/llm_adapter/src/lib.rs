//! llm_adapter — 万象 L5 AI/LLM 适配层
//!
//! Scope: model adapter interfaces, cache/routing helpers, multimodal metadata utilities,
//! and provider-neutral OpenAI-compatible request/transport contracts.

pub mod base;
pub mod router;
pub mod multimodal;
pub mod openai_compatible;

pub use base::{
    Message, Usage, ToolCall, ChatResponse, LLMError, BaseLLMAdapter, MockAdapter,
    ResponseCache, backoff_delay, retry_success_probability, estimate_tokens,
    streaming_total_latency, ttft_estimate, tpot_estimate,
};
pub use router::{
    ModelTier, RouterConfig, LLMRouter, ProbeReport,
    system_reliability, cascade_failure_probability,
    SIMPLE_TASK_THRESHOLD, COMPLEX_TASK_THRESHOLD,
};
pub use multimodal::{
    Multimodal, VisionBackend, ImagePreprocess, PreprocessResult,
    vision_token_count, image_cost, cross_attention_flops, image_preprocess,
};
pub use openai_compatible::{
    JsonTransport, OpenAiCompatibleConfig, OpenAiCompatibleRequest,
    authorization_headers, parse_chat_response,
};
