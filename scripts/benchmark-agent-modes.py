#!/usr/bin/env python3
"""Compare live Agent retrieval modes on a fixed task suite.

Run three isolated live_server instances with the same model/provider, seed and
WANXIANG_TICK_MS=0, but WANXIANG_AGENT_RETRIEVAL=no-memory|memory|causal.
This intentionally calls a provider and may incur API charges.
"""
from __future__ import annotations

import argparse
import json
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
    return {
        "tasks": len(records),
        "success_rate": round(sum(bool(r["success"]) for r in records) / max(1, len(records)), 4),
        "source_unchanged_rate": round(sum(bool(r["source_unchanged"]) for r in records) / max(1, len(records)), 4),
        "tool_error_steps": sum(int(r["failed_tool_steps"]) for r in records),
        "average_tool_steps": round(statistics.mean(float(r["tool_steps"]) for r in records), 3) if records else 0,
        "mean_latency_ms": round(statistics.mean(latencies), 3) if latencies else 0,
        "p95_latency_ms": round(sorted(latencies)[min(len(latencies) - 1, int(len(latencies) * 0.95))], 3) if latencies else 0,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--no-memory-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=no-memory")
    parser.add_argument("--memory-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=memory")
    parser.add_argument("--causal-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=causal")
    parser.add_argument("--token", default=None)
    args = parser.parse_args()
    modes = {
        "no-memory": args.no_memory_url,
        "memory": args.memory_url,
        "causal": args.causal_url,
    }
    report: dict[str, Any] = {
        "benchmark": "wanxiang_agent_retrieval_modes",
        "schema_version": 1,
        "note": "Results are model/provider dependent; this measures integration behavior, latency and safety invariants, not semantic truth.",
        "modes": {},
    }
    all_ok = True

    for mode, base in modes.items():
        status, health, _ = request(base, "/health", token=args.token)
        if status != 200 or health.get("status") != "ok":
            raise AssertionError(f"{mode} health failed: HTTP {status} {health}")
        if not health.get("llm_configured"):
            raise AssertionError(f"{mode} server has no LLM configured")
        if int(health.get("auto_tick_ms", -1)) != 0:
            raise AssertionError(f"{mode} needs WANXIANG_TICK_MS=0 to make source-world comparisons reproducible")
        reported_mode = health.get("agent_retrieval_mode")
        if reported_mode and not (
            (mode == "no-memory" and reported_mode in ("no-memory", "none"))
            or reported_mode == mode
        ):
            raise AssertionError(f"{mode} URL reports retrieval mode {reported_mode}")

        records: list[dict[str, Any]] = []
        for task in TASKS:
            st_before, before, _ = request(base, "/api/state", token=args.token)
            if st_before != 200:
                raise AssertionError(f"{mode}/{task['name']} pre-state failed: {before}")
            key = f"agent-benchmark-{mode}-{task['name']}-{time.time_ns()}"
            status, result, latency = request(
                base, "/api/agent", "POST", {"task": task["task"]}, args.token, key
            )
            if status != 200:
                raise AssertionError(f"{mode}/{task['name']} agent call failed: HTTP {status} {result}")
            st_after, after, _ = request(base, "/api/state", token=args.token)
            if st_after != 200:
                raise AssertionError(f"{mode}/{task['name']} post-state failed: {after}")
            before_snapshot = before.get("snapshot")
            after_snapshot = after.get("snapshot")
            unchanged = before_snapshot == after_snapshot
            steps = result.get("steps") if isinstance(result.get("steps"), list) else []
            record = {
                "task": task["name"],
                "latency_ms": round(latency, 3),
                "success": bool(result.get("success")),
                "tool_steps": len(steps),
                "failed_tool_steps": sum(1 for step in steps if not step.get("success", False)),
                "source_unchanged": unchanged,
                "error": result.get("error"),
                "answer": result.get("answer"),
            }
            if not record["success"] or not unchanged:
                all_ok = False
            records.append(record)
        report["modes"][mode] = {"summary": summarize(records), "records": records}

    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
