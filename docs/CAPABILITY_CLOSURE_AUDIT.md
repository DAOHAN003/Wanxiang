# Capability and closure audit

This document records the integration status visible in the repository. “Closed loop” means more than having a type, trait, formula, or unit test: inputs must reach the relevant component, outputs must affect the next step, and the behaviour must be observable in an end-to-end test.

## Status summary

| Loop | Current status | Evidence in repository | What is still missing |
|---|---|---|---|
| World simulation: rules/seed -> ticks -> state/events/snapshot | Implemented in the main workspace | `wanxiang_sdk::WorldModel`, `wanxiang_engine::LivingCore::tick`, `wanxiang_world::WorldSimulator` | A concise end-to-end acceptance test covering a documented user scenario and its invariants |
| Rule comparison: common initial conditions -> altered rules -> replay -> diff | Implemented at SDK level; verify on each change | `WorldModel::branch`, `rule_compare`, `web_demo_data` | Keep regression tests for all operation types and custom initial entities; this is model sensitivity analysis, not real-world causal proof |
| Event -> learning/evolution bookkeeping | Partially connected | `LivingCore::tick` publishes events, updates state, calls `learn_from_text`, and periodically runs an evolution cycle | Evidence that learned/evolved parameters are fed back into subsequent world dynamics, with a measurable before/after test |
| Agent ReAct: decision -> actual tool execution -> observation -> next decision | **Not closed; currently a simulated loop** | `AgentRunner::run` synthesizes thought/action/observation strings and comments that tool execution is simulated; it does not call `ToolRegistry::execute` or a model adapter | Connect a decision/adapter interface to the registry, feed tool output back as the next observation, enforce bounds and approval policy, and assert the whole cycle in an integration test |
| Agent + memory + causal analysis + LLM adapter | Not established as one end-to-end product loop | Separate crates in `engines/`; optional dependencies between some components | A wired orchestration example demonstrating write/retrieve memory, use evidence in a decision, execute an allowed tool, and persist a trace |
| LLM provider networking/streaming/fallback | Not proven production-ready from adapter interfaces and mock tests | `engines/llm_adapter` exposes adapter/router abstractions | Provider-specific contract tests for authentication, timeouts, retries, streaming, malformed responses, and fallback |
| Static demo data pipeline: Rust SDK -> JSON -> schema checks -> browser display | Implemented in the main-branch deployment workflow, with limited checks | `web_demo_data`, `.github/workflows/static.yml`, `site/index.html` | Move payload/schema regression checks into PR CI; test browser rendering with valid and invalid fixtures |
| Whole-repository integration | Not a single closed loop | Root workspace and `engines/` are separate Cargo workspaces, each tested independently by `.github/workflows/ci.yml` | A deliberately scoped integration test or example connecting the supported public components; do not imply all research crates are integrated |

## What we can accurately claim today

- The main workspace implements a seeded world simulation and an SDK for stepping, exporting, and comparing worlds.
- The SDK's rule branch reconstructs and replays the recorded operation history under changed rules. This compares complete simulated trajectories.
- The engine workspace contains experimental building blocks for agent runtime, memory, causal graph utilities, LLM adapter abstractions, security helpers, and narrative workflows.
- The current `AgentRunner` is not a model-backed agent executor: it generates placeholder steps and synthetic observations. The tool registry is a separate API and is not wired into that loop.
- Having independent crates compile and pass unit tests does not establish a complete Agent-to-world feedback loop, empirical model quality, provider reliability, or production security.

## Recommended closure order

1. Preserve the existing world simulation and rule-branch regression tests.
2. Add one documented end-to-end example with a stable seed, explicit rules, a bounded tick count, and machine-checkable output invariants.
3. Close the Agent runtime gap: introduce an explicit decision/adapter boundary, execute selected tools through `ToolRegistry`, feed results back into the next iteration, and test approval/blocked/error/termination paths. Keep a deterministic fake adapter for CI.
4. Only after that, add an integration example for memory retrieval and causal evidence. Keep the LLM provider as an injected adapter so the core test remains deterministic and network-free.
5. Run both Cargo workspaces in CI and run integration/schema tests on pull requests. Publish a feature as integrated only after its end-to-end test passes.

## Validation standard

For each claimed loop, document: (a) entry point, (b) connected components, (c) state/output that flows back into the next iteration, (d) termination/error behaviour, and (e) an automated test that fails if the connection is broken.
