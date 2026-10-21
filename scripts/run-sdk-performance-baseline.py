#!/usr/bin/env python3
"""Build the SDK benchmark and measure the benchmark process' peak RSS on Linux."""

import argparse
import json
import platform
import re
import subprocess
import sys
import time
from pathlib import Path

TIME_BINARY = Path("/usr/bin/time")
BENCHMARK_BINARY = Path("target/release/examples/sdk_performance_baseline")


def parse_time(stderr: str, label: str) -> float:
    match = re.search(rf"^{re.escape(label)}:\s*([0-9.]+)\s*$", stderr, re.MULTILINE)
    if match is None:
        raise ValueError(f"GNU time output did not contain {label!r}")
    return float(match.group(1))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ticks", type=int, default=10, choices=range(1, 1001))
    parser.add_argument("--repeats", type=int, default=3, choices=range(1, 11))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    if not sys.platform.startswith("linux"):
        raise SystemExit("peak-memory measurement currently requires Linux GNU /usr/bin/time")
    if not TIME_BINARY.is_file():
        raise SystemExit("GNU /usr/bin/time is required to measure peak resident memory")

    started = time.perf_counter()
    subprocess.run(
        [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "wanxiang_sdk",
            "--example",
            "sdk_performance_baseline",
        ],
        check=True,
    )
    build_seconds = time.perf_counter() - started

    if not BENCHMARK_BINARY.is_file():
        raise SystemExit(f"benchmark binary not found: {BENCHMARK_BINARY}")

    timed = subprocess.run(
        [
            str(TIME_BINARY),
            "-v",
            str(BENCHMARK_BINARY.resolve()),
            str(args.ticks),
            str(args.repeats),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    try:
        report = json.loads(timed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"benchmark produced invalid JSON: {exc}") from exc

    rss = re.search(
        r"^\s*Maximum resident set size \(kbytes\):\s*(\d+)\s*$",
        timed.stderr,
        re.MULTILINE,
    )
    if rss is None:
        raise SystemExit("GNU time output did not include maximum resident set size")

    report["runtime_measurements"] = {
        "method": "GNU /usr/bin/time -v; benchmark binary only, excluding compilation",
        "platform": platform.platform(),
        "release_build_seconds": round(build_seconds, 6),
        "process_max_rss_kib": int(rss.group(1)),
        "user_cpu_seconds": parse_time(timed.stderr, "User time (seconds)"),
        "system_cpu_seconds": parse_time(timed.stderr, "System time (seconds)"),
    }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(
        "Measured SDK benchmark process peak RSS: "
        f"{report['runtime_measurements']['process_max_rss_kib']} KiB"
    )


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, OSError, ValueError) as exc:
        print(f"SDK performance measurement failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
