//! Reproducible performance baseline for the public SDK integration path.
//!
//! Usage:
//!   cargo run --locked --release -p wanxiang_sdk --example sdk_performance_baseline -- 10 3
//!
//! Timings are observations, not hard pass/fail thresholds. Compare artifacts produced by
//! the same runner/profile; correctness invariants are enforced independently.

use serde_json::{json, Value};
use std::env;
use std::time::Instant;
use wanxiang_sdk::{GenrePreset, WorldModel};
use wanxiang_types::WorldPosition;

const SEED: u64 = 20_261_021;

#[derive(Clone, Copy)]
struct Workload {
    name: &'static str,
    extra_characters: usize,
    extra_locations: usize,
}

const WORKLOADS: [Workload; 3] = [
    Workload {
        name: "small",
        extra_characters: 0,
        extra_locations: 0,
    },
    Workload {
        name: "medium",
        extra_characters: 25,
        extra_locations: 10,
    },
    Workload {
        name: "large",
        extra_characters: 100,
        extra_locations: 40,
    },
];

fn bounded_arg(index: usize, name: &str, default: u64, min: u64, max: u64) -> u64 {
    match env::args().nth(index) {
        None => default,
        Some(value) => match value.parse::<u64>() {
            Ok(parsed) if (min..=max).contains(&parsed) => parsed,
            _ => {
                eprintln!("{name} must be an integer in {min}..={max}");
                std::process::exit(2);
            }
        },
    }
}

fn build_preset(workload: Workload) -> GenrePreset {
    let mut preset = GenrePreset::shitian();
    let location_template = preset
        .locations
        .iter()
        .find(|location| location.id == "loc_city")
        .expect("Shitian preset must contain loc_city")
        .clone();
    let character_template = preset
        .characters
        .first()
        .expect("Shitian preset must contain a character")
        .clone();

    for index in 0..workload.extra_locations {
        let id = format!("bench_loc_{index:04}");
        let mut location = location_template.clone();
        location.id = id.clone();
        location.name = format!("Benchmark location {index:04}");
        location.environment = "Synthetic performance fixture".to_string();
        location.connected_ids = vec!["loc_city".to_string()];
        location.position = Some(WorldPosition::new(
            (index % 50) as f64,
            (index / 50) as f64,
            0.0,
        ));
        location.inventory.clear();
        preset.locations.push(location);

        let faction = preset
            .factions
            .iter_mut()
            .find(|faction| faction.id == "fac_shimeng")
            .expect("Shitian preset must contain fac_shimeng");
        faction.territory_ids.push(id);
    }

    for index in 0..workload.extra_characters {
        let mut character = character_template.clone();
        character.id = format!("bench_char_{index:05}");
        character.name = format!("Benchmark character {index:05}");
        character.faction_id = "fac_shimeng".to_string();
        character.location_id = Some(if workload.extra_locations == 0 {
            "loc_city".to_string()
        } else {
            format!("bench_loc_{:04}", index % workload.extra_locations)
        });
        preset.characters.push(character);
    }

    preset
}

fn elapsed_us(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000_000.0
}

