//! causal_core — causal-graph and separation utilities
//!
//! Scope: graph-based separation utilities and simplified causal-analysis experiments.
//!
//! 四个子模块：
//! - [`graph`]：CausalGraph/CausalEdge/CausalNode + add_node/add_edge/find_node/intervene
//! - [`calculus`]：do-calculus 干预 + d-分离 + 后门准则 + 反事实（CausalGraph 扩展 impl）
//! - [`verifier`]：CausalClaim + CausalVerifier + VerifyResult，零 LLM token 幻觉检测
//! - [`abstraction`]: bounded permutation-based graph matching and cross-domain mapping experiments
//!
//! 设计原则：
//! - 零 LLM token：所有因果推理本地完成，不进 prompt
//! - 一条因果边代替 N 条事实：存储 Θ(V+E)，远小于事实库 Θ(N·L)
//! - Deterministic output does not by itself establish causal identifiability or empirical correctness.

pub mod graph;
pub mod calculus;
pub mod verifier;
pub mod abstraction;
pub mod immunity;

pub use graph::{CausalGraph, CausalNode, CausalEdge, CausalGraphError, VarKind};
pub use verifier::{CausalClaim, Direction, CausalVerifier, VerifyResult};
pub use abstraction::{CausalAbstractionDiscovery, AbstractionResult};
pub use immunity::{CausalImmunity, ErrorRecord, InterceptResult};

// causal_core already depends on world_state; re-exporting the definition boundary keeps
// higher layers from duplicating a second world model or adding another dependency edge.
pub use world_state::{
    CausalityRules, GrowthSystem, InteractionRules, PhysicsRules,
    WorldDefinition, XianxiaWorld, ScifiWorld, UrbanWorld,
};
pub use world_state::world_trait::GrowthCurve;
