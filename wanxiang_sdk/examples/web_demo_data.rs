//! Generate a real, deterministic world comparison package for the static demo.
//! cargo run --locked -p wanxiang_sdk --example web_demo_data -- 42 100 > demo.json
use std::env;
use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let seed = args.get(0).and_then(|s| s.parse::<u64>().ok()).unwrap_or(42);
    let ticks = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(100).clamp(1, 2_000);
    let rules = WorldRules::for_genre(WorldType::Xianxia);
    let mut baseline = WorldModel::new(rules, seed);
    baseline.step(ticks);

    let baseline_energy = baseline.snapshot().rules.energy_density;
    let altered_energy = if (baseline_energy - 0.95).abs() < f64::EPSILON { 0.35 } else { 0.95 };
    let altered = baseline.branch(|rules| { rules.energy_density = altered_energy; });

    let baseline_json: serde_json::Value =
        serde_json::from_str(&baseline.snapshot_json()).expect("baseline snapshot must be valid JSON");
    let altered_json: serde_json::Value =
        serde_json::from_str(&altered.snapshot_json()).expect("altered snapshot must be valid JSON");
    let diff_json: serde_json::Value =
        serde_json::from_str(&baseline.diff_json(&altered)).expect("world diff must be valid JSON");
    let output = serde_json::json!({
        "schema_version": 1,
        "generated_by": "wanxiang_sdk::WorldModel",
        "scenario": {
            "seed": seed,
            "ticks": ticks,
            "changed_rule": "energy_density",
            "baseline_energy_density": baseline_energy,
            "altered_energy_density": altered_energy
        },
        "baseline": baseline_json,
        "altered": altered_json,
        "diff": diff_json
    });
    println!("{}", serde_json::to_string_pretty(&output).expect("comparison package must serialize"));
}
