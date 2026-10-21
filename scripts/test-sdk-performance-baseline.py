#!/usr/bin/env python3
"""Validate SDK performance baseline structure and model invariants."""

import json
import math
import sys
from pathlib import Path

EXPECTED = {
    "small": {"characters": 4, "locations": 3, "factions": 3},
    "medium": {"characters": 29, "locations": 13, "factions": 3},
    "large": {"characters": 104, "locations": 43, "factions": 3},
}
MEASUREMENTS = (
    "construct_us",
    "simulate_us",
    "ticks_per_second",
    "snapshot_export_us",
    "checkpoint_encode_us",
    "checkpoint_restore_us",
)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: python scripts/test-sdk-performance-baseline.py REPORT.json")
    report = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    assert report.get("benchmark") == "wanxiang_sdk_performance"
    assert report.get("schema_version") == 1
    assert report.get("profile") == "release"
    ticks = report.get("tick_horizon")
    repeats = report.get("repeats")
    assert isinstance(ticks, int) and 1 <= ticks <= 1_000
    assert isinstance(repeats, int) and 1 <= repeats <= 10
    assert report.get("timing_policy")

    workloads = report.get("workloads")
    assert isinstance(workloads, list) and len(workloads) == len(EXPECTED)
    assert [item.get("name") for item in workloads] == list(EXPECTED)

    previous_characters = 0
    previous_locations = 0
    for workload in workloads:
        name = workload["name"]
        assert workload["tick_horizon"] == ticks
        configured = workload["configured_counts"]
        assert configured == EXPECTED[name], (name, configured)
        assert configured["characters"] >= previous_characters
        assert configured["locations"] >= previous_locations
        previous_characters = configured["characters"]
        previous_locations = configured["locations"]

        observed = workload["observed_counts"]
        assert observed["living_characters_after_simulation"] >= 1
        assert observed["locations"] == configured["locations"]
        assert isinstance(observed["factions"], int) and observed["factions"] >= 0
        assert isinstance(observed["events"], int) and observed["events"] >= 0

        integrity = workload["integrity"]
        assert integrity["snapshot_tick_matches"] is True
        assert integrity["resource_conserved"] is True
        assert integrity["checkpoint_replay_equal"] is True
        drift = float(integrity["conservation_drift"])
        assert math.isfinite(drift) and abs(drift) < 1e-4, (name, drift)

        assert workload["output_bytes"]["snapshot_json"] > 0
        assert workload["output_bytes"]["checkpoint_json"] > 0
        samples = workload.get("samples")
        assert isinstance(samples, list) and len(samples) == repeats
        for sample in samples:
            for field in MEASUREMENTS:
                value = sample[field]
                assert isinstance(value, (int, float)) and math.isfinite(value), (name, field, value)
                assert value >= 0, (name, field, value)
            assert sample["ticks_per_second"] > 0

        medians = workload.get("median", {})
        for field in MEASUREMENTS:
            value = medians[field]
            assert isinstance(value, (int, float)) and math.isfinite(value)
            assert value >= 0
        assert medians["ticks_per_second"] > 0

    print("Validated release SDK performance report: 3 workload sizes, replay equality, and resource conservation.")


if __name__ == "__main__":
    main()
