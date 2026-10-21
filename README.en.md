# Wanxiang

**Wanxiang is an early-stage Rust framework for reproducible world simulation and controlled rule experiments.** The repository also contains research components for Agent runtime, memory, causal graphs, model adapters, and narrative tooling.

> **Project stage: experimental public release (0.1.x).** Module maturity varies and APIs may change. Reproducibility does not establish real-world predictive accuracy; causal utilities are not a complete causal-inference system; security helpers are not an OS isolation boundary.

> **Try the demo:** [Open the World Simulation Lab](https://daohan003.github.io/Wanxiang/). The static demo shows build-generated sample trajectories (seed 42, 100 ticks) and requires no login, model API key, or local Rust toolchain. Live ticking, state-changing operations, and Agent actions require running or deploying the Rust API separately; no default public live backend is provided.

**Languages:** [简体中文](README.md) · English

## What it does

Wanxiang explores a specific question: given world rules, an initial configuration, and a random seed, can we advance a structured simulated world and inspect or compare snapshots, events, resource ledgers, and trajectories under different rules?

- **Reproducible world simulation:** create a world from an explicit seed, advance it by ticks, and export snapshots, events, and reports.
- **Data-driven world setup:** use `GenrePreset` to describe locations, factions, characters, and parameters instead of hard-coding one story genre into the engine.
- **Controlled rule comparison:** compare a baseline and a rule variant using the same seed and tick count.
- **Research components:** separate engine crates explore causal graphs, memory, action selection, Agent tool workflows, LLM adapters, and narrative assistance.

The core claim to validate is not the number of modules. It is whether **reproducible world-state evolution and controlled experiments provide measurable value for simulation, interactive narrative, or Agent applications**.

## Quick start

Open the [World Simulation Lab](https://daohan003.github.io/Wanxiang/) to compare the baseline world with a rule variant, inspect metrics and events, and export a JSON comparison report. The static demo is generated at build time; it is not a live backend.

Requirements for local examples: a stable Rust toolchain and Cargo. These examples do not require an LLM, model download, or API key. Cargo may download Rust crate dependencies during the first build.

Generate a deterministic event/outline skeleton without an LLM:

```bash
cargo run --locked -p wanxiang_sdk --example gen_world -- --demo 7 100 12.0 4
```

The arguments are demo mode, seed, tick count, intensity parameter, and chapter count. Output is a rule-driven narrative outline and event table, not a complete LLM-written story.

Compare two world trajectories:

```bash
cargo run --locked -p wanxiang_sdk --example rule_compare
```

## Workspaces

The repository contains two Rust workspaces:

- **Root world-model workspace:** shared types, core orchestration, world model, engine, SDK, Agent bridge, and experiment utilities.
- **`engines/` workspace:** smaller research components for accounting, causal graphs, memory, Agent runtime, model adapters, security helpers, configuration, and narrative tooling.

These workspaces have different maturity and integration levels. A compiling crate or passing unit tests do not by themselves validate its modeling assumptions.

## Validation

Run the root workspace tests:

```bash
cargo test --locked --workspace
```

Run the separate engine workspace tests:

```bash
cargo test --locked --workspace --manifest-path engines/Cargo.toml
```

On Linux/macOS, `bash scripts/test-all.sh` runs both workspaces. On Windows, use `./scripts/test-all.ps1`.

## Documentation

- [What is Wanxiang?](docs/WHAT_IS_WANXIANG.md)
- [Documentation index](docs/README.md)
- [First experiment](docs/FIRST_EXPERIMENT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Product acceptance criteria](docs/PRODUCT_ACCEPTANCE.md)
- [Algorithm status and limitations](docs/ALGORITHM_STATUS.md)
- [Capability closure audit](docs/CAPABILITY_CLOSURE_AUDIT.md)
- [Security model](docs/SECURITY_MODEL.md)
- [Roadmap](docs/ROADMAP.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

## Limitations

Wanxiang is an early 0.1.x snapshot with uneven maturity across modules and potentially changing APIs. Reproducibility does not prove predictive accuracy. Causal-graph utilities are not complete causal inference. Security helpers are not an OS sandbox. See the architecture, algorithm-status, and security-model documents for scope and limitations.

The SDK regression suite includes a 1,000-tick fixed-seed replay test, resource-ledger invariant checks, and a test that a changed rule creates a distinct trajectory. These tests validate properties of the implemented simulation model; they do not establish real-world predictive or causal accuracy.
