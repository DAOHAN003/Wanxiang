//! Run a reproducible baseline-vs-intervention experiment.
//!
//! Usage: cargo run --locked -p wanxiang_experiment --example run_experiment -- [seed] [ticks] [energy_density]

use std::process::ExitCode;
use wanxiang_experiment::{
    run_experiment, suggest_energy_density_intervention, ExperimentSpec, MAX_EXPERIMENT_TICKS,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 3 {
        return Err("usage: run_experiment [seed] [ticks] [energy_density]".into());
    }

    let seed = args.first().map_or(Ok(42_u64), |value| {
        value.parse::<u64>().map_err(|_| "seed must be an unsigned integer".to_string())
    })?;
    let ticks = args.get(1).map_or(Ok(100_u64), |value| {
        value.parse::<u64>().map_err(|_| "ticks must be an integer".to_string())
    })?;
    if !(1..=MAX_EXPERIMENT_TICKS).contains(&ticks) {
        return Err(format!("ticks must be in 1..={MAX_EXPERIMENT_TICKS}"));
    }

    let energy_density = match args.get(2) {
        Some(value) => value
            .parse::<f64>()
            .map_err(|_| "energy_density must be a number in 0..=1".to_string())?,
        None => suggest_energy_density_intervention(seed).map_err(|error| error.to_string())?,
    };

    let report = run_experiment(ExperimentSpec::new(seed, ticks, energy_density))
        .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );

    if report.passed {
        Ok(())
    } else {
        Err("experiment completed but one or more invariants failed; inspect report.invariants".into())
    }
}
