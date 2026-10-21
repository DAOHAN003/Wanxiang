//! External-consumer tests for the bridge across the two Cargo workspaces.
//!
//! These tests intentionally live under `tests/` rather than inside `src/lib.rs`:
//! they compile against the bridge's public API as a downstream crate would.

use agent_runtime::{memory_with_causal_graph, MemoryBackedLlmPlanner};
use causal_core::CausalGraph;
use llm_adapter::{BaseLLMAdapter, ChatResponse, LLMError, Message};
use memory_core::NmmUnified;
use std::collections::VecDeque;
use std::sync::Mutex;
use wanxiang_agent_bridge::{run_rule_experiment, WorldAgent};
use wanxiang_sdk::WorldModel;
use wanxiang_types::WorldRules;

struct ScriptedAdapter {
    responses: Mutex<VecDeque<String>>,
}

impl ScriptedAdapter {
    fn new(responses: &[&str]) -> Self {
        Self {
            responses: Mutex::new(responses.iter().map(|response| (*response).to_string()).collect()),
        }
    }
}

impl BaseLLMAdapter for ScriptedAdapter {
    fn name(&self) -> &str {
        "cross-workspace-contract-test"
    }

    fn chat(&self, _messages: &[Message]) -> Result<ChatResponse, LLMError> {
        let response = self.responses.lock().expect("script mutex").pop_front().ok_or_else(|| {
            LLMError::ApiError {
                status: 500,
                message: "scripted adapter has no remaining responses".to_string(),
            }
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
    let outcome = graph.add_node("outcome");
    graph.add_edge(goal, outcome, 0.8);

    let mut memory: NmmUnified = memory_with_causal_graph(graph);
    memory.store_memory(
        "The task is to advance the simulated world and inspect the resulting state.",
        5.0,
        vec!["integration".to_string()],
        Vec::new(),
        "test",
        None,
        Some(0.8),
    );
    MemoryBackedLlmPlanner::new(adapter, memory)
}

#[test]
fn public_bridge_connects_engine_planner_to_sdk_world_and_returns_feedback() {
    let adapter = ScriptedAdapter::new(&[
        r#"{"thought":"advance two ticks","tool_name":"world_step","arguments":{"ticks":2}}"#,
        r#"{"thought":"inspect the state after advancing","tool_name":"world_info","arguments":{}}"#,
        r#"{"thought":"the task is complete","tool_name":null,"arguments":null}"#,
    ]);
    let mut agent = WorldAgent::new(42, WorldRules::default(), planner(adapter), 4);

    let result = agent.run("Advance the world by two ticks and verify its state.");

    assert!(result.success, "agent failed: {:?}", result.error);
    assert_eq!(result.steps.len(), 3);
    assert_eq!(result.steps[0].action, "world_step");
    assert!(result.steps[0].observation.contains(r#""tick":2"#));
    assert_eq!(result.steps[1].action, "world_info");
    assert!(result.steps[1].observation.contains(r#""tick":2"#));
    assert_eq!(result.steps[2].action, "FINISH");
    assert_eq!(agent.world_info()["tick"], 2);
    assert!(
        agent.planner().memory().len() >= 3,
        "tool observations should be written back to the engine memory component"
    );
}

#[test]
fn public_rule_experiment_runs_on_sdk_world_without_mutating_source() {
    let mut world = WorldModel::new(WorldRules::default(), 20261022);
    world.step(3);
    let before = world.snapshot_json();

    let report = run_rule_experiment(&world, "energy_density", 0.95, 5)
        .expect("supported rule experiment should succeed");

    assert_eq!(world.snapshot_json(), before, "the live source world must remain unchanged");
    assert_eq!(report["source_tick"], 3);
    assert_eq!(report["ticks_simulated_per_branch"], 5);
    assert_eq!(report["source_world_unchanged"], true);
    assert_eq!(report["baseline"]["meta"]["tick"], 8);
    assert_eq!(report["altered"]["meta"]["tick"], 8);
    assert_eq!(report["diff"]["rules_differ"], true);
}
