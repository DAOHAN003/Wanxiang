//! End-to-end integration of world-state, causal graph, memory retrieval, LLM planning,
//! and guarded tool execution.
//!
//! This module is deliberately deterministic in CI. Applications can replace the LLM adapter
//! with a real provider without changing the integration boundary.

use crate::{AgentResult, AgentRunner, MemoryBackedLlmPlanner, RiskLevel, Tool, ToolRegistry};
use causal_core::{CausalGraph, WorldDefinition, XianxiaWorld};
use llm_adapter::BaseLLMAdapter;
use memory_core::{NmmUnified, SdrEncoder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldContext {
    pub name: String,
    pub physics: causal_core::PhysicsRules,
    pub causality: causal_core::CausalityRules,
    pub interaction: causal_core::InteractionRules,
    pub initial_attributes: std::collections::HashMap<String, f64>,
}

impl WorldContext {
    pub fn from_definition(world: &dyn WorldDefinition) -> Self {
        Self {
            name: world.name().to_string(),
            physics: world.physics(),
            causality: world.causality(),
            interaction: world.interaction(),
            initial_attributes: world.initial_attributes(),
        }
    }
}

/// One deterministic composition boundary for the experimental research crates.
pub struct ResearchLoop<A: BaseLLMAdapter> {
    pub world: WorldContext,
    pub causal_graph: CausalGraph,
    pub planner: MemoryBackedLlmPlanner<A>,
    pub registry: ToolRegistry,
    pub runner: AgentRunner,
}

impl<A: BaseLLMAdapter> ResearchLoop<A> {
    pub fn new(adapter: A, world: impl WorldDefinition + 'static) -> Self {
        let world = WorldContext::from_definition(&world);
        let mut graph = CausalGraph::new();
        let goal = graph.add_node("goal");
        let result = graph.add_node("result");
        graph.add_edge(goal, result, 0.8);
        let memory = NmmUnified::new(SdrEncoder::default()).with_causal_graph(graph.clone());
        let planner = MemoryBackedLlmPlanner::new(adapter, memory).with_retrieval(5, 0.0, true);

        let world_snapshot = world.clone();
        let mut registry = ToolRegistry::new();
        registry.register(
            Tool::new("world_info", "Read the current deterministic world definition", RiskLevel::Low),
            move |_| serde_json::to_value(&world_snapshot).unwrap_or_else(|_| serde_json::json!({"error":"serialization failed"})),
        );

        Self {
            world,
            causal_graph: graph,
            planner,
            registry,
            runner: AgentRunner::new(8),
        }
    }

    pub fn run(&mut self, user_input: &str) -> AgentResult {
        self.runner.run_with_tools(user_input, &mut self.registry, &mut self.planner)
    }

    pub fn causal_preview(&self, value: f64) -> Option<Vec<(usize, f64)>> {
        self.causal_graph.find_node("goal").map(|node| self.causal_graph.intervene(node, value))
    }
}

pub fn default_world() -> XianxiaWorld {
    XianxiaWorld
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_adapter::{ChatResponse, LLMError, Message};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct ScriptedAdapter(Mutex<VecDeque<String>>);
    impl BaseLLMAdapter for ScriptedAdapter {
        fn name(&self) -> &str { "integration-script" }
        fn chat(&self, _: &[Message]) -> Result<ChatResponse, LLMError> {
            let response = self.0.lock().unwrap().pop_front().unwrap_or_else(|| {
                r#"{"thought":"finish","tool_name":null,"arguments":null}"#.to_string()
            });
            Ok(ChatResponse::new(response, self.name()))
        }
        fn stream(&self, _: &[Message]) -> Result<Vec<String>, LLMError> { Ok(Vec::new()) }
    }

    #[test]
    fn world_causal_memory_llm_and_tool_layers_execute_as_one_loop() {
        let adapter = ScriptedAdapter(Mutex::new(VecDeque::from([
            r#"{"thought":"inspect the world before answering","tool_name":"world_info","arguments":{}}"#.to_string(),
            r#"{"thought":"world observation is sufficient","tool_name":null,"arguments":null}"#.to_string(),
        ])));
        let mut loop_engine = ResearchLoop::new(adapter, default_world());
        let result = loop_engine.run("inspect the world");
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].action, "world_info");
        assert!(result.steps[0].observation.contains("xianxia"));
        assert_eq!(result.steps[1].action, "FINISH");
        assert!(loop_engine.planner.memory().len() >= 1);
        let effects = loop_engine.causal_preview(1.0).unwrap();
        assert!(effects.iter().any(|(node, value)| *node == 1 && (*value - 0.8).abs() < 1e-12));
    }
}
