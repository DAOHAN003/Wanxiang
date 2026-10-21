# SDK compatibility and versioning policy

This document defines the intended integration contract for `wanxiang_sdk`. It is deliberately conservative: the workspace is currently versioned `0.1.0`, so the SDK is pre-1.0 and should not be treated as a permanently stable API.

## Stability levels

| Surface | Current level | Guidance |
|---|---|---|
| `WorldModel` as the intended embedding entry point | Preview | Prefer it over depending directly on internal crates, but pin the exact crate/workspace revision in production applications. |
| Constructors, stepping, read queries, branch/fork and checkpoint methods | Preview Rust API | Public and documented, but source-level breaking changes may still occur before 1.0. Review release notes and compile against the exact version being upgraded to. |
| `GenrePreset` and its public configuration structs | Preview configuration API | Treat field additions, defaults, validation and preset semantics as version-sensitive. Keep application-owned preset fixtures under test. |
| `snapshot_json`, `info_json`, `report_json`, `economy_json` | Unversioned JSON views | Do not assume arbitrary fields, nested shapes or byte-for-byte compatibility across crate versions. |
| `checkpoint_json` / `from_checkpoint_json` | Legacy unversioned rebuild recipe | Retained for compatibility with existing saved files. It stores initialization and operations, not an opaque runtime image. Keep the original file and fail closed if restore fails. |
| `checkpoint_envelope_json` / `from_checkpoint_envelope_json` | Versioned persistence envelope, schema v1 | Writes `format: "wanxiang-checkpoint"` and `schema_version: 1` around the rebuild recipe. Unknown formats/schema versions and unknown top-level or v1 payload fields are rejected. See `schemas/wanxiang-checkpoint-v1.schema.json`. The reader currently supports v1 only; there is no automatic cross-version migration. |
| `wanxiang-experiment-report-v1` export | Versioned report format | Validate with the repository schema and validator for that exact format. Its version does not make every nested SDK JSON structure a general-purpose stable API. |

## Versioning rules

1. Before 1.0, treat the Rust source API as preview. Patch releases should avoid source-breaking changes where practical; a breaking change may occur in a new 0.y release and must be called out explicitly.
2. Every public-API change must include an API-focused test or an explanation of why testing is not applicable, plus a migration note in the pull request/release notes when downstream code or data may be affected.
3. Once 1.0 is declared, use semantic versioning for the documented stable surface: no breaking change to that surface in a patch or minor release. New optional behaviour should have explicit defaults and tests.
4. Do not silently repurpose a serialized field, enum value, rule meaning or operation. For persisted formats, introduce an explicit format/schema version and a migration or a clear rejection path before promising cross-version restore.
5. Deterministic replay is scoped to identical explicit inputs and a compatible build/runtime. A seed alone is not a complete experiment specification: rules, preset/configuration, operation sequence and relevant version information matter.
6. Timing measurements and model assumptions are not compatibility guarantees. Resource-conservation and replay checks are correctness invariants, not a promise that every future model produces the same trajectory.

## Consumer upgrade procedure

- Pin the current dependency version or repository commit; do not float production deployments on an unreviewed branch.
- Before upgrading, run the consumer's compile/test suite and replay its saved representative checkpoints in an isolated process. Compare structured snapshots, tick metadata, resource invariants and rule-diff outputs.
- For new durable writes, prefer `checkpoint_envelope_json` and restore with `from_checkpoint_envelope_json`; retain the envelope's format and schema version with the payload. Unknown versions are rejected rather than guessed.
- Existing unversioned checkpoints continue to use `from_checkpoint_json`. If restore rejects a saved file or replay diverges, stop the upgrade path, preserve the source checkpoint and surface a migration error. Do not silently reset the world or claim it restored successfully.
- Review changes to `WorldModel`, `GenrePreset`, exported structs, operation variants, JSON schema files and benchmark reports separately.
- Record the old/new version, seed, rules/preset, operation history, expected tick and snapshot comparison in the upgrade report.

## Minimal embedding example

```rust
use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut world = WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42);
    world.step(100);

    let checkpoint = world.checkpoint_envelope_json()?;
    let restored = WorldModel::from_checkpoint_envelope_json(&checkpoint)?;
    assert_eq!(world.snapshot_json(), restored.snapshot_json());
    assert!(restored.is_conserved());

    println!("{}", restored.info_json());
    Ok(())
}
```

The example proves same-build local reconstruction only. The versioned envelope makes the data format explicit; it does not guarantee that a future simulator version can replay the same operations. The legacy unversioned reader remains available, but the SDK does not automatically migrate between schema versions yet.

## Release checklist

Before declaring the SDK's stable 1.0 surface:

- Identify the exact stable public types and methods; mark unstable APIs explicitly.
- [x] Add an explicit v1 envelope for durable SDK checkpoints, publish its JSON Schema, and reject unknown format/schema versions; legacy unversioned reads remain supported through the existing API.
- [ ] Define and implement explicit migration paths and a support window before promising cross-version checkpoint restoration.
- Compile a downstream consumer fixture using only the declared stable surface.
- Run seed/horizon replay, conservation and rule-intervention baselines in CI.
- Publish migration notes and support-window policy for any persisted-data format.
