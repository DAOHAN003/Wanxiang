#!/usr/bin/env python3
"""Verify bearer-token enforcement and standards-compliant CORS preflight."""
from __future__ import annotations
import argparse, json, os, subprocess, sys, tempfile, time, urllib.error, urllib.request
from pathlib import Path


def request(base, path, method="GET", token=None, origin=None):
    headers = {"Accept": "application/json"}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if origin is not None:
        headers["Origin"] = origin
    req = urllib.request.Request(base.rstrip("/") + path, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=5) as response:
            status, response_headers, payload = response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as exc:
        status, response_headers, payload = exc.code, dict(exc.headers), exc.read()
    value = json.loads(payload.decode("utf-8")) if payload else {}
    return status, response_headers, payload, value


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--server-path", required=True)
    parser.add_argument("--port", type=int, default=8790)
    args = parser.parse_args()
    binary = Path(args.server_path).resolve()
    if not binary.is_file():
        raise FileNotFoundError(f"server executable not found: {binary}")
    token, origin = "ci-only-live-auth-token", "https://trusted.example"
    with tempfile.TemporaryDirectory(prefix="wanxiang-auth-test-") as tmp:
        log_path = Path(tmp) / "server.log"
        env = os.environ.copy()
        env.update({
            "WANXIANG_BIND": f"127.0.0.1:{args.port}",
            "WANXIANG_TICK_MS": "0",
            "WANXIANG_DATA_DIR": str(Path(tmp) / "state"),
            "WANXIANG_API_TOKEN": token,
            "WANXIANG_CORS_ORIGIN": origin,
        })
        for name in ("WANXIANG_LLM_ENDPOINT", "WANXIANG_LLM_MODEL", "WANXIANG_LLM_API_KEY"):
            env.pop(name, None)
        with log_path.open("ab") as log:
            process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT)
            base = f"http://127.0.0.1:{args.port}"
            try:
                ready = False
                deadline = time.monotonic() + 25
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError(f"server exited: {log_path.read_text(encoding='utf-8', errors='replace')}")
                    try:
                        status, _, _, health = request(base, "/health", token=token)
                        if status == 200 and health.get("status") == "ok":
                            ready = True
                            break
                    except (OSError, ValueError, json.JSONDecodeError):
                        pass
                    time.sleep(0.2)
                assert ready, "authenticated health endpoint did not become ready"

                status, _, _, body = request(base, "/health")
                assert status == 401, f"missing token must return 401, got {status} {body}"
                status, _, _, body = request(base, "/health", token=token)
                assert status == 200 and body.get("status") == "ok", f"valid token rejected: {status} {body}"
                status, _, _, body = request(base, "/health", token=token, origin="https://attacker.example")
                assert status == 403, f"untrusted origin must return 403, got {status} {body}"
                status, headers, payload, _ = request(base, "/api/step", method="OPTIONS", origin=origin)
                assert status == 204, f"trusted preflight must return 204, got {status}"
                assert headers.get("Access-Control-Allow-Origin") == origin, "exact CORS allow-origin missing"
                assert headers.get("Content-Length") == "0", f"204 must have empty body length, got {headers.get('Content-Length')}"
                assert payload == b"", f"204 must not contain a response body: {payload!r}"
                print(json.dumps({"status":"passed","checks":[
                    "missing bearer token rejected",
                    "valid bearer token accepted",
                    "untrusted origin rejected",
                    "trusted CORS preflight returns an empty 204"
                ]}, indent=2))
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
