//! Reproducible experiment orchestration for the Wanxiang world simulator.
//!
//! This crate is an experiment framework, not an Agent. It coordinates paired
//! baseline/intervention runs, records their snapshots and diff, and evaluates
//! explicit invariants. The built-in energy-density intervention is a model
//! intervention; it is not by itself evidence of real-world causal effects.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::fmt;
use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

pub const EXPERIMENT_SCHEMA_VERSION: u32 = 1;
pub const MAX_EXPERIMENT_TICKS: u64 = 1_000;
pub const DEFAULT_CONSERVATION_TOLERANCE: f64 = 1e-4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentSpec {
    pub schema_version: u32,
    pub seed: u64,
    pub ticks: u64,
    /// The current first-party experiment adapter supports this explicit rule.
    pub intervention_energy_density: f64,
}

impl ExperimentSpec {
    pub fn new(seed: u64, ticks: u64, intervention_energy_density: f64) -> Self {
        Self {
            schema_version: EXPERIMENT_SCHEMA_VERSION,
            seed,
            ticks,
            intervention_energy_density,
        }
    }

    fn validate(&self) -> Result<(), ExperimentError> {
        if self.schema_version != EXPERIMENT_SCHEMA_VERSION {
            return Err(ExperimentError::UnsupportedSchema(self.schema_version));
        }
        if !(1..=MAX_EXPERIMENT_TICKS).contains(&self.ticks) {
            return Err(ExperimentError::InvalidTicks(self.ticks));
        }
        if !self.intervention_energy_density.is_finite()
            || !(0.0..=1.0).contains(&self.intervention_energy_density)
        {
            return Err(ExperimentError::InvalidEnergyDensity(
                self.intervention_energy_density,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BranchResult {
    pub seed: u64,
    pub tick: u64,
    pub event_count: u64,
    pub conservation_drift: f64,
    pub conserved: bool,
    /// Bounded SDK snapshot; the full live runtime is not serialized.
    pub snapshot: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvariantResult {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

pub struct InvariantInput<'a> {
    pub seed: u64,
    pub initial_tick: u64,
    pub ticks: u64,
    pub baseline: &'a Value,
    pub intervention: &'a Value,
    pub baseline_replay: &'a Value,
    pub diff: &'a Value,
    /// Canonical conservation status from WorldModel::is_conserved().
    pub baseline_conserved: bool,
    pub intervention_conserved: bool,
    /// Canonical drift values from WorldModel::conservation_drift().
    pub baseline_conservation_drift: f64,
    pub intervention_conservation_drift: f64,
    pub conservation_tolerance: f64,
}

/// A pluggable check over a paired experiment. Custom checks can be registered
/// without changing the simulator or the report schema.
pub trait WorldInvariant: Send + Sync {
    fn name(&self) -> &'static str;
    fn evaluate(&self, input: &InvariantInput<'_>) -> InvariantResult;
}

#[derive(Default)]
pub struct InvariantRegistry {
    invariants: Vec<Box<dyn WorldInvariant>>,
}

impl InvariantRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry.register(SeedMatches);
        registry.register(TickAdvanceMatches);
        registry.register(ResourceLedgerConserved);
        registry.register(RulesDiffer);
        registry.register(ReplayDeterministic);
        registry
    }

    pub fn register<I>(&mut self, invariant: I)
    where
        I: WorldInvariant + 'static,
    {
        self.invariants.push(Box::new(invariant));
    }

    pub fn evaluate(&self, input: &InvariantInput<'_>) -> Vec<InvariantResult> {
        self.invariants
            .iter()
            .map(|invariant| invariant.evaluate(input))
            .collect()
    }
}

pub struct SeedMatches;
impl WorldInvariant for SeedMatches {
    fn name(&self) -> &'static str { "seed_matches" }

    fn evaluate(&self, input: &InvariantInput<'_>) -> InvariantResult {
        let baseline_seed = input.baseline["meta"]["seed"].as_u64();
        let intervention_seed = input.intervention["meta"]["seed"].as_u64();
        let passed = baseline_seed == Some(input.seed) && intervention_seed == Some(input.seed);
        InvariantResult {
            name: self.name().into(),
            passed,
            expected: format!("both branches use seed {}", input.seed),
            actual: format!("baseline={baseline_seed:?}, intervention={intervention_seed:?}"),
            detail: "The paired branches must use the same requested seed.".into(),
        }
    }
}

pub struct TickAdvanceMatches;
impl WorldInvariant for TickAdvanceMatches {
    fn name(&self) -> &'static str { "tick_advance_matches" }

    fn evaluate(&self, input: &InvariantInput<'_>) -> InvariantResult {
        let expected_tick = input.initial_tick.saturating_add(input.ticks);
        let baseline_tick = input.baseline["meta"]["tick"].as_u64();
        let intervention_tick = input.intervention["meta"]["tick"].as_u64();
        let passed = baseline_tick == Some(expected_tick)
            && intervention_tick == Some(expected_tick);
        InvariantResult {
            name: self.name().into(),
            passed,
            expected: format!("both branches reach tick {expected_tick}"),
            actual: format!("baseline={baseline_tick:?}, intervention={intervention_tick:?}"),
            detail: "The rule change must not alter the requested simulation horizon.".into(),
        }
    }
}

pub struct ResourceLedgerConserved;
impl WorldInvariant for ResourceLedgerConserved {
    fn name(&self) -> &'static str { "resource_ledger_conserved" }

    fn evaluate(&self, input: &InvariantInput<'_>) -> InvariantResult {
        let baseline_drift = input.baseline_conservation_drift;
        let intervention_drift = input.intervention_conservation_drift;
        let baseline_ok = input.baseline_conserved
            && baseline_drift.is_finite()
            && baseline_drift.abs() <= input.conservation_tolerance;
        let intervention_ok = input.intervention_conserved
            && intervention_drift.is_finite()
            && intervention_drift.abs() <= input.conservation_tolerance;
        let passed = baseline_ok && intervention_ok;
        InvariantResult {
            name: self.name().into(),
            passed,
            expected: format!(
                "both declared conserved with |drift| <= {}",
                input.conservation_tolerance
            ),
            actual: format!(
                "baseline={baseline_ok} (drift={baseline_drift:?}), intervention={intervention_ok} (drift={intervention_drift:?})"
            ),
            detail: "This uses the canonical WorldModel conservation ledger and its own drift metric; exported resource summaries may expose separate domain-level accounting. Neither is a physical conservation-law claim.".into(),
        }
    }
}

pub struct RulesDiffer;
impl WorldInvariant for RulesDiffer {
    fn name(&self) -> &'static str { "intervention_changes_rule" }

    fn evaluate(&self, input: &InvariantInput<'_>) -> InvariantResult {
        let rules_differ = input.diff["rules_differ"].as_bool() == Some(true);
        InvariantResult {
            name: self.name().into(),
            passed: rules_differ,
            expected: "the baseline and intervention rules differ".into(),
            actual: format!("rules_differ={rules_differ}"),
            detail: "This validates a model-internal rule intervention; it is not a real-world causal claim.".into(),
        }
    }
}

pub struct ReplayDeterministic;
impl WorldInvariant for ReplayDeterministic {
    fn name(&self) -> &'static str { "fixed_seed_replay_matches" }

    fn evaluate(&self, input: &InvariantInput<'_>) -> InvariantResult {
        let same = input.baseline == input.baseline_replay;
        InvariantResult {
            name: self.name().into(),
            passed: same,
            expected: "equivalent baseline runs produce equal structured snapshots".into(),
            actual: if same { "snapshots match".into() } else { "snapshots differ".into() },
            detail: "Equality is checked on parsed JSON values, not on raw serialized bytes.".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentReport {
    pub schema_version: u32,
    pub framework: String,
    pub engine_version: String,
    pub spec: ExperimentSpec,
    pub initial_tick: u64,
    pub target_tick: u64,
    pub baseline: BranchResult,
    pub intervention: BranchResult,
    pub intervention_rule: String,
    pub diff: Value,
    pub invariants: Vec<InvariantResult>,
    pub passed: bool,
    pub interpretation: String,
}

#[derive(Debug)]
pub enum ExperimentError {
    UnsupportedSchema(u32),
    InvalidTicks(u64),
    InvalidEnergyDensity(f64),
    InvalidSnapshot(String),
    InvalidDiff(String),
}

impl fmt::Display for ExperimentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => write!(f, "unsupported experiment schema version: {version}"),
            Self::InvalidTicks(ticks) => write!(f, "ticks must be in 1..={MAX_EXPERIMENT_TICKS}, got {ticks}"),
            Self::InvalidEnergyDensity(value) => write!(f, "intervention energy_density must be finite and in 0..=1, got {value}"),
            Self::InvalidSnapshot(message) => write!(f, "invalid world snapshot: {message}"),
            Self::InvalidDiff(message) => write!(f, "invalid world diff: {message}"),
        }
    }
}

impl Error for ExperimentError {}

fn parse_snapshot(world: &WorldModel) -> Result<Value, ExperimentError> {
    serde_json::from_str(&world.snapshot_json())
        .map_err(|error| ExperimentError::InvalidSnapshot(error.to_string()))
}

/// Suggest a deterministic value different from the default world's energy density.
/// The caller may override this value, subject to the spec's 0..=1 validation.
pub fn suggest_energy_density_intervention(seed: u64) -> Result<f64, ExperimentError> {
    let initial = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), seed);
    let snapshot = parse_snapshot(&initial)?;
    let current = snapshot["rules"]["energy_density"]
        .as_f64()
        .ok_or_else(|| ExperimentError::InvalidSnapshot("rules.energy_density is not numeric".into()))?;
    Ok(if (current - 0.95).abs() < 1e-9 { 0.15 } else { 0.95 })
}

/// Run two paired branches from the same initial world and return a deterministic report.
///
/// The function records snapshots and model-internal comparisons. It does not claim that
/// simulator interventions identify real-world causal effects.
pub fn run_experiment(spec: ExperimentSpec) -> Result<ExperimentReport, ExperimentError> {
    spec.validate()?;

    let initial = WorldModel::new(
        WorldRules::for_genre(WorldType::Xianxia),
        spec.seed,
    );
    let initial_tick = initial.current_tick();
    let baseline_initial = initial.fork();
    let replay_initial = initial.fork();
    let mut baseline = baseline_initial;
    let mut baseline_replay = replay_initial;
    let mut intervention = initial.branch(|rules| {
        rules.energy_density = spec.intervention_energy_density;
    });

    baseline.step(spec.ticks);
    baseline_replay.step(spec.ticks);
    intervention.step(spec.ticks);

    let baseline_snapshot = parse_snapshot(&baseline)?;
    let baseline_replay_snapshot = parse_snapshot(&baseline_replay)?;
    let intervention_snapshot = parse_snapshot(&intervention)?;
    let diff: Value = serde_json::from_str(&baseline.diff_json(&intervention))
        .map_err(|error| ExperimentError::InvalidDiff(error.to_string()))?;

    let baseline_result = branch_result(spec.seed, &baseline, baseline_snapshot.clone());
    let intervention_result = branch_result(spec.seed, &intervention, intervention_snapshot.clone());
    let target_tick = initial_tick.saturating_add(spec.ticks);

    let input = InvariantInput {
        seed: spec.seed,
        initial_tick,
        ticks: spec.ticks,
        baseline: &baseline_snapshot,
        intervention: &intervention_snapshot,
        baseline_replay: &baseline_replay_snapshot,
        diff: &diff,
        baseline_conserved: baseline.is_conserved(),
        intervention_conserved: intervention.is_conserved(),
        baseline_conservation_drift: baseline.conservation_drift(),
        intervention_conservation_drift: intervention.conservation_drift(),
        conservation_tolerance: DEFAULT_CONSERVATION_TOLERANCE,
    };
    let invariants = InvariantRegistry::standard().evaluate(&input);
    let passed = invariants.iter().all(|check| check.passed);

    Ok(ExperimentReport {
        schema_version: EXPERIMENT_SCHEMA_VERSION,
        framework: "Wanxiang reproducible world experiment".into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
        spec,
        initial_tick,
        target_tick,
        baseline: baseline_result,
        intervention: intervention_result,
        intervention_rule: "energy_density".into(),
        diff,
        invariants,
        passed,
        interpretation: "Results describe the configured simulator only. Rule sensitivity is not evidence of real-world causal identification; the resource ledger is an abstract model invariant, not a physical conservation claim.".into(),
    })
}

fn branch_result(seed: u64, world: &WorldModel, snapshot: Value) -> BranchResult {
    BranchResult {
        seed,
        tick: world.current_tick(),
        event_count: world.event_count() as u64,
        conservation_drift: world.conservation_drift(),
        conserved: world.is_conserved(),
        snapshot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec(seed: u64, ticks: u64) -> ExperimentSpec {
        ExperimentSpec::new(
            seed,
            ticks,
            suggest_energy_density_intervention(seed).expect("the default world snapshot should be valid"),
        )
    }

    #[test]
    fn paired_experiment_checks_seed_ticks_ledger_and_replay() {
        let report = run_experiment(valid_spec(42, 10)).expect("experiment should run");
        assert!(report.passed, "failed checks: {:?}", report.invariants);
        assert_eq!(report.baseline.seed, 42);
        assert_eq!(report.intervention.seed, 42);
        assert_eq!(report.baseline.tick, report.target_tick);
        assert_eq!(report.intervention.tick, report.target_tick);
        assert!(report.baseline.conserved, "baseline SDK ledger drift: {}", report.baseline.conservation_drift);
        assert!(report.intervention.conserved, "intervention SDK ledger drift: {}", report.intervention.conservation_drift);
        assert!(report.invariants.iter().any(|item| item.name == "resource_ledger_conserved" && item.passed));
        assert_eq!(report.diff["rules_differ"].as_bool(), Some(true));
        assert_eq!(report.invariants.len(), 5);
    }

    #[test]
    fn same_spec_produces_the_same_report() {
        let spec = valid_spec(2026, 8);
        let first = run_experiment(spec.clone()).expect("first experiment should run");
        let second = run_experiment(spec).expect("second experiment should run");
        assert_eq!(
            serde_json::to_string(&first).expect("report serializes"),
            serde_json::to_string(&second).expect("report serializes")
        );
    }

    #[test]
    fn rejects_invalid_horizon_and_intervention() {
        assert!(matches!(
            run_experiment(ExperimentSpec::new(1, 0, 0.5)),
            Err(ExperimentError::InvalidTicks(0))
        ));
        assert!(matches!(
            run_experiment(ExperimentSpec::new(1, MAX_EXPERIMENT_TICKS + 1, 0.5)),
            Err(ExperimentError::InvalidTicks(_))
        ));
        assert!(matches!(
            run_experiment(ExperimentSpec::new(1, 1, f64::NAN)),
            Err(ExperimentError::InvalidEnergyDensity(_))
        ));
        assert!(matches!(
            run_experiment(ExperimentSpec::new(1, 1, 1.1)),
            Err(ExperimentError::InvalidEnergyDensity(_))
        ));
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let mut spec = valid_spec(9, 1);
        spec.schema_version = EXPERIMENT_SCHEMA_VERSION + 1;
        assert!(matches!(
            run_experiment(spec),
            Err(ExperimentError::UnsupportedSchema(_))
        ));
    }
}


pub mod causal;
pub mod evaluation;

pub use causal::{CausalInterventionResult, CausalModelError, LinearScm, LinearScmNode};
pub use evaluation::{
    compare_agents, summarize_agent_trials, AgentEvaluationError, AgentSummary,
    AgentTrialRecord, PairedAgentComparison,
};

pub mod environment;
pub use environment::{EnvironmentError, SdkWorldEnvironment, WorldAction, WorldEnvironment};
