#!/usr/bin/env python3
"""Deterministic smoke/evaluation suite for the running Wanxiang HTTP API."""
from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from typing import Any


def request(base: str, path: str, method: str = "GET", body: dict[str, Any] | None = None,
            token: str | None = None, idempotency_key: str | None = None) -> tuple[int, dict[str, Any], float]:
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if idempotency_key:
        headers["Idempotency-Key"] = idempotency_key
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    req = urllib.request.Request(base.rstrip("/") + path, data=data, headers=headers, method=method)
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            status = resp.status
            payload = resp.read()
    except urllib.error.HTTPError as exc:
        status = exc.code
        payload = exc.read()
    elapsed_ms = (time.perf_counter() - started) * 1000
    try:
        parsed = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AssertionError(f"{path} returned non-JSON payload (HTTP {status}): {exc}") from exc
    if not isinstance(parsed, dict):
        raise AssertionError(f"{path} returned a non-object JSON payload")
    return status, parsed, elapsed_ms


def check(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:8787")
    parser.add_argument("--token", default=None)
    parser.add_argument("--agent", action="store_true", help="also run a provider-backed world_info task")
    args = parser.parse_args()
    measurements: list[dict[str, Any]] = []

    status, health, ms = request(args.base_url, "/health", token=args.token)
    check(status == 200 and health.get("status") == "ok", f"health failed: HTTP {status} {health}")
    measurements.append({"name": "health", "latency_ms": round(ms, 3)})

    status, initial, ms = request(args.base_url, "/api/state", token=args.token)
    check(status == 200 and initial.get("backend") == "wanxiang_sdk::WorldModel", "state did not come from Rust WorldModel")
    snapshot = initial.get("snapshot")
    check(isinstance(snapshot, dict) and isinstance(snapshot.get("meta"), dict), "state snapshot missing metadata")
    initial_tick = int(initial["info"]["tick"])
    measurements.append({"name": "state", "latency_ms": round(ms, 3), "tick": initial_tick})

    status, invalid, ms = request(args.base_url, "/api/step", "POST", {"ticks": 101}, args.token)
    check(status == 400, f"invalid step should return HTTP 400, got {status}")
    status, after_invalid, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_invalid["info"]["tick"]) == initial_tick, "invalid step mutated world tick")
    measurements.append({"name": "invalid-step-no-mutation", "latency_ms": round(ms, 3)})

    rules = snapshot.get("rules", {})
    current = float(rules.get("energy_density", 0.5))
    intervention = 0.35 if abs(current - 0.95) < 1e-9 else 0.95
    experiment = {"rule": "energy_density", "value": intervention, "ticks": 12}
    status, result_a, ms = request(args.base_url, "/api/experiment", "POST", experiment, args.token)
    check(status == 200, f"experiment failed: HTTP {status} {result_a}")
    status, result_b, ms_repeat = request(args.base_url, "/api/experiment", "POST", experiment, args.token)
    check(status == 200, f"repeat experiment failed: HTTP {status} {result_b}")
    check(result_a.get("baseline") == result_b.get("baseline"), "repeated baseline branch is not deterministic")
    check(result_a.get("altered") == result_b.get("altered"), "repeated altered branch is not deterministic")
    check(result_a.get("diff") == result_b.get("diff"), "repeated diff is not deterministic")
    check(result_a.get("source_world_unchanged") is True, "experiment reports source world changed")
    for label in ("baseline", "altered"):
        ledger = result_a[label].get("resources", {}).get("conservation", {})
        check(ledger.get("conserved") is True, f"{label} branch failed conservation check")
    measurements.append({"name": "experiment-repeat-determinism", "latency_ms": round(ms, 3),
                         "repeat_latency_ms": round(ms_repeat, 3), "ticks_per_branch": 12})

    status, after_experiment, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_experiment["info"]["tick"]) == initial_tick, "experiment mutated the live world's tick")

    idempotency_key = f"wanxiang-eval-step-{time.time_ns()}"
    status_a, step_a, ms_a = request(args.base_url, "/api/step", "POST", {"ticks": 1},
                                    args.token, idempotency_key)
    status_b, step_b, ms_b = request(args.base_url, "/api/step", "POST", {"ticks": 1},
                                    args.token, idempotency_key)
    check(status_a == 200 and status_b == 200, f"idempotent step failed: {status_a}, {status_b}")
    check(step_a.get("tick") == step_b.get("tick"), "duplicate Idempotency-Key advanced world twice")
    status, after_step, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_step["info"]["tick"]) == initial_tick + 1, "single idempotent step did not advance exactly once")
    measurements.append({"name": "idempotency-exactly-once", "first_latency_ms": round(ms_a, 3),
                         "repeat_latency_ms": round(ms_b, 3)})

    if args.agent:
        task = "Use world_info to inspect the current seed, tick and conservation status. Do not advance the world. Report only those observed values."
        status, agent, ms = request(args.base_url, "/api/agent", "POST", {"task": task}, args.token)
        check(status == 200, f"agent route unavailable: HTTP {status} {agent}")
        check(isinstance(agent.get("steps"), list), "agent response omitted auditable tool steps")
        measurements.append({"name": "agent-world-info", "latency_ms": round(ms, 3),
                             "success": agent.get("success"), "steps": len(agent.get("steps", []))})

    print(json.dumps({"status": "passed", "base_url": args.base_url, "checks": measurements,
                      "final_tick": int(after_step["info"]["tick"])}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
