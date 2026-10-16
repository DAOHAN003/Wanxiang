# Wanxiang experiment kernel

The wanxiang_experiment crate coordinates paired simulation runs through the existing wanxiang_sdk::WorldModel. It is a framework component, not an Agent.

## Run

    cargo run --locked -p wanxiang_experiment --example run_experiment -- 42 100

The third argument can explicitly set energy_density in 0..=1. When omitted, the example chooses a deterministic value different from the default world rule for that seed.

The command emits a JSON report containing the experiment spec, baseline and intervention snapshots, the model diff, invariant checks, and an overall pass flag. The same spec is designed to produce the same structured report with the same simulator version and build configuration.

## Built-in checks

- seed_matches: both worlds use the requested seed.
- tick_advance_matches: both worlds advance the requested number of ticks.
- resource_ledger_conserved: both snapshots report the abstract resource ledger as conserved and within the configured drift tolerance.
- intervention_changes_rule: the simulation diff reports a changed rule.
- fixed_seed_replay_matches: duplicate baseline runs produce equal structured snapshots.

InvariantRegistry allows a caller to register additional WorldInvariant implementations. Each check reports expected and actual values; a failed check makes report.passed false and causes the example to exit unsuccessfully.

## Scope

The first adapter supports an energy_density rule intervention in the seeded Xianxia world. The report is a model-internal sensitivity experiment, not a general causal inference engine or evidence of real-world causal effects. The resource-ledger check verifies a declared simulation invariant, not a physical conservation law. Agent evaluation records, additional intervention adapters, graph/SCM validation, and larger benchmark suites remain follow-on work.

## Environment protocol

The WorldEnvironment trait exposes reset, observe, act, and current_tick operations. SdkWorldEnvironment adapts the existing WorldModel to this protocol using bounded Advance actions. This is the environment interface for callers and participants; it does not make the world itself an Agent.

## Explicit structural causal model

The causal module implements a small deterministic linear SCM with caller-supplied equations, exogenous values, validation of missing parents and cycles, and do-interventions that recompute descendants. It is intended as a transparent known-answer fixture and an evaluation baseline, not causal discovery or proof that the supplied equations describe reality.

## Agent evaluation records

The evaluation module defines typed trial records and paired comparisons. A comparison rejects duplicate records and requires baseline/candidate records to cover identical task/seed pairs. This prevents comparing agents on unmatched tasks, but the module only aggregates records: an application adapter must still execute real agents and record their outcomes.
