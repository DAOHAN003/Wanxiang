#!/usr/bin/env python3
"""Exercise deterministic replay and conservation across multiple tick horizons."""

import json
import math
import subprocess
import sys

HORIZONS = (1, 25, 100, 250)
EXPECTED_SEEDS = (7, 42, 2026, 9001)


def run_benchmark(horizon: int) -> dict:
    completed = subprocess.run(
        [
            "cargo",
            "run",
            "--locked",
            "-p",
            "wanxiang_sdk",
            "--example",
            "world_lab_benchmark",
            "--",
            str(horizon),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(
            f"benchmark output for horizon {horizon} is not valid JSON"
        ) from exc


def main() -> None:
    case_count = 0
    for horizon in HORIZONS:
        report = run_benchmark(horizon)
        assert report.get("benchmark") == "wanxiang_world_lab"
        assert report.get("schema_version") == 1
        assert report.get("horizon") == horizon
        assert report.get("passed") is True
        assert report.get("seeds") == list(EXPECTED_SEEDS)
        assert report.get("seed_count") == len(EXPECTED_SEEDS)
        cases = report.get("cases")
        assert isinstance(cases, list) and len(cases) == len(EXPECTED_SEEDS)

        actual_seeds = tuple(case.get("seed") for case in cases)
        assert actual_seeds == EXPECTED_SEEDS, (horizon, actual_seeds)
        for case in cases:
            seed = case["seed"]
            assert case["ticks_per_branch"] == horizon, (seed, horizon)
            assert case["deterministic_replay"] is True, (seed, horizon)
            for branch in ("baseline", "intervention"):
                result = case[branch]
                assert result["tick"] == horizon, (seed, horizon, branch)
                assert result["snapshot_tick_matches"] is True, (seed, horizon, branch)
                assert result["conserved"] is True, (seed, horizon, branch)
                drift = float(result["drift"])
                # Drift measures per-tick observation/routing discrepancy; the independent
                # conserved flag checks the four-pool total. Interventions may have finite,
                # non-zero drift without violating that total invariant.
                assert math.isfinite(drift), (seed, horizon, branch, result["drift"])
            assert case["diff"]["rules_differ"] is True, (seed, horizon)
            case_count += 1

    print(
        f"Verified {case_count} seed/horizon cases across "
        f"{len(HORIZONS)} horizons: deterministic replay, tick alignment, "
        "resource conservation, and rule intervention."
    )


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, KeyError, TypeError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"multi-horizon world regression failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
