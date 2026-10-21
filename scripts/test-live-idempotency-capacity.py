#!/usr/bin/env python3
"""Verify that a saturated idempotency cache rejects new writes without evicting receipts."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from typing import Any


def request(
    base: str,
    path: str,
    method: str = "GET",
    body: dict[str, Any] | None = None,
    idempotency_key: str | None = None,
) -> tuple[int, dict[str, Any], dict[str, str]]:
    headers = {"Accept": "application/json", "Cache-Control": "no-cache"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if idempotency_key:
        headers["Idempotency-Key"] = idempotency_key
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    req = urllib.request.Request(
        base.rstrip("/") + path, data=data, headers=headers, method=method
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            status, payload, response_headers = (
                response.status,
                response.read(),
                dict(response.headers.items()),
            )
    except urllib.error.HTTPError as exc:
        status, payload, response_headers = (
            exc.code,
            exc.read(),
            dict(exc.headers.items()),
        )
    parsed = json.loads(payload.decode("utf-8"))
    if not isinstance(parsed, dict):
        raise AssertionError(f"{path} returned a non-object payload")
    return status, parsed, {key.lower(): value for key, value in response_headers.items()}


def wait_ready(base: str, process: subprocess.Popen[bytes], log_path: Path) -> None:
    deadline = time.monotonic() + 25
    last_error = "server did not report ready"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(
                f"server exited with {process.returncode}: "
                f"{log_path.read_text(encoding='utf-8', errors='replace')}"
            )
        try:
            status, health, _ = request(base, "/health")
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
    parser.add_argument("--port", type=int, default=8793)
    args = parser.parse_args()

    server_path = Path(args.server_path).resolve()
    if not server_path.is_file():
        raise FileNotFoundError(f"server executable not found: {server_path}")
    if not 1 <= args.port <= 65535:
        raise ValueError("port must be in 1..=65535")
    base = f"http://127.0.0.1:{args.port}"

    with socket.socket() as sock:
        sock.bind(("127.0.0.1", args.port))

    with tempfile.TemporaryDirectory(prefix="wanxiang-idempotency-capacity-") as tmp:
        data_dir = Path(tmp) / "state"
        log_path = Path(tmp) / "server.log"
        env = os.environ.copy()
        env.update({
            "WANXIANG_BIND": f"127.0.0.1:{args.port}",
            "WANXIANG_TICK_MS": "0",
            "WANXIANG_IDEMPOTENCY_MAX_ENTRIES": "2",
            "WANXIANG_DATA_DIR": str(data_dir),
        })
        for key in ("WANXIANG_LLM_ENDPOINT", "WANXIANG_LLM_MODEL", "WANXIANG_LLM_API_KEY"):
            env.pop(key, None)

        with log_path.open("ab") as log_file:
            process = subprocess.Popen(
                [str(server_path)], env=env, stdout=log_file, stderr=subprocess.STDOUT
            )
            try:
                wait_ready(base, process, log_path)

                status, initial, _ = request(base, "/api/state")
                assert status == 200, f"initial state failed: {initial}"
                initial_tick = int(initial["info"]["tick"])

                first_key = "capacity-test-key-1"
                second_key = "capacity-test-key-2"
                overflow_key = "capacity-test-key-3"
                body = {"ticks": 1}

                status, first, _ = request(
                    base, "/api/step", "POST", body, idempotency_key=first_key
                )
                assert status == 200, f"first keyed mutation failed: {first}"
                status, second, _ = request(
                    base, "/api/step", "POST", body, idempotency_key=second_key
                )
                assert status == 200, f"second keyed mutation failed: {second}"
                assert int(second["tick"]) == initial_tick + 2

                status, rejected, headers = request(
                    base, "/api/step", "POST", body, idempotency_key=overflow_key
                )
                assert status == 503, f"full cache should fail closed: HTTP {status}: {rejected}"
                assert "no operation was executed" in str(rejected.get("error", ""))
                retry_after = int(headers.get("retry-after", "0"))
                assert retry_after > 0, f"Retry-After must be positive: {headers}"

                # Existing keys remain replayable even when no new slot is available.
                status, replayed, _ = request(
                    base, "/api/step", "POST", body, idempotency_key=first_key
                )
                assert status == 200 and replayed == first, (
                    "full cache must replay the exact response for an existing key"
                )

                status, conflict, _ = request(
                    base, "/api/step", "POST", {"ticks": 2}, idempotency_key=first_key
                )
                assert status == 409, f"same key with a different request must return 409: {conflict}"

                status, after, _ = request(base, "/api/state")
                assert status == 200, f"state query failed: {after}"
                assert int(after["info"]["tick"]) == int(second["tick"]), (
                    "the rejected third mutation advanced the world"
                )
                print(
                    "Verified cache saturation: a new key receives 503 without side effects, "
                    "existing receipts replay, key conflicts return 409, and Retry-After is set."
                )
            finally:
                stop(process)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
