#!/usr/bin/env python3
"""Exercise world checkpoint and idempotency recovery across a real process restart."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from typing import Any


def request(base: str, path: str, method: str = "GET", body: dict[str, Any] | None = None,
            idempotency_key: str | None = None) -> tuple[int, dict[str, Any]]:
    headers = {"Accept": "application/json", "Cache-Control": "no-cache"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if idempotency_key:
        headers["Idempotency-Key"] = idempotency_key
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    req = urllib.request.Request(base.rstrip("/") + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            status, payload = response.status, response.read()
    except urllib.error.HTTPError as exc:
        status, payload = exc.code, exc.read()
    parsed = json.loads(payload.decode("utf-8"))
    if not isinstance(parsed, dict):
        raise AssertionError(f"{path} returned a non-object payload")
    return status, parsed


def wait_ready(base: str, process: subprocess.Popen[bytes], log_path: Path) -> None:
    deadline = time.monotonic() + 25
    last_error = "server did not report ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(
                f"server exited with {process.returncode}: {log_path.read_text(encoding='utf-8', errors='replace')}"
            )
        try:
            status, health = request(base, "/health")
            if status == 200 and health.get("status") == "ok":
                return
            last_error = f"health returned HTTP {status}: {health}"
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            last_error = str(exc)
        time.sleep(0.2)
    raise AssertionError(f"server did not become ready: {last_error}")


def stop(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--server-path", required=True, help="built live_server executable")
    parser.add_argument("--port", type=int, default=8789)
    args = parser.parse_args()

    server_path = Path(args.server_path).resolve()
    if not server_path.is_file():
        raise FileNotFoundError(f"server executable not found: {server_path}")
    if not 1 <= args.port <= 65535:
        raise ValueError("port must be in 1..=65535")
    base = f"http://127.0.0.1:{args.port}"

    # Fail early if the requested test port is already occupied.
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", args.port))

    with tempfile.TemporaryDirectory(prefix="wanxiang-restart-test-") as tmp:
        data_dir = Path(tmp) / "state"
        log_path = Path(tmp) / "server.log"
        env = os.environ.copy()
        env.update({
            "WANXIANG_BIND": f"127.0.0.1:{args.port}",
            "WANXIANG_TICK_MS": "0",
            "WANXIANG_DATA_DIR": str(data_dir),
        })
        for key in ("WANXIANG_LLM_ENDPOINT", "WANXIANG_LLM_MODEL", "WANXIANG_LLM_API_KEY"):
            env.pop(key, None)

        def start() -> tuple[subprocess.Popen[bytes], Any]:
            log_file = log_path.open("ab")
            process = subprocess.Popen(
                [str(server_path)],
                env=env,
                stdout=log_file,
                stderr=subprocess.STDOUT,
            )
            return process, log_file

        first, first_log = start()
        try:
            wait_ready(base, first, log_path)
            status, before = request(base, "/api/state")
            assert status == 200, f"initial state failed: {before}"
            initial_tick = int(before["info"]["tick"])
            key = "restart-persistence-acceptance-key"
            status, stepped = request(
                base, "/api/step", "POST", {"ticks": 2}, idempotency_key=key
            )
            assert status == 200, f"step failed: {stepped}"
            expected_tick = initial_tick + 2
            assert int(stepped["tick"]) == expected_tick, (
                f"step result tick mismatch: expected {expected_tick}, got {stepped.get('tick')}"
            )
        finally:
            stop(first)
            first_log.close()

        second, second_log = start()
        try:
            wait_ready(base, second, log_path)
            status, restored = request(base, "/api/state")
            assert status == 200, f"restored state failed: {restored}"
            restored_tick = int(restored["info"]["tick"])
            assert restored_tick == expected_tick, (
                f"world checkpoint was not restored: expected tick {expected_tick}, got {restored_tick}"
            )
            persistence = restored.get("persistence", {})
            assert persistence.get("degraded") is False, (
                f"persistence reported degraded after recovery: {persistence}"
            )

            status, replayed = request(
                base, "/api/step", "POST", {"ticks": 2}, idempotency_key=key
            )
            assert status == 200, f"replayed idempotent request failed: {replayed}"
            assert int(replayed["tick"]) == expected_tick, (
                "the persisted idempotency record did not replay the original response"
            )
            status, after_replay = request(base, "/api/state")
            assert status == 200, f"state after replay failed: {after_replay}"
            assert int(after_replay["info"]["tick"]) == expected_tick, (
                "reusing the same Idempotency-Key advanced the world twice after restart"
            )
        finally:
            stop(second)
            second_log.close()

        print(json.dumps({
            "status": "passed",
            "initial_tick": initial_tick,
            "restored_tick": restored_tick,
            "tick_after_duplicate_request": int(after_replay["info"]["tick"]),
            "checks": [
                "world checkpoint restored after process restart",
                "persisted idempotency response restored after process restart",
                "duplicate request did not advance the world twice",
                "persistence health reported non-degraded",
            ],
        }, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
