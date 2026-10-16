//! Serializable, cross-request Agent tool sessions.
//!
//! `AgentRunner::run_with_approval` is intentionally synchronous. This module adds the
//! missing persistence boundary: a caller can serialize `ResumableSession`, return its
//! pending approval to a UI/API, and later resume with the same planner and registry.

use crate::runner::{ActionPlanner, AgentResult, AgentRunner, AgentStep};
use crate::tools::ToolRegistry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumableSession {
    pub session_id: String,
    pub user_input: String,
    pub brain_context: String,
    pub history: Vec<AgentStep>,
    pub pending_approval: Option<PendingApproval>,
    pub finished: bool,
    pub success: bool,
    pub error: Option<String>,
    pub max_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionStatus {
    Running,
    AwaitingApproval(PendingApproval),
    Finished { success: bool, answer: String },
}

impl ResumableSession {
    pub fn new(session_id: impl Into<String>, user_input: impl Into<String>, runner: &AgentRunner) -> Self {
        let user_input = user_input.into();
        let frame = runner.brain.start_task(&user_input, "", 0);
        let brain_context = runner.brain.build_prompt_context(&frame);
        Self {
            session_id: session_id.into(),
            user_input,
            brain_context,
            history: Vec::new(),
            pending_approval: None,
            finished: false,
            success: true,
            error: None,
            max_steps: runner.max_steps,
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.finished
    }

    pub fn pending(&self) -> Option<&PendingApproval> {
        self.pending_approval.as_ref()
    }

    /// Execute one or more bounded steps. If a high-risk tool requires approval,
    /// execution stops before the tool runs and the returned state is safe to persist.
    pub fn drive<P: ActionPlanner>(
        &mut self,
        runner: &mut AgentRunner,
        registry: &mut ToolRegistry,
        planner: &mut P,
        approval: Option<ApprovalDecision>,
    ) -> SessionStatus {
        if self.finished {
            return SessionStatus::Finished {
                success: self.success,
                answer: self.answer(),
            };
        }

        if let Some(pending) = self.pending_approval.clone() {
            match approval {
                Some(ApprovalDecision::Approve) => {
                    let result = registry.execute_with_approval(&pending.tool_name, &pending.arguments, true);
                    let observation = result.to_string();
                    let step_num = self.history.len() as u32 + 1;
                    let success = result.get("error").is_none() && !result.get("pending").and_then(|v| v.as_bool()).unwrap_or(false);
                    let mut step = AgentStep::new(step_num, "resumed after approval", &pending.tool_name, &observation, success);
                    step.action_input = pending.arguments.clone();
                    runner.brain.after_tool(&pending.tool_name, &observation, success);
                    planner.observe_step(&self.user_input, &step);
                    runner.trace_store.record_step(&self.session_id, step.clone());
                    self.history.push(step);
                    self.pending_approval = None;
                    if !success {
                        return self.fail(format!("approved tool {} failed: {}", pending.tool_name, observation));
                    }
                }
                Some(ApprovalDecision::Reject) => {
                    self.pending_approval = None;
                    return self.fail(format!("approval rejected for tool {}", pending.tool_name));
                }
                None => return SessionStatus::AwaitingApproval(pending),
            }
        }

        let available_tools = registry.generate_prompt_descriptions();
        while (self.history.len() as u32) < self.max_steps {
            if runner.brain.before_llm(&self.history).should_stop {
                self.finished = true;
                self.success = true;
                return SessionStatus::Finished { success: true, answer: self.answer() };
            }

            let planned = planner.next_action(&self.user_input, &self.brain_context, &self.history, &available_tools);
            if let Some(error) = planned.error {
                return self.fail(format!("planner failed: {error}"));
            }
            let action_name = planned.tool_name.clone().unwrap_or_else(|| "FINISH".to_string());
            let guard = runner.brain.after_llm(&planned.thought, "", &planned.thought, &action_name);
            if !guard.accept {
                return self.fail(format!("Brain guard rejected action: {action_name}"));
            }

            let Some(tool_name) = planned.tool_name else {
                let step_num = self.history.len() as u32 + 1;
                let step = AgentStep::new(step_num, &planned.thought, "FINISH", "Agent finished.", true);
                runner.brain.after_tool("FINISH", "Agent finished.", true);
                runner.trace_store.record_step(&self.session_id, step.clone());
                self.history.push(step);
                self.finished = true;
                self.success = true;
                return SessionStatus::Finished { success: true, answer: self.answer() };
            };

            let mut result = registry.execute(&tool_name, &planned.arguments);
            let pending = result.get("pending").and_then(|v| v.as_bool()).unwrap_or(false);
            if pending {
                let pending = PendingApproval { tool_name, arguments: planned.arguments };
                self.pending_approval = Some(pending.clone());
                return SessionStatus::AwaitingApproval(pending);
            }
            let observation = result.to_string();
            let step_num = self.history.len() as u32 + 1;
            let success = result.get("error").is_none();
            let mut step = AgentStep::new(step_num, &planned.thought, &tool_name, &observation, success);
            step.action_input = planned.arguments;
            runner.brain.after_tool(&tool_name, &observation, success);
            planner.observe_step(&self.user_input, &step);
            runner.trace_store.record_step(&self.session_id, step.clone());
            self.history.push(step);
            if !success {
                return self.fail(format!("tool {tool_name} failed: {observation}"));
            }
            result = serde_json::Value::Null;
            let _ = result;
        }

        self.fail(format!("maximum step count ({}) reached before FINISH", self.max_steps))
    }

    pub fn to_result(&self) -> AgentResult {
        AgentResult {
            answer: self.answer(),
            steps: self.history.clone(),
            total_tokens: 0,
            duration_s: 0.0,
            success: self.success && self.finished,
            error: self.error.clone(),
        }
    }

    fn answer(&self) -> String {
        if self.success && self.finished {
            format!("Completed in {} steps", self.history.len())
        } else {
            self.error.clone().unwrap_or_else(|| "Agent session awaiting approval".to_string())
        }
    }

    fn fail(&mut self, error: String) -> SessionStatus {
        self.finished = true;
        self.success = false;
        self.error = Some(error.clone());
        SessionStatus::Finished { success: false, answer: error }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlannedAction, RiskLevel, Tool};
    use std::collections::VecDeque;

    struct Planner(VecDeque<PlannedAction>);
    impl ActionPlanner for Planner {
        fn next_action(&mut self, _: &str, _: &str, _: &[AgentStep], _: &str) -> PlannedAction {
            self.0.pop_front().unwrap_or_else(PlannedAction::finish)
        }
    }

    #[test]
    fn approval_session_survives_serialization_boundary_and_resumes() {
        let mut registry = ToolRegistry::new();
        registry.register(Tool::new("write_note", "write", RiskLevel::High), |args| {
            serde_json::json!({"written": args["content"]})
        });
        let mut runner = AgentRunner::new(3);
        let mut planner = Planner(VecDeque::from([
            PlannedAction::call("write_note", serde_json::json!({"content":"approved"})),
            PlannedAction::finish(),
        ]));
        let mut session = ResumableSession::new("session-1", "write a note", &runner);

        let pending = session.drive(&mut runner, &mut registry, &mut planner, None);
        assert!(matches!(pending, SessionStatus::AwaitingApproval(_)));
        let wire = serde_json::to_string(&session).unwrap();
        let mut restored: ResumableSession = serde_json::from_str(&wire).unwrap();

        let finished = restored.drive(&mut runner, &mut registry, &mut planner, Some(ApprovalDecision::Approve));
        assert!(matches!(finished, SessionStatus::Finished { success: true, .. }));
        assert_eq!(restored.to_result().steps.len(), 2);
    }
}
