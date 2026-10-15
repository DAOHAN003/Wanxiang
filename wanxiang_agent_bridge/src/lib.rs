//! End-to-end bridge from an LLM-planned Agent to the real seeded world simulator.
//!
//! The bridge exposes bounded, read-only and mutating world tools through ToolRegistry.
//! Tool execution is subject to ToolSecurityPolicy; no shell or filesystem tools are registered.

use agent_runtime::{
    AgentResult, AgentRunner, MemoryBackedLlmPlanner, RiskLevel, Tool, ToolRegistry,
};
use llm_adapter::BaseLLMAdapter;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use wanxiang_sdk::WorldModel;
use wanxiang_types::WorldRules;

const MAX_TICKS_PER_ACTION: u64 = 100;

/// Agent runtime whose registered tools operate on one real WorldModel.
pub struct WorldAgent<A: BaseLLMAdapter> {
    world: Arc<Mutex<WorldModel>>,
    runner: AgentRunner,
    planner: MemoryBackedLlmPlanner<A>,
    tools: ToolRegistry,
}

impl<A: BaseLLMAdapter> WorldAgent<A> {
    pub fn new(
        seed: u64,
        rules: WorldRules,
        planner: MemoryBackedLlmPlanner<A>,
        max_steps: u32,
    ) -> Self {
        let world = Arc::new(Mutex::new(WorldModel::new(rules, seed)));
        let mut tools = ToolRegistry::new();

        let info_world = Arc::clone(&world);
        tools.register(
            Tool::new("world_info", "Read current tick, counts, and conservation status", RiskLevel::Low),
            move |_| match info_world.lock() {
                Ok(world) => serde_json::from_str(&world.info_json()).unwrap_or(Value::Null),
                Err(_) => json!({"error": "world lock poisoned"}),
            },
        );

        let snapshot_world = Arc::clone(&world);
        tools.register(
            Tool::new("world_snapshot", "Read the structured world snapshot", RiskLevel::Low),
            move |_| match snapshot_world.lock() {
                Ok(world) => serde_json::from_str(&world.snapshot_json()).unwrap_or(Value::Null),
                Err(_) => json!({"error": "world lock poisoned"}),
            },
        );

        let step_world = Arc::clone(&world);
        tools.register(
            Tool::new("world_step", "Advance the world by 1 to 100 ticks", RiskLevel::Medium),
            move |args| {
                let ticks = match args.get("ticks").and_then(Value::as_u64) {
                    Some(ticks) if (1..=MAX_TICKS_PER_ACTION).contains(&ticks) => ticks,
                    _ => return json!({"error": "ticks must be an integer in 1..=100"}),
                };
                match step_world.lock() {
                    Ok(mut world) => {
                        world.step(ticks);
                        json!({
                            "advanced_ticks": ticks,
                            "world": serde_json::from_str::<Value>(&world.info_json()).unwrap_or(Value::Null)
                        })
                    }
                    Err(_) => json!({"error": "world lock poisoned"}),
                }
            },
        );

        Self {
            world,
            runner: AgentRunner::new(max_steps),
            planner,
            tools,
        }
    }

    /// Run one bounded Agent task against the actual simulation.
    pub fn run(&mut self, user_input: &str) -> AgentResult {
        self.runner.run_with_tools(user_input, &mut self.tools, &mut self.planner)
    }

    /// Inspect simulation state without going through the planner.
    pub fn world_info(&self) -> Value {
        match self.world.lock() {
            Ok(world) => serde_json::from_str(&world.info_json()).unwrap_or(Value::Null),
            Err(_) => json!({"error": "world lock poisoned"}),
        }
    }

    pub fn trace(&self, session_id: &str) -> Vec<agent_runtime::AgentStep> {
        self.runner.trace_store.get_trace(session_id)
    }

    pub fn planner(&self) -> &MemoryBackedLlmPlanner<A> {
        &self.planner
    }

