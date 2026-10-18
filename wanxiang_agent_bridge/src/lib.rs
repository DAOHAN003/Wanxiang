//! End-to-end bridge from an LLM-planned Agent to the real seeded world simulator.
//!
//! The bridge exposes bounded, read-only and mutating world tools through ToolRegistry.
//! Tool execution is subject to ToolSecurityPolicy; no shell or filesystem tools are registered.

use agent_runtime::{
    ApprovalDecision, AgentResult, AgentRunner, MemoryBackedLlmPlanner, ResumableSession,
    RiskLevel, SessionStatus, Tool, ToolRegistry,
};
use llm_adapter::BaseLLMAdapter;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use wanxiang_sdk::WorldModel;
use wanxiang_types::WorldRules;

const MAX_TICKS_PER_ACTION: u64 = 100;
const MAX_ENTITY_SAMPLE: usize = 10;
const MAX_FACTION_SAMPLE: usize = 10;
const MAX_EVENT_SAMPLE: usize = 10;

fn summarize_array(value: Option<&Value>, sample_size: usize, from_end: bool) -> Value {
    let items = value.and_then(Value::as_array).cloned().unwrap_or_default();
    let count = items.len();
    let sample: Vec<Value> = if from_end {
        items.into_iter().skip(count.saturating_sub(sample_size)).collect()
    } else {
        items.into_iter().take(sample_size).collect()
    };
    json!({"count": count, "sample": sample, "truncated": count > sample_size})
}


/// Run a bounded counterfactual branch without mutating the live world.
/// Values are restricted to normalized public tuning knobs to keep the tool surface safe.
pub fn run_rule_experiment(
    world: &WorldModel,
    rule: &str,
    value: f64,
    ticks: u64,
) -> Result<Value, String> {
    if !matches!(
        rule,
        "energy_density" | "resource_abundance" | "chaos_level" | "order_level" | "life_fertility"
    ) {
        return Err("unsupported rule; choose energy_density, resource_abundance, chaos_level, order_level, or life_fertility".into());
    }
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err("value must be a finite number in 0..=1".into());
    }
    if !(1..=1_000).contains(&ticks) {
        return Err("ticks must be an integer in 1..=1000".into());
    }

    let start_tick = world.current_tick();
    let before: Value = serde_json::from_str(&world.snapshot_json())
        .map_err(|e| format!("could not read source snapshot: {e}"))?;
    let previous_value = before.get("rules")
        .and_then(|r| r.get(rule))
        .and_then(Value::as_f64)
        .ok_or_else(|| "selected rule is not exposed as a numeric public tuning knob".to_string())?;

    let mut baseline = world.fork();
    let mut altered = world.branch(|rules| match rule {
        "energy_density" => rules.energy_density = value,
        "resource_abundance" => rules.resource_abundance = value,
        "chaos_level" => rules.chaos_level = value,
        "order_level" => rules.order_level = value,
        "life_fertility" => rules.life_fertility = value,
        _ => unreachable!("rule whitelist validated above"),
    });
    baseline.step(ticks);
    altered.step(ticks);

    let baseline_json: Value = serde_json::from_str(&baseline.snapshot_json())
        .map_err(|e| format!("could not serialize baseline branch: {e}"))?;
    let altered_json: Value = serde_json::from_str(&altered.snapshot_json())
        .map_err(|e| format!("could not serialize altered branch: {e}"))?;
    let diff: Value = serde_json::from_str(&baseline.diff_json(&altered))
        .map_err(|e| format!("could not serialize world diff: {e}"))?;

    Ok(json!({
        "kind": "world_rule_experiment",
        "source_tick": start_tick,
        "ticks_simulated_per_branch": ticks,
        "changed_rule": rule,
        "previous_value": previous_value,
        "altered_value": value,
        "baseline": baseline_json,
        "altered": altered_json,
        "diff": diff,
        "source_world_unchanged": world.current_tick() == start_tick,
        "interpretation": "This is a controlled intervention inside the Wanxiang simulation model, not evidence of real-world causality."
    }))
}

fn bounded_snapshot(snapshot: Value) -> Value {
    json!({
        "meta": snapshot.get("meta").cloned().unwrap_or(Value::Null),
        "entities": summarize_array(snapshot.get("entities"), MAX_ENTITY_SAMPLE, false),
        "factions": summarize_array(snapshot.get("factions"), MAX_FACTION_SAMPLE, false),
        "events": summarize_array(snapshot.get("events"), MAX_EVENT_SAMPLE, true),
        "resources": snapshot.get("resources").cloned().unwrap_or(Value::Null),
        "rules": snapshot.get("rules").cloned().unwrap_or(Value::Null),
        "bounded": true
    })
}

