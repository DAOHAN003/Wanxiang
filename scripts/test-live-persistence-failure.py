#!/usr/bin/env python3
"""Prove persistence failures are surfaced instead of reported as healthy."""
from __future__ import annotations
import argparse, json, os, subprocess, sys, tempfile, time, urllib.error, urllib.request
from pathlib import Path


def request(base, path, method="GET", body=None, idempotency_key=None):
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    headers = {"Accept": "application/json", "Cache-Control": "no-cache"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if idempotency_key is not None:
        headers["Idempotency-Key"] = idempotency_key
    req = urllib.request.Request(base.rstrip("/") + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=5) as response:
            status, payload = response.status, response.read()
    except urllib.error.HTTPError as exc:
        status, payload = exc.code, exc.read()
    parsed = json.loads(payload.decode("utf-8")) if payload else {}
    return status, parsed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--server-path", required=True)
    parser.add_argument("--port", type=int, default=8791)
    args = parser.parse_args()
    binary = Path(args.server_path).resolve()
    if not binary.is_file():
        raise FileNotFoundError(f"server executable not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="wanxiang-persistence-failure-") as tmp:
        root = Path(tmp)
        blocker = root / "not-a-directory"
        blocker.write_text("intentionally blocks creation of nested persistence directory", encoding="utf-8")
        data_dir = blocker / "nested" / "state"
        log_path = root / "server.log"
        env = os.environ.copy()
        env.update({
            "WANXIANG_BIND": f"127.0.0.1:{args.port}",
            "WANXIANG_TICK_MS": "0",
            "WANXIANG_DATA_DIR": str(data_dir),
        })
        for name in ("WANXIANG_LLM_ENDPOINT", "WANXIANG_LLM_MODEL", "WANXIANG_LLM_API_KEY", "WANXIANG_API_TOKEN"):
            env.pop(name, None)
        with log_path.open("ab") as log:
            process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT)
            base = f"http://127.0.0.1:{args.port}"
            try:
                deadline = time.monotonic() + 25
                ready = False
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError(f"server exited: {log_path.read_text(encoding='utf-8', errors='replace')}")
                    try:
                        status, health = request(base, "/health")
                        if status == 200 and health.get("status") == "ok":
                            ready = True
                            break
                    except (OSError, ValueError, json.JSONDecodeError):
                        pass
                    time.sleep(0.2)
                assert ready, "server did not start for persistence-failure test"

                key = "persistence-failure-idempotency-key"
                status, body = request(
                    base, "/api/step", method="POST", body={"ticks": 1}, idempotency_key=key
                )
                assert status == 500, f"step should report persistence failure as HTTP 500, got {status} {body}"
                status, first_state = request(base, "/api/state")
                assert status == 200, f"state request failed: {first_state}"
                first_tick = int(first_state["info"]["tick"])
                retry_status, retry_body = request(
                    base, "/api/step", method="POST", body={"ticks": 1}, idempotency_key=key
                )
                assert retry_status == 500, (
                    f"retry with the same key must replay the failure, got {retry_status} {retry_body}"
                )
                status, state = request(base, "/api/state")
                assert status == 200, f"state request after retry failed: {state}"
                assert int(state["info"]["tick"]) == first_tick, (
                    "retry with the same Idempotency-Key re-executed a failed durable write"
                )
                persistence = state.get("persistence", {})
                assert persistence.get("degraded") is True, f"persistence failure was not exposed: {persistence}"
                assert persistence.get("world_checkpoint_degraded") is True, (
                    f"world checkpoint failure was not identified: {persistence}"
                )
                ready_status, readiness = request(base, "/ready")
                assert ready_status == 503 and readiness.get("status") == "degraded", (
                    f"readiness should fail while checkpoint persistence is degraded: {ready_status} {readiness}"
                )
                print(json.dumps({
                    "status": "passed",
                    "checks": [
                        "failed checkpoint write returned HTTP 500",
                        "same Idempotency-Key replays failure without repeating mutation",
                        "state API exposed degraded persistence status",
                        "health telemetry identifies world checkpoint as degraded",
                        "readiness endpoint returns 503 while persistence is degraded",
                    ],
                    "persistence": persistence,
                }, indent=2))
            finally:
                if process.poll() is None:
                    process.terminate()
                    try: process.wait(timeout=10)
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