    pub fn planner_mut(&mut self) -> &mut MemoryBackedLlmPlanner<A> {
        &mut self.planner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::{memory_with_causal_graph, CausalGraph};
    use llm_adapter::{ChatResponse, LLMError, Message};
    use memory_core::{NmmUnified, SdrEncoder};
    use std::collections::VecDeque;

    struct ScriptedAdapter {
        responses: Mutex<VecDeque<String>>,
        prompts: Arc<Mutex<Vec<String>>>,
    }

    impl ScriptedAdapter {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(str::to_string).collect()),
                prompts: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl BaseLLMAdapter for ScriptedAdapter {
        fn name(&self) -> &str { "world-bridge-test-adapter" }

        fn chat(&self, messages: &[Message]) -> Result<ChatResponse, LLMError> {
            self.prompts.lock().unwrap().push(
                messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n")
            );
            let response = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
                LLMError::ApiError { status: 500, message: "script exhausted".to_string() }
            })?;
            Ok(ChatResponse::new(response, self.name()))
        }

        fn stream(&self, _messages: &[Message]) -> Result<Vec<String>, LLMError> {
            Ok(Vec::new())
        }
    }

    fn planner(adapter: ScriptedAdapter) -> MemoryBackedLlmPlanner<ScriptedAdapter> {
        let mut graph = CausalGraph::new();
        let goal = graph.add_node("goal");
        let result = graph.add_node("result");
        graph.add_edge(goal, result, 0.8);
        let mut memory = memory_with_causal_graph(graph);
        memory.store_memory(
            "The goal is to advance the world and inspect the result.",
            5.0,
            vec!["world".to_string()],
            Vec::new(),
            "test",
            None,
            Some(0.8),
        );
        MemoryBackedLlmPlanner::new(adapter, memory)
    }

    #[test]
    fn agent_controls_real_world_and_observes_feedback_in_next_decision() {
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"advance two ticks","tool_name":"world_step","arguments":{"ticks":2}}"#,
            r#"{"thought":"inspect current state","tool_name":"world_info","arguments":{}}"#,
            r#"{"thought":"state was inspected","tool_name":null,"arguments":null}"#,
        ]);
        let prompts = Arc::clone(&adapter.prompts);
        let mut agent = WorldAgent::new(123, WorldRules::default(), planner(adapter), 5);

        let result = agent.run("goal");

        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.steps.len(), 3);
        assert_eq!(result.steps[0].action, "world_step");
        assert!(result.steps[0].observation.contains("\"tick\":2"));
        assert_eq!(result.steps[1].action, "world_info");
        assert!(result.steps[1].observation.contains("\"tick\":2"));
        assert_eq!(agent.world_info()["tick"], 2);
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 3);
        assert!(prompts[0].contains("advance the world and inspect the result"));
        assert!(prompts[1].contains("\"tick\":2"));
        assert!(prompts[2].contains("world_info"));
        assert!(prompts[2].contains("\"tick\":2"));
    }

    #[test]
    fn invalid_world_step_arguments_do_not_advance_the_simulation() {
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"try invalid step","tool_name":"world_step","arguments":{"ticks":1000}}"#,
        ]);
        let mut agent = WorldAgent::new(123, WorldRules::default(), planner(adapter), 2);

        let result = agent.run("goal");

        assert!(!result.success);
        assert!(result.steps[0].observation.contains("ticks must be an integer"));
        assert_eq!(agent.world_info()["tick"], 0);
    }

    #[test]
    fn memory_store_helper_works_without_causal_graph_too() {
        let memory = NmmUnified::new(SdrEncoder::default());
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"finish","tool_name":null,"arguments":null}"#,
        ]);
        let planner = MemoryBackedLlmPlanner::new(adapter, memory);
        let mut agent = WorldAgent::new(7, WorldRules::default(), planner, 2);
        let result = agent.run("simple task");
        assert!(result.success);
    }
}
