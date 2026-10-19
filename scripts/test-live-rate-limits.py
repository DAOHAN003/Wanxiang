#!/usr/bin/env python3
"""Integration-test live API route throttling and health telemetry."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


def request(base, path, method="GET", body=None):
    headers = {"Accept": "application/json"}
    payload = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        payload = json.dumps(body, separators=(",", ":")).encode("utf-8")
    req = urllib.request.Request(base.rstrip("/") + path, data=payload, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=15) as response:
            status, response_headers, raw = response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as exc:
        status, response_headers, raw = exc.code, dict(exc.headers), exc.read()
    try:
        value = json.loads(raw.decode("utf-8")) if raw else {}
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AssertionError(f"{path} returned non-JSON (HTTP {status}): {exc}") from exc
    if not isinstance(value, dict):
        raise AssertionError(f"{path} returned non-object JSON")
    return status, response_headers, value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--server-path", required=True)
    parser.add_argument("--port", type=int, default=8792)
    args = parser.parse_args()
    binary = Path(args.server_path).resolve()
    if not binary.is_file():
        raise FileNotFoundError(f"server executable not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="wanxiang-rate-limit-") as tmp:
        root = Path(tmp)
        env = os.environ.copy()
        env.update({
            "WANXIANG_BIND": f"127.0.0.1:{args.port}",
            "WANXIANG_TICK_MS": "0",
            "WANXIANG_DATA_DIR": str(root / "state"),
            "WANXIANG_RATE_LIMIT_AGENT_PER_MIN": "2",
            "WANXIANG_RATE_LIMIT_EXPERIMENT_PER_MIN": "1",
            "WANXIANG_RATE_LIMIT_STREAM_PER_MIN": "2",
            "WANXIANG_RATE_LIMIT_SESSIONS_PER_MIN": "2",
            "WANXIANG_RATE_LIMIT_BURST": "1",
        })
        for name in (
            "WANXIANG_LLM_ENDPOINT", "WANXIANG_LLM_MODEL", "WANXIANG_LLM_API_KEY",
            "WANXIANG_API_TOKEN",
        ):
            env.pop(name, None)

        log_path = root / "server.log"
        with log_path.open("ab") as log:
            process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT)
            base = f"http://127.0.0.1:{args.port}"
            try:
                ready = False
                deadline = time.monotonic() + 25
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError(f"server exited early: {log_path.read_text(encoding='utf-8', errors='replace')}")
                    try:
                        status, _, health = request(base, "/health")
                        if status == 200 and health.get("status") == "ok":
                            ready = True
                            break
                    except (OSError, ValueError, AssertionError):
                        pass
                    time.sleep(0.2)
                assert ready, "server did not become healthy"

                status, _, first = request(
                    base, "/api/experiment", method="POST",
                    body={"rule": "energy_density", "value": 0.95, "ticks": 1},
                )
                assert status == 200, f"first experiment should pass, got {status}: {first}"

                status, headers, limited = request(
                    base, "/api/experiment", method="POST",
                    body={"rule": "energy_density", "value": 0.35, "ticks": 1},
                )
                assert status == 429, f"second experiment should be rate limited, got {status}: {limited}"
                retry_after = int(headers.get("Retry-After", headers.get("retry-after", "0")))
                assert retry_after >= 1, f"429 must include positive Retry-After, got {headers}"
                assert limited.get("route") == "experiment", f"incorrect rate-limit route: {limited}"
                assert limited.get("limit_per_minute") == 1, f"configured limit absent from 429: {limited}"
                assert limited.get("retry_after_seconds") == retry_after, (
                    f"Retry-After header/body differ: {retry_after} vs {limited}"
                )

                status, _, health = request(base, "/health")
                assert status == 200, f"health request failed: {health}"
                telemetry = health.get("telemetry", {})
                assert telemetry.get("rate_limited_requests") == 1, (
                    f"health telemetry should count rate-limited calls: {telemetry}"
                )
                assert telemetry.get("requests_after_auth_gate", 0) >= 3, (
                    f"post-auth-gate request counter should be visible: {telemetry}"
                )
                limits = telemetry.get("rate_limits", {})
                assert limits.get("scope") == "instance-wide-per-route", limits
                assert limits.get("experiment_per_minute") == 1, limits
                assert "token" not in json.dumps(telemetry).lower(), "health telemetry must not expose credentials"

                print(json.dumps({
                    "status": "passed",
                    "checks": [
                        "configured experiment burst permits one request",
                        "excess request returns HTTP 429",
                        "Retry-After header matches response body",
                        "health endpoint reports safe route-limit configuration",
                        "health telemetry increments rate-limit counter without exposing secrets",
                    ],
                    "retry_after_seconds": retry_after,
                    "telemetry": telemetry,
                }, indent=2))
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
