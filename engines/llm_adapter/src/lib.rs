//! llm_adapter — 万象 L5 AI/LLM 适配层
//!
//! Scope: model adapter interfaces, cache/routing helpers, and multimodal metadata utilities.
//!
//! 三个子模块：
//! - [`base`]：BaseLLMAdapter 抽象基类 — 统一 chat/stream/embed 契约（LSP 里氏替换）
//!   ChatResponse（content+thinking+usage+tool_calls）；LLMError（Network/RateLimit/Timeout/ApiError/Auth）；
//!   ResponseCache（SHA256 key + LRU 淘汰）；指数退避 T(n)=T_base×2ⁿ×(1+jitter)；
//!   重试成功率 P=1-failure_rate^max_retries；流式 TTFT+n×TPOT
//! - [`router`]：LLMRouter 管理中枢 — primary→fallback[0]→fallback[1]→报错（进程不退出）
//!   智能路由 ≤500 tok→Fast / ≥2000 tok→Powerful / 中间→Primary；启动探测逐个 ping；
//!   串联可靠性 R=1-∏(1-Rᵢ)；causal_core 可选做级联失效 do-calculus 分析
//! - [`multimodal`]：Multimodal 跨模态 — 视觉 Token ceil(W/512)×ceil(H/512)×170+85；
//!   跨模态注意力 FLOPS=N_v×N_t×d×2；图像预处理等比缩放+居中裁剪；图像成本估算
//!
//! 设计原则：
//! - 复用 L0：[`router::LLMRouter`] 可选 `causal_core::CausalGraph` 做 do-calculus
//!   级联失效分析（干预 observed 节点，传播到其余模型），无图则独立假设 R=1-∏(1-Rᵢ)
//! - 无 pyo3：纯 Rust rlib+cdylib，遵循 L0 crate-type 双输出
//! - LSP 契约：所有适配器必须可替换 BaseLLMAdapter 而不破坏正确性
//!   ∀ adapter: chat(messages).content 非空（MockAdapter 验证）
//! - 数学确定性：
//!   - 退避 T(n) = T_base × 2ⁿ × (1+jitter)，jitter∈[0,0.5]
//!   - 重试成功率 P = 1 - failure_rate^max_retries（failure_rate=0.1, n=4 → 99.99%）
//!   - 串联可靠性 R = 1 - ∏(1-Rᵢ)（3×0.95 → 99.9875%）
//!   - 视觉 Token = ceil(W/512) × ceil(H/512) × 170 + 85
//!   - 跨模态注意力 FLOPS = N_v × N_t × d × 2
//!   - Token 估算 = max(字符数/3, 1)
//! - 静默降级：causal_graph 可选（None 时按独立假设计算级联失效）

pub mod base;
pub mod router;
pub mod multimodal;

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
