//! world_state — 万象 L0 核心真理源层
//!
//! Scope: world-state traits, abstract resource accounting, and deterministic identifiers.
//!
//! 三个子模块：
//! - [`conservation`]: an abstract four-pool resource ledger; its invariant is a bookkeeping rule, not a physical law
//! - [`world_trait`]：WorldDefinition trait + 仙侠/科幻/都市三种世界实现
//! - [`deterministic_id`]：prefix_{tick}_{hash/index} 公式，无 UUID/SystemTime，跨进程可复现

pub mod conservation;
pub mod world_trait;
pub mod deterministic_id;

pub use conservation::{Conserved, ConservationError, ConservationOp, ConservationLedger};
pub use world_trait::{WorldDefinition, PhysicsRules, CausalityRules, InteractionRules, GrowthSystem,
                       XianxiaWorld, ScifiWorld, UrbanWorld};
pub use deterministic_id::{deterministic_id, fnv1a32};
