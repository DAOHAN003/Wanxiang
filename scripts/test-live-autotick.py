#!/usr/bin/env python3
"""Verify the live server advances the real world without browser requests."""
from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from typing import Any


def get_json(base: str, path: str) -> tuple[int, dict[str, Any]]:
    request = urllib.request.Request(
        base.rstrip("/") + path,
        headers={"Accept": "application/json", "Cache-Control": "no-cache"},
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            status, payload = response.status, response.read()
    except urllib.error.HTTPError as exc:
        status, payload = exc.code, exc.read()
    value = json.loads(payload.decode("utf-8"))
    if not isinstance(value, dict):
        raise AssertionError(f"{path} did not return a JSON object")
    return status, value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:8788")
    parser.add_argument("--wait-seconds", type=float, default=1.2)
    args = parser.parse_args()

    status, health = get_json(args.base_url, "/health")
    if status != 200 or health.get("status") != "ok":
        raise AssertionError(f"health failed: HTTP {status} {health}")
    interval = int(health.get("auto_tick_ms", 0))
    if interval <= 0:
        raise AssertionError(f"server auto tick must be enabled, got {interval} ms")

    status, before = get_json(args.base_url, "/api/state")
    if status != 200:
        raise AssertionError(f"initial state failed: HTTP {status} {before}")
    tick_before = int(before["info"]["tick"])
    started = time.perf_counter()
    time.sleep(max(args.wait_seconds, interval / 1000.0 * 2.0))
    status, after = get_json(args.base_url, "/api/state")
    elapsed = (time.perf_counter() - started) * 1000.0
    if status != 200:
        raise AssertionError(f"later state failed: HTTP {status} {after}")
    tick_after = int(after["info"]["tick"])
    if tick_after <= tick_before:
        raise AssertionError(
            f"world did not advance without a POST: tick {tick_before} -> {tick_after}"
        )

    print(json.dumps({
        "status": "passed",
        "backend": health.get("backend"),
        "auto_tick_ms": interval,
        "tick_before": tick_before,
        "tick_after": tick_after,
        "tick_delta": tick_after - tick_before,
        "wait_elapsed_ms": round(elapsed, 3),
        "proof": "The server's Rust WorldModel advanced while the test sent no mutation requests."
    }, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
