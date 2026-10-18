#!/usr/bin/env python3
"""Offline unit tests for the provider-backed Agent benchmark preflight."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("benchmark-agent-modes.py")
SPEC = importlib.util.spec_from_file_location("benchmark_agent_modes", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


def state(seed=42, snapshot_tick=0, info_tick=0):
    return {
        "snapshot": {
            "meta": {"seed": seed, "tick": snapshot_tick},
            "rules": {"energy_density": 0.5},
            "entities": [{"id": "entity-1"}],
        },
        "info": {"tick": info_tick},
    }


def observation(seed=42, tick=0, digest="a" * 64):
    return {"seed": seed, "tick": tick, "snapshot_sha256": digest, "snapshot": {}}


class AgentBenchmarkTests(unittest.TestCase):
    def test_extracts_valid_seed_tick_and_stable_hash(self):
        first = BENCH.snapshot_metadata(state())
        second = BENCH.snapshot_metadata({
            "snapshot": {"entities": [{"id": "entity-1"}],
                         "rules": {"energy_density": 0.5},
                         "meta": {"tick": 0, "seed": 42}},
            "info": {"tick": 0},
        })
        self.assertEqual(first["seed"], 42)
        self.assertEqual(first["tick"], 0)
        self.assertEqual(first["snapshot_sha256"], second["snapshot_sha256"])

    def test_rejects_snapshot_info_tick_mismatch(self):
        with self.assertRaisesRegex(AssertionError, "differs from snapshot tick"):
            BENCH.snapshot_metadata(state(snapshot_tick=4, info_tick=5))

    def test_rejects_boolean_seed_and_boolean_tick(self):
        with self.assertRaisesRegex(AssertionError, "invalid snapshot seed"):
            BENCH.snapshot_metadata(state(seed=True))
        with self.assertRaisesRegex(AssertionError, "invalid snapshot tick"):
            BENCH.snapshot_metadata(state(snapshot_tick=True, info_tick=True))

    def test_validates_three_modes_are_paired(self):
        result = BENCH.validate_paired_worlds({
            "no-memory": observation(),
            "memory": observation(digest="b" * 64),
            "causal": observation(digest="c" * 64),
        }, expected_seed=42, expected_tick=0)
        self.assertTrue(result["same_seed"])
        self.assertTrue(result["same_starting_tick"])
        self.assertEqual(result["seed"], 42)
        self.assertEqual(result["starting_tick"], 0)
        self.assertEqual(set(result["modes"]), {"no-memory", "memory", "causal"})

    def test_rejects_seed_mismatch_before_provider_calls(self):
        with self.assertRaisesRegex(AssertionError, "seed mismatch"):
            BENCH.validate_paired_worlds({
                "no-memory": observation(seed=42),
                "memory": observation(seed=7),
                "causal": observation(seed=42),
            })

    def test_rejects_string_and_numeric_seed_coercion(self):
        with self.assertRaisesRegex(AssertionError, "seed mismatch"):
            BENCH.validate_paired_worlds({
                "no-memory": observation(seed=42),
                "memory": observation(seed="42"),
                "causal": observation(seed=42),
            })

    def test_rejects_different_starting_ticks(self):
        with self.assertRaisesRegex(AssertionError, "tick mismatch"):
            BENCH.validate_paired_worlds({
                "no-memory": observation(tick=0),
                "memory": observation(tick=1),
                "causal": observation(tick=0),
            })

    def test_checks_optional_expected_seed_and_tick(self):
        all_modes = {
            "no-memory": observation(seed=42, tick=0),
            "memory": observation(seed=42, tick=0),
            "causal": observation(seed=42, tick=0),
        }
        with self.assertRaisesRegex(AssertionError, "expected seed"):
            BENCH.validate_paired_worlds(all_modes, expected_seed=99)
        with self.assertRaisesRegex(AssertionError, "expected starting tick"):
            BENCH.validate_paired_worlds(all_modes, expected_tick=10)

    def test_summary_uses_nearest_rank_p95_and_strict_true(self):
        summary = BENCH.summarize([
            {"latency_ms": 10, "success": True, "source_unchanged": True,
             "failed_tool_steps": 0, "tool_steps": 2},
            {"latency_ms": 20, "success": 1, "source_unchanged": True,
             "failed_tool_steps": 1, "tool_steps": 4},
            {"latency_ms": 30, "success": True, "source_unchanged": False,
             "failed_tool_steps": 0, "tool_steps": 3},
        ])
        self.assertEqual(summary["tasks"], 3)
        self.assertEqual(summary["success_rate"], round(2 / 3, 4))
        self.assertEqual(summary["source_unchanged_rate"], round(2 / 3, 4))
        self.assertEqual(summary["p95_latency_ms"], 30)
        self.assertEqual(summary["tool_error_steps"], 1)

    def test_empty_summary_is_well_defined(self):
        summary = BENCH.summarize([])
        self.assertEqual(summary["tasks"], 0)
        self.assertEqual(summary["p95_latency_ms"], 0)
        self.assertEqual(summary["success_rate"], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
