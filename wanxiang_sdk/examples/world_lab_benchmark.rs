//! Fixed-seed acceptance benchmark for deterministic long-horizon world experiments.
//!
//! Usage: cargo run --locked -p wanxiang_sdk --example world_lab_benchmark -- 100
//! The optional horizon is bounded to 1..=1000 ticks.

use serde_json::{json, Value};
use std::time::Instant;
use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

fn main() {
    let ticks = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100);
    if !(1..=1_000).contains(&ticks) {
        eprintln!("ticks must be an integer in 1..=1000");
        std::process::exit(2);
    }

    let seeds = [7_u64, 42, 2_026, 9_001];
    let started = Instant::now();
    let mut cases: Vec<Value> = Vec::with_capacity(seeds.len());

    for seed in seeds {
        let initial = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), seed);
        let source: Value = serde_json::from_str(&initial.snapshot_json())
            .expect("SDK snapshot should be valid JSON");
        let original_energy = source["rules"]["energy_density"]
            .as_f64()
            .expect("energy density must be numeric");
        let intervention_energy = if (original_energy - 0.95).abs() < 1e-9 {
            0.15
        } else {
            0.95
        };

        let mut replay_a = initial.fork();
        let mut replay_b = initial.fork();
        replay_a.step(ticks);
        replay_b.step(ticks);
        let deterministic = replay_a.snapshot_json() == replay_b.snapshot_json();
        if !deterministic {
            eprintln!("determinism regression for seed {seed} at {ticks} ticks");
            std::process::exit(1);
        }

        let mut baseline = initial.fork();
        let mut altered = initial.branch(|rules| rules.energy_density = intervention_energy);
        baseline.step(ticks);
        altered.step(ticks);
        let baseline_snapshot: Value = serde_json::from_str(&baseline.snapshot_json())
            .expect("baseline snapshot should be valid JSON");
        let altered_snapshot: Value = serde_json::from_str(&altered.snapshot_json())
            .expect("altered snapshot should be valid JSON");
        let diff: Value = serde_json::from_str(&baseline.diff_json(&altered))
            .expect("world diff should be valid JSON");
        let baseline_snapshot_tick = baseline_snapshot["meta"]["tick"].as_u64();
        let altered_snapshot_tick = altered_snapshot["meta"]["tick"].as_u64();
        let snapshots_match_live_state = baseline_snapshot_tick == Some(baseline.current_tick())
            && altered_snapshot_tick == Some(altered.current_tick());
        if !snapshots_match_live_state {
            eprintln!("snapshot tick regression for seed {seed} at {ticks} ticks");
            std::process::exit(1);
        }

        let baseline_conserved = baseline.is_conserved();
        let altered_conserved = altered.is_conserved();
        if !baseline_conserved || !altered_conserved {
            eprintln!("conservation regression for seed {seed} at {ticks} ticks");
            std::process::exit(1);
        }

        cases.push(json!({
            "seed": seed,
            "ticks_per_branch": ticks,
            "deterministic_replay": deterministic,
            "baseline": {
                "tick": baseline.current_tick(),
                "events": baseline.event_count(),
                "drift": baseline.conservation_drift(),
                "snapshot_tick_matches": baseline_snapshot_tick == Some(baseline.current_tick()),
                "conserved": baseline_conserved
            },
            "intervention": {
                "rule": "energy_density",
                "from": original_energy,
                "to": intervention_energy,
                "tick": altered.current_tick(),
                "events": altered.event_count(),
                "drift": altered.conservation_drift(),
                "snapshot_tick_matches": altered_snapshot_tick == Some(altered.current_tick()),
                "conserved": altered_conserved
            },
            "diff": {
                "rules_differ": diff["rules_differ"],
                "first_divergence_tick": diff["first_divergence_tick"],
                "event_count": diff["event_count"],
                "count_delta": diff["count_delta"],
                "summary": diff["summary"]
            },
            "interpretation": "Model-internal intervention sensitivity only; not a real-world causal claim."
        }));
    }

    println!("{}", serde_json::to_string_pretty(&json!({
        "benchmark": "wanxiang_world_lab",
        "schema_version": 1,
        "horizon": ticks,
        "seeds": seeds,
        "seed_count": seeds.len(),
        "elapsed_ms": started.elapsed().as_millis(),
        "passed": true,
        "cases": cases
    })).expect("benchmark report should serialize"));
}
