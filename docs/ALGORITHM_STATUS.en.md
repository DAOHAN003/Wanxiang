# Algorithm Status and Scope

This document separates implementation presence from theoretical or empirical validation. It summarizes the current public repository; it is not a claim that every component is integrated or production-ready.

| Area | Present in code | Scope and limitation |
|---|---|---|
| World state | Entity/world types, deterministic identifiers, abstract resource ledger | Ledger totals are bookkeeping invariants, not physical conservation laws. Domain units and invalid numeric inputs still require validation. |
| Causal graphs | Weighted directed graphs, separation utilities, path/intervention-style propagation | Results depend on graph assumptions. Simplified path sums are not a complete causal-inference implementation. |
| Causal abstraction | Candidate node mappings based on permutation enumeration and edge/sign matches | Search is factorial and suitable only for small graphs; it is not a full VF2 implementation. |
| Memory | Deterministic sparse codes, weighting, association and causal-link components | Determinism does not establish semantic similarity. Evaluate retrieval against keyword/BM25 and vector baselines on labeled tasks. |
| Action selection | UCB1-style selection, rule-based metacognitive checks, evolutionary selection | Named algorithms are established methods; surrounding heuristics require independent evaluation. |
| Agent runtime | ReAct-style flow, tool registry, approval policies, skills and team orchestration APIs | Interfaces and unit tests do not prove robust autonomy, safe tool execution, or distributed reliability. |
| LLM adapter | Adapter trait, cache and route-selection helpers | A common interface does not mean every provider has a production-ready network/SSE implementation. |
| Security helpers | Pattern scanning, path-policy checks, audit and privacy helpers | Defense-in-depth utilities, not an OS sandbox or formal privacy guarantee. |
| Narrative tools | Concept spreading, causal checks, consistency heuristics and memory workflows | Heuristic scores do not establish story quality or reliable semantic understanding. |

## Recommended validation

1. Add unit tests for algorithm invariants and malformed inputs.
2. Use property-based tests for resource accounting and graph operations.
3. Evaluate memory retrieval, causal alerts and character-consistency checks on labeled datasets.
4. Report baselines, false positives and false negatives.
5. Benchmark with explicit runtime and memory limits.
6. Run clean-checkout CI for both Rust workspaces before tagging a release.

A test count, deterministic output, or use of a named algorithm is not proof of theoretical correctness or application-level effectiveness.

## Related documents

- [What Is Wanxiang?](WHAT_IS_WANXIANG.md)
- [Architecture](ARCHITECTURE.md)
- [Roadmap](ROADMAP.md)
- [Security model](SECURITY_MODEL.md)
