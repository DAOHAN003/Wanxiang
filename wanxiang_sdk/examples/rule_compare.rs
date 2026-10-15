//! Compare two deterministic world trajectories under different rules.
//!
//! Run with:
//! cargo run --locked -p wanxiang_sdk --example rule_compare

use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

fn main() {
    let rules = WorldRules::for_genre(WorldType::Xianxia);
    let mut baseline = WorldModel::new(rules, 42);

    // Advance the baseline for a fixed number of ticks.
    baseline.step(50);

    // Rebuild the same seeded scenario while changing one rule parameter.
    // The SDK replays from the initial seed to the same tick under the new rules.
    let altered = baseline.branch(|rules| {
        rules.energy_density = 0.95;
    });

    println!("Baseline world:");
    println!("{}", baseline.info_json());
    println!();
    println!("World with modified energy_density:");
    println!("{}", altered.info_json());
    println!();
    println!("Structured comparison:");
    println!("{}", baseline.diff_json(&altered));
}