/// Agent runtime whose registered tools operate on one real WorldModel.
pub struct WorldAgent<A: BaseLLMAdapter> {
    world: Arc<Mutex<WorldModel>>,
    runner: AgentRunner,
    planner: MemoryBackedLlmPlanner<A>,
    tools: ToolRegistry,
    run_count: u64,
    last_session_id: Option<String>,
}

impl<A: BaseLLMAdapter> WorldAgent<A> {
    pub fn new(
        seed: u64,
        rules: WorldRules,
        planner: MemoryBackedLlmPlanner<A>,
        max_steps: u32,
    ) -> Self {
        Self::with_shared_world(Arc::new(Mutex::new(WorldModel::new(rules, seed))), planner, max_steps)
    }

    /// Construct an Agent bridge over a world shared with another application component.
    pub fn with_shared_world(
        world: Arc<Mutex<WorldModel>>,
        planner: MemoryBackedLlmPlanner<A>,
        max_steps: u32,
    ) -> Self {
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
                Ok(world) => bounded_snapshot(serde_json::from_str(&world.snapshot_json()).unwrap_or(Value::Null)),
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


        let move_world = Arc::clone(&world);
        tools.register(
            Tool::new(
                "world_move_character",
                "Move a living character to a directly connected authored location; arguments: {character_id,destination_id}",
                RiskLevel::Medium,
            ),
            move |args| {
                let character_id = match args.get("character_id").and_then(Value::as_str) {
                    Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                    _ => return json!({"error":"character_id must be a non-empty string of at most 128 characters"}),
                };
                let destination_id = match args.get("destination_id").and_then(Value::as_str) {
                    Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                    _ => return json!({"error":"destination_id must be a non-empty string of at most 128 characters"}),
                };
                match move_world.lock() {
                    Ok(mut world) => match world.move_character(character_id, destination_id) {
                        Ok(()) => json!({
                            "moved":true,
                            "character_id":character_id,
                            "destination_id":destination_id,
                            "tick":world.current_tick(),
                            "snapshot":bounded_snapshot(serde_json::from_str(&world.snapshot_json()).unwrap_or(Value::Null))
                        }),
                        Err(error) => json!({"error":error,"moved":false}),
                    },
                    Err(_) => json!({"error":"world lock poisoned","moved":false}),
                }
            },
        );

        let plan_world = Arc::clone(&world);
        tools.register(
            Tool::new(
                "world_plan_travel",
                "Read-only forecast of a character journey: authored path, model cost, estimated ticks, exposure risk, predicted encounters and supply affordability; arguments: {character_id,destination_id}",
                RiskLevel::Low,
            ),
            move |args| {
                let character_id = match args.get("character_id").and_then(Value::as_str) {
                    Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                    _ => return json!({"error":"character_id must be a non-empty string of at most 128 characters"}),
                };
                let destination_id = match args.get("destination_id").and_then(Value::as_str) {
                    Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                    _ => return json!({"error":"destination_id must be a non-empty string of at most 128 characters"}),
                };
                match plan_world.lock() {
                    Ok(world) => match world.plan_travel_character(character_id, destination_id) {
                        Ok(plan) => json!({"planned":true,"tick":world.current_tick(),"plan":plan}),
                        Err(error) => json!({"planned":false,"error":error}),
                    },
                    Err(_) => json!({"planned":false,"error":"world lock poisoned"}),
                }
            },
        );

        let travel_world = Arc::clone(&world);
        tools.register(
            Tool::new(
                "world_travel_character",
                "Execute a simulated journey along the planned authored route; advances world time, spends supplies, records encounters and returns a travel report; arguments: {character_id,destination_id}",
                RiskLevel::Medium,
            ),
            move |args| {
                let character_id = match args.get("character_id").and_then(Value::as_str) {
                    Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                    _ => return json!({"error":"character_id must be a non-empty string of at most 128 characters"}),
                };
                let destination_id = match args.get("destination_id").and_then(Value::as_str) {
                    Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                    _ => return json!({"error":"destination_id must be a non-empty string of at most 128 characters"}),
                };
                match travel_world.lock() {
                    Ok(mut world) => match world.simulate_travel_character(character_id, destination_id) {
                        Ok(report) => {
                            let travel = serde_json::to_value(&report).unwrap_or(Value::Null);
                            json!({
                                "moved":report.completed,
                                "character_id":character_id,
                                "destination_id":destination_id,
                                "path":report.path,
                                "route_cost":report.route_cost,
                                "travel":travel,
                                "tick":world.current_tick(),
                                "snapshot":bounded_snapshot(serde_json::from_str(&world.snapshot_json()).unwrap_or(Value::Null))
                            })
                        },
                        Err(error) => json!({"error":error,"moved":false}),
                    },
                    Err(_) => json!({"error":"world lock poisoned","moved":false}),
                }
            },
        );

        let experiment_world = Arc::clone(&world);
        tools.register(
            Tool::new(
                "world_experiment",
                "Compare baseline and one safe rule intervention without mutating the live world; arguments: {rule,value,ticks}",
                RiskLevel::Medium,
            ),
            move |args| {
                let rule = match args.get("rule").and_then(Value::as_str) {
                    Some(rule) => rule,
                    None => return json!({"error":"rule must be a string"}),
                };
                let value = match args.get("value").and_then(Value::as_f64) {
                    Some(value) => value,
                    None => return json!({"error":"value must be a number"}),
                };
                let ticks = match args.get("ticks").and_then(Value::as_u64) {
                    Some(ticks) => ticks,
                    None => return json!({"error":"ticks must be an integer"}),
                };
                match experiment_world.lock() {
                    Ok(world_guard) => {
                        let source_snapshot = world_guard.fork();
                        drop(world_guard);
                        match run_rule_experiment(&source_snapshot, rule, value, ticks) {
                            Ok(mut result) => {
                                if let Some(base) = result.get("baseline").cloned() {
                                    result["baseline"] = bounded_snapshot(base);
                                }
                                if let Some(altered) = result.get("altered").cloned() {
                                    result["altered"] = bounded_snapshot(altered);
                                }
                                result
                            }
                            Err(error) => json!({"error": error}),
                        }
                    }
                    Err(_) => json!({"error":"world lock poisoned"}),
                }
            },
        );

        Self {
            world,
            runner: AgentRunner::new(max_steps),
            planner,
            tools,
            run_count: 0,
            last_session_id: None,
        }
    }

