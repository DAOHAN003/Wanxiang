//! Verify and deterministically replay a `wanxiang-replay-v1` package exported by the live API.
//!
//! Usage:
//! cargo run --locked -p wanxiang_sdk --example verify_replay -- replay.json
use std::{env, fs, process};
use serde_json::{json, Value};
use wanxiang_sdk::WorldModel;

const MAX_RESPONSE_EVENTS: usize = 200;
const MAX_RESPONSE_ENTITIES: usize = 500;
const MAX_RESPONSE_FACTIONS: usize = 100;

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("replay verification failed: {}", message.as_ref());
    process::exit(1);
}

fn truncate_tail(value: &mut Value, max: usize) {
    if let Some(items) = value.as_array_mut() {
        if items.len() > max {
            let remove = items.len() - max;
            items.drain(0..remove);
        }
    }
}

/// Mirror the version-1 live API snapshot window before comparing it with the full replay.
fn compact_snapshot(mut snapshot: Value) -> Value {
    let total_events = snapshot.get("events").and_then(Value::as_array).map_or(0, Vec::len);
    let (total_entities, alive_entities) = snapshot.get("entities")
        .and_then(Value::as_array)
        .map(|items| (
            items.len(),
            items.iter().filter(|item| item.get("alive").and_then(Value::as_bool).unwrap_or(false)).count(),
        ))
        .unwrap_or((0, 0));
    let total_factions = snapshot.get("factions").and_then(Value::as_array).map_or(0, Vec::len);

    truncate_tail(&mut snapshot["events"], MAX_RESPONSE_EVENTS);
    truncate_tail(&mut snapshot["entities"], MAX_RESPONSE_ENTITIES);
    truncate_tail(&mut snapshot["factions"], MAX_RESPONSE_FACTIONS);

    if !snapshot.get("meta").is_some_and(Value::is_object) {
        snapshot["meta"] = json!({});
    }
    snapshot["meta"]["total_events"] = json!(total_events);
    snapshot["meta"]["event_window_start"] = json!(total_events.saturating_sub(MAX_RESPONSE_EVENTS));
    snapshot["meta"]["events_truncated"] = json!(total_events > MAX_RESPONSE_EVENTS);
    snapshot["meta"]["total_entities"] = json!(total_entities);
    snapshot["meta"]["alive_entities"] = json!(alive_entities);
    snapshot["meta"]["entities_truncated"] = json!(total_entities > MAX_RESPONSE_ENTITIES);
    snapshot["meta"]["total_factions"] = json!(total_factions);
    snapshot["meta"]["factions_truncated"] = json!(total_factions > MAX_RESPONSE_FACTIONS);
    snapshot
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: cargo run --locked -p wanxiang_sdk --example verify_replay -- <replay.json>");
        process::exit(2);
    });
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| fail(format!("cannot read {path}: {error}")));
    let package: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| fail(format!("invalid JSON: {error}")));

    if package.get("format").and_then(Value::as_str) != Some("wanxiang-replay-v1")
        || package.get("schema_version").and_then(Value::as_u64) != Some(1)
    {
        fail("unsupported package format; expected wanxiang-replay-v1 schema_version 1");
    }

    let checkpoint = package.get("checkpoint")
        .filter(|value| value.is_object())
        .unwrap_or_else(|| fail("package is missing checkpoint object"));
    let operations = checkpoint.get("operations")
        .and_then(Value::as_array)
        .unwrap_or_else(|| fail("checkpoint is missing operation history"));
    let seed = checkpoint.get("seed").and_then(Value::as_u64)
        .unwrap_or_else(|| fail("checkpoint seed is missing or invalid"));
    let tick = checkpoint.get("tick").and_then(Value::as_u64)
        .unwrap_or_else(|| fail("checkpoint tick is missing or invalid"));

    if package.get("seed").and_then(Value::as_u64) != Some(seed)
        || package.get("tick").and_then(Value::as_u64) != Some(tick)
        || package.get("operation_count").and_then(Value::as_u64) != Some(operations.len() as u64)
    {
        fail("package header does not match checkpoint seed, tick, or operation count");
    }

    let exported_snapshot = package.get("snapshot")
        .filter(|value| value.is_object())
        .unwrap_or_else(|| fail("package is missing its exported WorldExport snapshot"));
    let checkpoint_json = serde_json::to_string(checkpoint)
        .unwrap_or_else(|error| fail(format!("cannot encode checkpoint: {error}")));
    let rebuilt = WorldModel::from_checkpoint_json(&checkpoint_json)
        .unwrap_or_else(|error| fail(format!("WorldModel replay failed: {error}")));

    let rebuilt_checkpoint: Value = serde_json::from_str(
        &rebuilt.checkpoint_json().unwrap_or_else(|error| fail(format!("cannot encode rebuilt checkpoint: {error}")))
    ).unwrap_or_else(|error| fail(format!("rebuilt checkpoint JSON is invalid: {error}")));
    if &rebuilt_checkpoint != checkpoint {
        fail("reconstructed checkpoint differs from the package checkpoint");
    }

    let rebuilt_snapshot: Value = serde_json::from_str(&rebuilt.snapshot_json())
        .unwrap_or_else(|error| fail(format!("rebuilt snapshot JSON is invalid: {error}")));
    let normalized_rebuilt = compact_snapshot(rebuilt_snapshot);
    if &normalized_rebuilt != exported_snapshot {
        fail("replayed world snapshot differs from the snapshot embedded in the package");
    }
    if rebuilt.current_tick() != tick {
        fail(format!("replayed tick mismatch: package={tick}, reconstructed={}", rebuilt.current_tick()));
    }

    println!("{}", serde_json::to_string_pretty(&json!({
        "status":"verified",
        "format":"wanxiang-replay-v1",
        "seed":seed,
        "tick":tick,
        "operation_count":operations.len(),
        "snapshot_match":true,
        "checkpoint_match":true,
        "replay_verified":true,
        "source_file":path
    })).expect("verification report must serialize"));
}