fn median(samples: &[Value], key: &str) -> f64 {
    let mut values: Vec<f64> = samples
        .iter()
        .map(|sample| {
            sample[key]
                .as_f64()
                .unwrap_or_else(|| panic!("sample field {key} must be numeric"))
        })
        .collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn run_workload(workload: Workload, ticks: u64, repeats: usize) -> Value {
    let mut samples = Vec::with_capacity(repeats);
    // Keep fixture counts explicit so workload-size changes require an intentional baseline update.
    let configured_characters = 4 + workload.extra_characters;
    let configured_locations = 3 + workload.extra_locations;
    let configured_factions = 3;

    for _ in 0..repeats {
        let preset = build_preset(workload);
        assert_eq!(preset.characters.len(), configured_characters);
        assert_eq!(preset.locations.len(), configured_locations);
        assert_eq!(preset.factions.len(), configured_factions);

        let started = Instant::now();
        let mut world = WorldModel::from_preset(preset, SEED);
        let construct_us = elapsed_us(started);

        let started = Instant::now();
        world.step(ticks);
        let simulate_us = elapsed_us(started);

        let started = Instant::now();
        let snapshot_json = world.snapshot_json();
        let snapshot_export_us = elapsed_us(started);
        let snapshot: Value =
            serde_json::from_str(&snapshot_json).expect("snapshot JSON must parse");
        let snapshot_tick = snapshot["meta"]["tick"].as_u64();
        let location_count = snapshot["locations"].as_array().map_or(0, Vec::len);

        let started = Instant::now();
        let checkpoint_json = world
            .checkpoint_json()
            .expect("checkpoint serialization must succeed");
        let checkpoint_encode_us = elapsed_us(started);

        let started = Instant::now();
        let restored = WorldModel::from_checkpoint_json(&checkpoint_json)
            .expect("checkpoint must restore by deterministic replay");
        let checkpoint_restore_us = elapsed_us(started);
        let replay_equal = restored.snapshot_json() == snapshot_json;

        drift = world.conservation_drift();
        if world.current_tick() != ticks
            || snapshot_tick != Some(ticks)
            || location_count != configured_locations
            || !world.is_conserved()
            || !restored.is_conserved()
            || !replay_equal
        {
            eprintln!(
                "performance baseline integrity failure: workload={}, tick={}, snapshot_tick={snapshot_tick:?}, locations={location_count}/{configured_locations}, conserved={}, restored_conserved={}, replay_equal={replay_equal}",
                workload.name,
                world.current_tick(),
                world.is_conserved(),
                restored.is_conserved()
            );
            std::process::exit(1);
        }

        let living_characters = world.character_count();
        let current_factions = world.faction_count();
        let current_events = world.event_count();
        let snapshot_bytes = snapshot_json.len();
        let checkpoint_bytes = checkpoint_json.len();
        let ticks_per_second = ticks as f64 / (simulate_us / 1_000_000.0).max(1e-9);

        samples.push(json!({
            "construct_us": construct_us,
            "simulate_us": simulate_us,
            "ticks_per_second": ticks_per_second,
            "snapshot_export_us": snapshot_export_us,
            "checkpoint_encode_us": checkpoint_encode_us,
            "checkpoint_restore_us": checkpoint_restore_us,
            "living_characters_after_simulation": living_characters,
            "locations": location_count,
            "factions": current_factions,
            "events": current_events,
            "snapshot_bytes": snapshot_bytes,
            "checkpoint_bytes": checkpoint_bytes,
            "conservation_drift": last["conservation_drift"]
        }));
    }

    let last = samples.last().expect("repeat count is bounded to at least one");
    json!({
        "name": workload.name,
        "seed": SEED,
        "tick_horizon": ticks,
        "configured_counts": {
            "characters": configured_characters,
            "locations": configured_locations,
            "factions": configured_factions
        },
        "observed_counts": {
            "living_characters_after_simulation": last["living_characters_after_simulation"],
            "locations": last["locations"],
            "factions": last["factions"],
            "events": last["events"]
        },
        "output_bytes": {
            "snapshot_json": last["snapshot_bytes"],
            "checkpoint_json": last["checkpoint_bytes"]
        },
        "integrity": {
            "snapshot_tick_matches": true,
            "resource_conserved": true,
            "checkpoint_replay_equal": true,
            "conservation_drift": drift
        },
        "samples": samples,
        "median": {
            "construct_us": median(&samples, "construct_us"),
            "simulate_us": median(&samples, "simulate_us"),
            "ticks_per_second": median(&samples, "ticks_per_second"),
            "snapshot_export_us": median(&samples, "snapshot_export_us"),
            "checkpoint_encode_us": median(&samples, "checkpoint_encode_us"),
            "checkpoint_restore_us": median(&samples, "checkpoint_restore_us")
        }
    })
}

fn main() {
    let ticks = bounded_arg(1, "tick horizon", 10, 1, 1_000);
    let repeats = bounded_arg(2, "repeat count", 3, 1, 10) as usize;
    let workloads: Vec<Value> = WORKLOADS
        .iter()
        .copied()
        .map(|workload| run_workload(workload, ticks, repeats))
        .collect();

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "benchmark": "wanxiang_sdk_performance",
            "schema_version": 1,
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "seed": SEED,
            "tick_horizon": ticks,
            "repeats": repeats,
            "timing_policy": "Informational only; compare artifacts from the same runner and build profile. No wall-clock regression threshold is enforced.",
            "workloads": workloads
        }))
        .expect("performance report should serialize")
    );
}
