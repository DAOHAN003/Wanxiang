# What Is Wanxiang?

## Short answer

Wanxiang is best described today as an **early-stage, Rust-based world-simulation framework and experimentation platform**. Its central capability is to construct a structured simulated world, advance it deterministically from explicit rules and a seed, inspect events and snapshots, and compare trajectories under controlled rule changes.

It is reasonable to call Wanxiang a framework in the broad software-engineering sense: it provides reusable domain types, a simulation engine, an embedding-facing SDK, and integration points that applications can build on. The more precise public description is “reproducible world-simulation framework” rather than a general-purpose AI-agent framework.

## What makes it framework-like?

- **Reusable execution model:** callers can create a `WorldModel`, advance ticks, inspect state, export snapshots, and compare rule branches through the SDK.
- **Explicit experiment inputs:** seeds, initial configuration, rules, and tick counts make model-internal experiments inspectable and reproducible within the documented compatibility boundaries.
- **Structured outputs:** world snapshots, event records, and comparison reports can be inspected or exported by downstream applications.
- **An integration surface:** `wanxiang_sdk` is the intended embedding API; `wanxiang_agent_bridge` connects a bounded set of Agent tools to the world model.
- **Multiple extension areas:** the repository also contains research crates for memory, causal-graph utilities, Agent runtime, model adapters, and narrative tooling.

## What it is not yet

The repository should not currently be presented as all of the following:

- A single, uniformly mature AI-agent framework. The root world-model workspace and the separate `engines/` workspace are distinct Cargo workspaces, and their components have different maturity and integration levels.
- A validated cognitive architecture. The presence of memory, causal, and Agent crates does not by itself prove that their combined behavior is generally reliable.
- A real-world forecasting or causal-inference system. Reproducibility and rule-branch comparison establish properties of the implemented simulation model, not real-world predictive validity or causal effects.
- A production-ready hosted multi-tenant service. The repository includes a local/live API path and deployment artifacts, but a successful build does not mean a public production service is operated.
- A drop-in replacement for an existing engine or Agent framework. That claim would require compatibility, workload, and comparative evaluations.

## Current product boundary

The clearest first use case is **building and evaluating reproducible simulated worlds**. An application can define a world, run it under a fixed seed and rule set, compare a baseline with a changed rule, then inspect or export the resulting structured evidence. Agent execution is an optional way to act in a world, not the definition of the whole product.

The static web demo is an entry point to compare build-generated sample trajectories without an API key or local Rust installation. Live ticking, state-changing operations, and Agent execution require running or deploying the Rust API separately.

## Recommended one-sentence description

> **Wanxiang is an early-stage Rust framework for reproducible world simulation and controlled rule experiments, with optional Agent, memory, causal-graph, and narrative research components.**

As the project matures, stronger claims should be earned through stable public interfaces, end-to-end integration tests, reproducible benchmark datasets, documented compatibility guarantees, and evidence from external users.