    /// Run one bounded Agent task against the actual simulation.
    pub fn run(&mut self, user_input: &str) -> AgentResult {
        self.run_count += 1;
        self.last_session_id = Some(format!("tool_session_{}_{}", self.run_count, user_input.len()));
        self.runner.run_with_tools(user_input, &mut self.tools, &mut self.planner)
    }

    /// Start a serializable Agent session that can be checkpointed between HTTP requests.
    pub fn start_session(
        &self,
        session_id: impl Into<String>,
        user_input: impl Into<String>,
    ) -> ResumableSession {
        ResumableSession::new(session_id, user_input, &self.runner)
    }

    /// Drive a persisted session until it finishes or pauses for approval.
    pub fn drive_session(
        &mut self,
        session: &mut ResumableSession,
        approval: Option<ApprovalDecision>,
    ) -> SessionStatus {
        session.drive(&mut self.runner, &mut self.tools, &mut self.planner, approval)
    }

    /// Inspect simulation state without going through the planner.
    pub fn world_info(&self) -> Value {
        match self.world.lock() {
            Ok(world) => serde_json::from_str(&world.info_json()).unwrap_or(Value::Null),
            Err(_) => json!({"error": "world lock poisoned"}),
        }
    }

    pub fn last_trace(&self) -> Vec<agent_runtime::AgentStep> {
        self.last_session_id.as_deref()
            .map(|session_id| self.runner.trace_store.get_trace(session_id))
            .unwrap_or_default()
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
    use agent_runtime::memory_with_causal_graph;
    use causal_core::CausalGraph;
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
            r#"{"thought":"inspect bounded snapshot","tool_name":"world_snapshot","arguments":{}}"#,
            r#"{"thought":"inspect current state","tool_name":"world_info","arguments":{}}"#,
            r#"{"thought":"state was inspected","tool_name":null,"arguments":null}"#,
        ]);
        let prompts = Arc::clone(&adapter.prompts);
        let mut agent = WorldAgent::new(123, WorldRules::default(), planner(adapter), 5);

        let result = agent.run("goal");

        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.steps.len(), 4);
        assert_eq!(agent.last_trace().len(), 4);
        assert_eq!(agent.planner().memory().len(), 4, "each executed world tool should be written to memory");
        assert_eq!(result.steps[0].action, "world_step");
        assert!(result.steps[0].observation.contains("\"tick\":2"));
        assert_eq!(result.steps[1].action, "world_snapshot");
        assert!(result.steps[1].observation.contains("\"bounded\":true"));
        assert!(result.steps[1].observation.contains("\"entities\":{"));
        assert_eq!(result.steps[2].action, "world_info");
        assert!(result.steps[2].observation.contains("\"tick\":2"));
        assert_eq!(agent.world_info()["tick"], 2);
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 4);
        assert!(prompts[0].contains("advance the world and inspect the result"));
        assert!(prompts[1].contains("\"tick\":2"));
        assert!(prompts[1].contains("Tool: world_step"));
        assert!(prompts[2].contains("world_snapshot"));
        assert!(prompts[2].contains("Tool: world_step"));
        assert!(prompts[2].contains("\"bounded\":true"));
        assert!(prompts[3].contains("world_info"));
        assert!(prompts[3].contains("\"tick\":2"));
    }

    #[test]
    fn agent_can_move_a_character_over_an_authored_route_and_observe_it() {
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"move Tao through the connected route","tool_name":"world_move_character","arguments":{"character_id":"char_tao","destination_id":"loc_abyss"}}"#,
            r#"{"thought":"confirm resident state","tool_name":"world_snapshot","arguments":{}}"#,
            r#"{"thought":"movement verified","tool_name":null,"arguments":null}"#,
        ]);
        let shared = Arc::new(Mutex::new(WorldModel::new_shitian(20301017)));
        let mut agent = WorldAgent::with_shared_world(shared.clone(), planner(adapter), 4);
        let result = agent.run("Move char_tao to loc_abyss, then verify the snapshot.");

        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.steps[0].action, "world_move_character");
        assert!(result.steps[0].observation.contains(r#""moved":true"#));
        assert!(result.steps[0].observation.contains("loc_abyss"));
        assert_eq!(result.steps[1].action, "world_snapshot");
        assert!(result.steps[1].observation.contains("loc_abyss"));
        let world = shared.lock().unwrap();
        let snapshot: Value = serde_json::from_str(&world.snapshot_json()).unwrap();
        assert_eq!(
            snapshot["entities"].as_array().unwrap().iter()
                .find(|e| e["id"] == "char_tao").unwrap()["location_id"],
            "loc_abyss"
        );
        assert!(snapshot["events"].as_array().unwrap().iter()
            .any(|e| e["description"].as_str().unwrap_or("").contains("移动到 无底渊")));
    }

    #[test]
    fn agent_can_plan_a_journey_before_executing_it() {
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"forecast the route first","tool_name":"world_plan_travel","arguments":{"character_id":"char_tao","destination_id":"loc_abyss"}}"#,
            r#"{"thought":"forecast is feasible; execute the journey","tool_name":"world_travel_character","arguments":{"character_id":"char_tao","destination_id":"loc_abyss"}}"#,
            r#"{"thought":"verify the resulting state","tool_name":"world_snapshot","arguments":{}}"#,
            r#"{"thought":"completed","tool_name":null,"arguments":null}"#,
        ]);
        let shared = Arc::new(Mutex::new(WorldModel::new_shitian(20301020)));
        let mut agent = WorldAgent::with_shared_world(shared, planner(adapter), 5);
        let result = agent.run("Plan a route from char_tao to loc_abyss, then travel only if the estimate is affordable.");
        assert!(result.success, "{:?}", result.error);
        assert!(result.steps.len() >= 3);
        assert_eq!(result.steps[0].action, "world_plan_travel");
        assert_eq!(result.steps[1].action, "world_travel_character");
        assert_eq!(result.steps[2].action, "world_snapshot");
    }

    #[test]
    fn agent_uses_travel_tool_for_a_multi_hop_route_and_observes_new_location() {
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"travel from the hunting ground to the abyss","tool_name":"world_travel_character","arguments":{"character_id":"char_shikong","destination_id":"loc_abyss"}}"#,
            r#"{"thought":"check the final snapshot","tool_name":"world_snapshot","arguments":{}}"#,
            r#"{"thought":"verified","tool_name":null,"arguments":null}"#,
        ]);
        let mut preset = wanxiang_sdk::GenrePreset::shitian();
        preset.devourer_ids.clear();
        preset.appetite_base = 0.0;
        let shared = Arc::new(Mutex::new(WorldModel::from_preset(preset, 20301019)));
        let mut agent = WorldAgent::with_shared_world(shared.clone(), planner(adapter), 4);
        let result = agent.run("Travel char_shikong to loc_abyss by the shortest connected route.");
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.steps[0].action, "world_travel_character");
        assert!(result.steps[0].observation.contains("loc_hunt"));
        assert!(result.steps[0].observation.contains("loc_city"));
        assert!(result.steps[0].observation.contains("loc_abyss"));
        assert_eq!(result.steps[1].action, "world_snapshot");
        let guard = shared.lock().unwrap();
        let snapshot: Value = serde_json::from_str(&guard.snapshot_json()).unwrap();
        assert_eq!(
            snapshot["entities"].as_array().unwrap().iter()
                .find(|e| e["id"] == "char_shikong").unwrap()["location_id"],
            "loc_abyss"
        );
    }

    #[test]
    fn agent_movement_tool_rejects_unconnected_routes_without_mutation() {
        let adapter = ScriptedAdapter::new(vec![
            r#"{"thought":"try to travel without a direct route","tool_name":"world_move_character","arguments":{"character_id":"char_shikong","destination_id":"loc_abyss"}}"#,
        ]);
        let shared = Arc::new(Mutex::new(WorldModel::new_shitian(20301018)));
        let before = shared.lock().unwrap().snapshot_json();
        let mut agent = WorldAgent::with_shared_world(shared.clone(), planner(adapter), 2);
        let result = agent.run("Move char_shikong directly to loc_abyss.");

        assert!(!result.success);
        assert!(result.steps[0].observation.contains("no authored route"));
        assert_eq!(shared.lock().unwrap().snapshot_json(), before);
    }

    #[test]
    fn rule_experiment_is_deterministic_and_does_not_mutate_live_world() {
        let mut world = WorldModel::new(WorldRules::default(), 77);
        world.step(7);
        let before = world.snapshot_json();
        let first = run_rule_experiment(&world, "energy_density", 0.95, 12).unwrap();
        let second = run_rule_experiment(&world, "energy_density", 0.95, 12).unwrap();
        assert_eq!(first["baseline"], second["baseline"]);
        assert_eq!(first["altered"], second["altered"]);
        assert_eq!(first["diff"], second["diff"]);
        assert_eq!(world.snapshot_json(), before, "experiment must not mutate the source world");
        assert_eq!(first["source_world_unchanged"], true);
        assert_eq!(first["diff"]["rules_differ"], true);
        assert_eq!(first["baseline"]["resources"]["conservation"]["conserved"], true);
        assert_eq!(first["altered"]["resources"]["conservation"]["conserved"], true);
    }

    #[test]
    fn invalid_rule_experiment_arguments_fail_before_branching() {
        let world = WorldModel::new(WorldRules::default(), 78);
        let before = world.snapshot_json();
        assert!(run_rule_experiment(&world, "unknown_field", 0.9, 2).is_err());
        assert!(run_rule_experiment(&world, "energy_density", f64::NAN, 2).is_err());
        assert!(run_rule_experiment(&world, "energy_density", 1.2, 2).is_err());
        assert!(run_rule_experiment(&world, "energy_density", 0.9, 1_001).is_err());
        assert_eq!(world.snapshot_json(), before);
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
    fn snapshot_tool_bounds_entity_faction_and_event_payloads() {
        let snapshot = json!({
            "meta": {"tick": 12},
            "entities": (0..25).map(|i| json!({"id": i})).collect::<Vec<_>>(),
            "factions": (0..15).map(|i| json!({"id": i})).collect::<Vec<_>>(),
            "events": (0..40).map(|i| json!({"id": i})).collect::<Vec<_>>(),
            "resources": {"energy": 0.5},
            "rules": {"energy_density": 0.7}
        });
        let bounded = bounded_snapshot(snapshot);
        assert_eq!(bounded["entities"]["count"], 25);
        assert_eq!(bounded["entities"]["sample"].as_array().unwrap().len(), MAX_ENTITY_SAMPLE);
        assert_eq!(bounded["factions"]["sample"].as_array().unwrap().len(), MAX_FACTION_SAMPLE);
        assert_eq!(bounded["events"]["sample"].as_array().unwrap().len(), MAX_EVENT_SAMPLE);
        assert_eq!(bounded["events"]["sample"][0]["id"], 30);
        assert_eq!(bounded["events"]["sample"][9]["id"], 39);
        assert_eq!(bounded["bounded"], true);
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
