//! agent_runtime — 万象 L4 智能体工具层
//!
//! Scope: reusable Agent runtime, tool policy, skill matching, team workflow components,
//! serializable cross-request execution sessions, and deterministic research-stack integration.

pub mod runner;
pub mod planner;
pub mod session;
pub mod integration;
pub mod tools;
pub mod skills;
pub mod team;

pub use runner::{
    AgentStep, AgentResult, Frame, BrainDecision, BrainGuard, TaskSummary,
    AgentBrain, NoOpBrain, GrillMe, TraceStatistics, ExecutionTraceStore, AgentRunner,
    completion_probability, expected_steps, expected_latency, streaming_latency,
    ActionPlanner, PlannedAction,
};
pub use session::{ApprovalDecision, PendingApproval, ResumableSession, SessionStatus};
pub use integration::{ResearchLoop, WorldContext, default_world};
pub use tools::{
    RiskLevel, ApprovalMode, Tool, Inspection, ToolRegistry, ToolSecurityPolicy,
    mm1_wait, mm1_utilization, mm1_queue_length,
};
pub use skills::{
    SkillPrerequisites, SkillMetadata, Skill, PrereqCheckResult, SkillManager,
    tfidf_cosine, prerequisite_probability, parse_skill_md,
};
pub use team::{
    Role, TaskStatus, ProjectStatus, TeamMember, Task, Project, ConflictType,
    Conflict, Progress, TeamEngine,
    topological_sort, critical_path, gustafson_speedup, hungarian_assignment,
    info_sharing_efficiency,
};

pub use planner::{MemoryBackedLlmPlanner, memory_with_causal_graph};
