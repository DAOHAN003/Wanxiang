#!/usr/bin/env python3
"""Offline unit tests for the provider-backed Agent benchmark preflight."""
from __future__ import annotations

import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


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
            "memory": observation(),
            "causal": observation(),
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

    def test_rejects_different_initial_snapshot_with_same_seed_and_tick(self):
        with self.assertRaisesRegex(AssertionError, "initial world snapshot mismatch"):
            BENCH.validate_paired_worlds({
                "no-memory": observation(digest="a" * 64),
                "memory": observation(digest="b" * 64),
                "causal": observation(digest="a" * 64),
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

    def test_detects_provider_authentication_failures(self):
        cases = [
            {"error": "planner failed: adapter deepseek-flash: 认证失败（API Key 无效）"},
            {"answer": "provider returned HTTP 401 Unauthorized"},
            {"error": "invalid_api_key"},
        ]
        for record in cases:
            with self.subTest(record=record):
                self.assertTrue(BENCH.is_provider_auth_failure(record))

    def test_ordinary_incorrect_answers_are_not_provider_auth_failures(self):
        self.assertFalse(BENCH.is_provider_auth_failure({
            "error": None,
            "answer": "{\\"tick\\": 99}",
        }))

    def fixture_state(self):
        return {
            "info": {
                "seed": 42, "tick": 0, "characters": 7, "factions": 2,
                "events": 5, "conserved": True, "conservation_drift": 0.0,
            },
            "snapshot": {
                "meta": {
                    "seed": 42, "tick": 0, "total_entities": 9,
                    "total_factions": 2, "total_events": 5,
                },
                "rules": {"energy_density": 0.5},
                "entities": [{"id": "entity-1"}],
            },
        }

    def answer_for(self, task, reference_state):
        return BENCH.expected_answer(task, reference_state)

    def test_world_info_task_requires_correct_json_and_grounded_tool(self):
        task = next(item for item in BENCH.TASKS if item["name"] == "inspect_world_info")
        reference = self.fixture_state()
        result = {
            "success": True,
            "answer": __import__("json").dumps(self.answer_for(task, reference)),
            "steps": [{
                "action": "world_info", "action_input": {},
                "observation": __import__("json").dumps(reference["info"]), "success": True,
            }],
        }
        grade = BENCH.grade_task(task, result, reference, reference["snapshot"])
        self.assertTrue(grade["task_correct"])
        self.assertTrue(grade["evidence_consistent"])
        self.assertTrue(grade["tool_policy_pass"])
        self.assertTrue(grade["answer_correct"])

    def test_memory_recall_task_fails_if_it_calls_a_live_tool(self):
        task = next(item for item in BENCH.TASKS if item["name"] == "recall_prior_world_info")
        reference = self.fixture_state()
        result = {
            "success": True,
            "answer": __import__("json").dumps(self.answer_for(task, reference)),
            "steps": [{
                "action": "world_info", "action_input": {},
                "observation": __import__("json").dumps(reference["info"]), "success": True,
            }],
        }
        grade = BENCH.grade_task(task, result, reference, reference["snapshot"])
        self.assertFalse(grade["tool_policy_pass"])
        self.assertFalse(grade["task_correct"])

    def test_memory_recall_is_checked_against_authoritative_state_without_tools(self):
        task = next(item for item in BENCH.TASKS if item["name"] == "recall_prior_world_info")
        reference = self.fixture_state()
        result = {
            "success": True,
            "answer": __import__("json").dumps(self.answer_for(task, reference)),
            "steps": [],
        }
        grade = BENCH.grade_task(task, result, reference, reference["snapshot"])
        self.assertTrue(grade["task_correct"])
        self.assertTrue(grade["tool_policy_pass"])

    def test_snapshot_task_checks_full_counts_not_sample_length(self):
        task = next(item for item in BENCH.TASKS if item["name"] == "inspect_snapshot")
        reference = self.fixture_state()
        observation = {
            "meta": {"seed": 42, "tick": 0},
            "entities": {"count": 9, "sample": [{"id": "entity-1"}], "truncated": True},
            "factions": {"count": 2, "sample": [], "truncated": False},
            "events": {"count": 5, "sample": [], "truncated": False},
        }
        result = {
            "success": True, "answer": __import__("json").dumps(self.answer_for(task, reference)),
            "steps": [{
                "action": "world_snapshot", "action_input": {},
                "observation": __import__("json").dumps(observation), "success": True,
            }],
        }
        grade = BENCH.grade_task(task, result, reference, reference["snapshot"])
        self.assertTrue(grade["task_correct"])
        self.assertEqual(grade["answer_expected"]["entity_count"], 9)

    def test_experiment_grader_checks_action_arguments_and_branch_ticks(self):
        task = next(item for item in BENCH.TASKS if item["name"] == "counterfactual_experiment")
        reference = self.fixture_state()
        observation = {
            "source_tick": 0, "ticks_simulated_per_branch": 8,
            "source_world_unchanged": True, "changed_rule": "energy_density",
            "altered_value": 0.95, "previous_value": 0.5,
            "baseline": {"meta": {"seed": 42, "tick": 8}},
            "altered": {"meta": {"seed": 42, "tick": 8}},
        }
        result = {
            "success": True, "answer": __import__("json").dumps(self.answer_for(task, reference)),
            "steps": [{
                "action": "world_experiment",
                "action_input": {"rule": "energy_density", "value": 0.95, "ticks": 8},
                "observation": __import__("json").dumps(observation), "success": True,
            }],
        }
        grade = BENCH.grade_task(task, result, reference, reference["snapshot"])
        self.assertTrue(grade["task_correct"])

    def test_wrong_answer_or_mutated_source_world_fails_task_correctness(self):
        task = next(item for item in BENCH.TASKS if item["name"] == "inspect_world_info")
        reference = self.fixture_state()
        answer = self.answer_for(task, reference)
        answer["tick"] = 123
        result = {
            "success": True, "answer": __import__("json").dumps(answer),
            "steps": [{
                "action": "world_info", "action_input": {},
                "observation": __import__("json").dumps(reference["info"]), "success": True,
            }],
        }
        after = dict(reference["snapshot"])
        after["meta"] = dict(reference["snapshot"]["meta"], tick=1)
        grade = BENCH.grade_task(task, result, reference, after)
        self.assertFalse(grade["answer_correct"])
        self.assertFalse(grade["source_unchanged"])
        self.assertFalse(grade["task_correct"])

    def test_comparison_reports_matched_wins_losses_and_latency_delta(self):
        baseline = [
            {"task": "a", "repetition": 1, "task_correct": False, "answer_correct": False,
             "tool_policy_pass": True, "source_unchanged": True, "latency_ms": 100},
            {"task": "b", "repetition": 1, "task_correct": True, "answer_correct": True,
             "tool_policy_pass": True, "source_unchanged": True, "latency_ms": 100},
        ]
        candidate = [
            {"task": "a", "repetition": 1, "task_correct": True, "answer_correct": True,
             "tool_policy_pass": True, "source_unchanged": True, "latency_ms": 150},
            {"task": "b", "repetition": 1, "task_correct": False, "answer_correct": False,
             "tool_policy_pass": False, "source_unchanged": True, "latency_ms": 50},
        ]
        comparison = BENCH.paired_comparison(baseline, candidate)
        self.assertEqual(comparison["paired_tasks"], 2)
        self.assertEqual(comparison["task_correct"]["candidate_wins"], 1)
        self.assertEqual(comparison["task_correct"]["candidate_losses"], 1)
        self.assertEqual(comparison["mean_latency_ms_delta"], 0)
        self.assertIn("does not establish statistical significance", comparison["interpretation"])


    def test_preflight_only_validates_three_servers_without_agent_calls(self):
        urls = {
            "no-memory": "http://127.0.0.1:8787",
            "memory": "http://127.0.0.1:8788",
            "causal": "http://127.0.0.1:8789",
        }
        mode_by_url = {url: mode for mode, url in urls.items()}
        calls = []
        initial_state = self.fixture_state()

        def fake_request(base, path, method="GET", body=None, token=None, key=None):
            calls.append((base, path, method))
            if path == "/health":
                mode = mode_by_url[base]
                return 200, {
                    "status": "ok",
                    "llm_configured": True,
                    "auto_tick_ms": 0,
                    "persistence": {"degraded": False},
                    "agent_retrieval_mode": mode,
                }, 0.1
            if path == "/api/state":
                return 200, initial_state, 0.1
            raise AssertionError(f"preflight unexpectedly requested {method} {path}")

        with tempfile.TemporaryDirectory() as temp_dir:
            report_path = Path(temp_dir) / "agent-preflight.json"
            argv = [
                str(SCRIPT),
                "--no-memory-url", urls["no-memory"],
                "--memory-url", urls["memory"],
                "--causal-url", urls["causal"],
                "--output", str(report_path),
                "--preflight-only",
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(BENCH, "request", side_effect=fake_request),
                patch("sys.stdout", new_callable=io.StringIO) as stdout,
                patch("sys.stderr", new_callable=io.StringIO),
            ):
                exit_code = BENCH.main()

            report = json.loads(stdout.getvalue())
            self.assertTrue(report_path.exists())
            self.assertEqual(json.loads(report_path.read_text(encoding="utf-8")), report)

        self.assertEqual(exit_code, 0)
        self.assertEqual(len(calls), 6)
        self.assertFalse(any(path == "/api/agent" for _, path, _ in calls))
        self.assertFalse(report["provider_calls_expected"])
        self.assertFalse(report["may_incur_provider_cost"])
        self.assertEqual(report["configuration"]["expected_agent_task_requests"], 0)
        self.assertTrue(report["preflight"]["passed"])
        self.assertEqual(report["preflight"]["agent_tasks_run"], 0)
        self.assertEqual(report["preflight"]["provider_calls_made"], 0)
        self.assertTrue(report["pairing"]["same_seed"])
        self.assertTrue(report["pairing"]["same_starting_tick"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
