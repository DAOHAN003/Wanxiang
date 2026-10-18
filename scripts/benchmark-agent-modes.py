#!/usr/bin/env python3
"""Compare live Agent retrieval modes on a paired, fixed task suite.

Run three isolated live_server instances with the same model/provider, seed,
starting tick and WANXIANG_TICK_MS=0, using
WANXIANG_AGENT_RETRIEVAL=no-memory|memory|causal. Calls a model provider and may
incur costs. All three server worlds are preflighted before the first Agent call.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import math
import json
from pathlib import Path
import statistics
import sys
import time
import urllib.error
import urllib.request
from typing import Any


TASKS = [
    {
        "name": "inspect_world_info",
        "task": "Call world_info. Report only the observed seed, current tick, entity count, and conservation flag. Do not advance or mutate the live world.",
    },
    {
        "name": "inspect_snapshot",
        "task": "Call world_snapshot. Report the observed entity count, faction count, event count, and current tick. Do not advance or mutate the live world.",
    },
    {
        "name": "counterfactual_experiment",
        "task": "Call world_experiment with rule energy_density, value 0.95, ticks 8. Report source_tick, ticks simulated per branch, and whether source_world_unchanged is true. Never call world_step.",
    },
]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def request(base: str, path: str, method: str = "GET", body: dict[str, Any] | None = None,
            token: str | None = None, key: str | None = None) -> tuple[int, dict[str, Any], float]:
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if key:
        headers["Idempotency-Key"] = key
    payload = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    req = urllib.request.Request(base.rstrip("/") + path, data=payload, headers=headers, method=method)
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=180) as response:
            status, raw = response.status, response.read()
    except urllib.error.HTTPError as exc:
        status, raw = exc.code, exc.read()
    elapsed_ms = (time.perf_counter() - started) * 1000
    try:
        value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AssertionError(f"{base}{path} returned invalid JSON (HTTP {status}): {exc}") from exc
    if not isinstance(value, dict):
        raise AssertionError(f"{base}{path} returned non-object JSON")
    return status, value, elapsed_ms


def summarize(records: list[dict[str, Any]]) -> dict[str, Any]:
    latencies = [float(r["latency_ms"]) for r in records]
    ordered = sorted(latencies)
    p95 = ordered[max(0, min(len(ordered) - 1, math.ceil(0.95 * len(ordered)) - 1))] if ordered else 0
    count = len(records)
    return {
        "tasks": count,
        "success_rate": round(sum(r["success"] is True for r in records) / count, 4) if count else 0,
        "source_unchanged_rate": round(sum(r["source_unchanged"] is True for r in records) / count, 4) if count else 0,
        "tool_error_steps": sum(int(r["failed_tool_steps"]) for r in records),
        "average_tool_steps": round(statistics.mean(float(r["tool_steps"]) for r in records), 3) if records else 0,
        "mean_latency_ms": round(statistics.mean(latencies), 3) if latencies else 0,
        "p95_latency_ms": round(p95, 3),
    }


def snapshot_metadata(state: dict[str, Any]) -> dict[str, Any]:
    """Extract and validate the exact world identity used to pair benchmark modes."""
    snapshot = state.get("snapshot")
    if not isinstance(snapshot, dict):
        raise AssertionError("state payload is missing snapshot")
    meta = snapshot.get("meta")
    info = state.get("info")
    if not isinstance(meta, dict) or not isinstance(info, dict):
        raise AssertionError("state payload is missing snapshot.meta or info")
    seed = meta.get("seed")
    tick = meta.get("tick")
    info_tick = info.get("tick")
    if isinstance(seed, bool) or not isinstance(seed, (int, str)) or seed == "":
        raise AssertionError(f"invalid snapshot seed: {seed!r}")
    if isinstance(tick, bool) or not isinstance(tick, int) or tick < 0:
        raise AssertionError(f"invalid snapshot tick: {tick!r}")
    if isinstance(info_tick, bool) or not isinstance(info_tick, int) or info_tick != tick:
        raise AssertionError(f"state tick {info_tick!r} differs from snapshot tick {tick!r}")
    canonical = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "seed": seed,
        "tick": tick,
        "snapshot_sha256": hashlib.sha256(canonical).hexdigest(),
        "snapshot": snapshot,
    }


def validate_paired_worlds(observations: dict[str, dict[str, Any]],
                           expected_seed: int | None = None,
                           expected_tick: int | None = None) -> dict[str, Any]:
    """Fail before any model call unless all modes start from the same seed and tick."""
    if set(observations) != {"no-memory", "memory", "causal"}:
        raise AssertionError("preflight requires exactly no-memory, memory and causal modes")
    baseline = observations["no-memory"]
    seed, tick = baseline["seed"], baseline["tick"]
    for mode, item in observations.items():
        if type(item["seed"]) is not type(seed) or item["seed"] != seed:
            raise AssertionError(f"seed mismatch before provider calls: no-memory={seed!r}, {mode}={item['seed']!r}")
        if type(item["tick"]) is not type(tick) or item["tick"] != tick:
            raise AssertionError(f"tick mismatch before provider calls: no-memory={tick!r}, {mode}={item['tick']!r}")
        if item["snapshot_sha256"] != baseline["snapshot_sha256"]:
            raise AssertionError(
                f"initial world snapshot mismatch before provider calls: no-memory and {mode} are not identical"
            )
    if expected_seed is not None and (type(seed) is not int or seed != expected_seed):
        raise AssertionError(f"expected seed {expected_seed}, got {seed!r}")
    if expected_tick is not None and tick != expected_tick:
        raise AssertionError(f"expected starting tick {expected_tick}, got {tick!r}")
    return {
        "seed": seed,
        "starting_tick": tick,
        "same_seed": True,
        "same_starting_tick": True,
        "modes": {
            mode: {"seed": item["seed"], "tick": item["tick"], "snapshot_sha256": item["snapshot_sha256"]}
            for mode, item in observations.items()
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-memory-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=no-memory")
    parser.add_argument("--memory-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=memory")
    parser.add_argument("--causal-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=causal")
    parser.add_argument("--token", default=None)
    parser.add_argument("--expected-seed", type=int, default=None, help="optional additional assertion against the configured world seed")
    parser.add_argument("--expected-tick", type=int, default=None, help="optional additional assertion against the common starting tick")
    parser.add_argument("--provider-label", default="not-specified", help="operator-declared provider label; not verified by the server")
    parser.add_argument("--model-label", default="not-specified", help="operator-declared model label; do not put credentials here")
    parser.add_argument("--output", type=Path, default=None, help="also write the complete JSON report to this path")
    args = parser.parse_args()
    modes = {
        "no-memory": args.no_memory_url,
        "memory": args.memory_url,
        "causal": args.causal_url,
    }
    started_at = utc_now()
    run_id = f"agent-eval-{time.time_ns()}"
    report: dict[str, Any] = {
        "benchmark": "wanxiang_agent_retrieval_modes",
        "schema_version": 1,
        "run_id": run_id,
        "started_at": started_at,
        "finished_at": None,
        "provider_calls_expected": True,
        "may_incur_provider_cost": True,
        "note": "Results are provider/model dependent. This measures integration behavior, latency and world-state safety invariants, not semantic truth or general intelligence.",
        "pairing": None,
        "configuration": {
            "task_names": [task["name"] for task in TASKS],
            "mode_names": list(modes),
            "expected_seed": args.expected_seed,
            "expected_tick": args.expected_tick,
            "provider_label": args.provider_label,
            "model_label": args.model_label,
            "provider_and_model_labels_are_operator_declared": True,
            "auto_tick_ms_required": 0,
        },
        "modes": {},
    }

    # Phase 1: preflight every mode and validate pairing before the first billable Agent call.
    health_by_mode: dict[str, dict[str, Any]] = {}
    observations: dict[str, dict[str, Any]] = {}
    for mode, base in modes.items():
        status, health, _ = request(base, "/health", token=args.token)
        if status != 200 or health.get("status") != "ok":
            raise AssertionError(f"{mode} health failed: HTTP {status} {health}")
        if not health.get("llm_configured"):
            raise AssertionError(f"{mode} server has no LLM configured")
        if isinstance(health.get("auto_tick_ms"), bool) or health.get("auto_tick_ms") != 0:
            raise AssertionError(f"{mode} needs WANXIANG_TICK_MS=0 to make source-world comparisons reproducible")
        persistence = health.get("persistence")
        if not isinstance(persistence, dict) or persistence.get("degraded") is not False:
            raise AssertionError(f"{mode} persistence is degraded or unreported: {persistence}")
        reported_mode = health.get("agent_retrieval_mode")
        if reported_mode and not (
            (mode == "no-memory" and reported_mode in ("no-memory", "none"))
            or reported_mode == mode
        ):
            raise AssertionError(f"{mode} URL reports retrieval mode {reported_mode}")
        state_status, state, _ = request(base, "/api/state", token=args.token)
        if state_status != 200:
            raise AssertionError(f"{mode} preflight state failed: HTTP {state_status} {state}")
        health_by_mode[mode] = health
        observations[mode] = snapshot_metadata(state)

    report["pairing"] = validate_paired_worlds(observations, args.expected_seed, args.expected_tick)
    report["configuration"]["retrieval_modes"] = {
        mode: health.get("agent_retrieval_mode", mode) for mode, health in health_by_mode.items()
    }

    all_ok = True
    for mode, base in modes.items():
        records: list[dict[str, Any]] = []
        expected_snapshot = observations[mode]["snapshot"]
        for task in TASKS:
            st_before, before, _ = request(base, "/api/state", token=args.token)
            if st_before != 200:
                raise AssertionError(f"{mode}/{task['name']} pre-state failed: {before}")
            before_snapshot = before.get("snapshot")
            if before_snapshot != expected_snapshot:
                raise AssertionError(f"{mode}/{task['name']} world changed between preflight and task start")
            key = f"{run_id}-{mode}-{task['name']}"
            status, result, latency = request(
                base, "/api/agent", "POST", {"task": task["task"]}, args.token, key
            )
            if status != 200:
                raise AssertionError(f"{mode}/{task['name']} agent call failed: HTTP {status} {result}")
            st_after, after, _ = request(base, "/api/state", token=args.token)
            if st_after != 200:
                raise AssertionError(f"{mode}/{task['name']} post-state failed: {after}")
            after_snapshot = after.get("snapshot")
            unchanged = before_snapshot == after_snapshot
            raw_steps = result.get("steps")
            steps = raw_steps if isinstance(raw_steps, list) else []
            record = {
                "task": task["name"],
                "latency_ms": round(latency, 3),
                "success": result.get("success") is True,
                "tool_steps": len(steps),
                "failed_tool_steps": sum(
                    1 for step in steps
                    if not isinstance(step, dict) or step.get("success") is not True
                ),
                "source_unchanged": unchanged,
                "error": result.get("error"),
                "answer": result.get("answer"),
            }
            if record["success"] is not True or unchanged is not True:
                all_ok = False
            records.append(record)
        report["modes"][mode] = {
            "summary": summarize(records),
            "records": records,
        }

    report["finished_at"] = utc_now()
    serialized = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(serialized, encoding="utf-8")
    print(serialized, end="")
    if args.output is not None:
        print(f"Report written to {args.output}", file=sys.stderr)
    return 0 if all_ok else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
