#!/usr/bin/env python3
"""Unit tests for the versioned Wanxiang offline experiment report contract."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("validate-experiment-report.py")
SPEC = importlib.util.spec_from_file_location("validate_experiment_report", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


def snapshot(seed: int, tick: int, energy_density: float) -> dict:
    return {
        "meta": {"seed": seed, "tick": tick},
        "entities": [],
        "factions": [],
        "locations": [],
        "events": [],
        "resources": {},
        "rules": {"energy_density": energy_density, "gravity": 1.0},
    }


def valid_report() -> dict:
    baseline = snapshot(42, 100, 0.5)
    altered = snapshot(42, 100, 0.75)
    return {
        "format": "wanxiang-experiment-report-v1",
        "schema_version": 1,
        "title": "万象可复现规则对照实验报告",
        "generated_at": "2026-10-19T00:00:00Z",
        "generated_by": "Wanxiang browser experiment lab",
        "source": "test fixture",
        "build": {"source": "github-actions", "revision": "abc123", "repository": "owner/repo", "workflow": "Rust CI"},
        "scenario": {"seed": 42, "ticks": 100, "changed_rule": "energy_density"},
        "experiment": {
            "seed": 42,
            "ticks": 100,
            "changed_rule": "energy_density",
            "changed_rules": ["energy_density"],
            "baseline_value": 0.5,
            "altered_value": 0.75,
            "same_seed": True,
            "same_tick": True,
        },
        "baseline": baseline,
        "altered": altered,
        "diff": {"rules_differ": True},
        "interpretation": {
            "scope": "只描述模型内差异，不代表现实世界预测或因果证明。",
            "replay": "快照报告不是完整操作历史回放包。",
        },
    }


class ExperimentReportValidationTests(unittest.TestCase):
    def test_valid_report(self) -> None:
        VALIDATOR.validate(valid_report())

    def test_rejects_mismatched_tick(self) -> None:
        report = valid_report()
        report["altered"]["meta"]["tick"] = 99
        with self.assertRaisesRegex(ValueError, "snapshot ticks"):
            VALIDATOR.validate(report)

    def test_rejects_missing_changed_rule(self) -> None:
        report = valid_report()
        report["experiment"]["changed_rules"] = []
        with self.assertRaisesRegex(ValueError, "changed_rules"):
            VALIDATOR.validate(report)

    def test_rejects_wrong_format(self) -> None:
        report = valid_report()
        report["format"] = "wanxiang-replay-v1"
        with self.assertRaisesRegex(ValueError, "format must be"):
            VALIDATOR.validate(report)

    def test_rejects_unqualified_timestamp(self) -> None:
        report = valid_report()
        report["generated_at"] = "2026-10-19T00:00:00"
        with self.assertRaisesRegex(ValueError, "timezone"):
            VALIDATOR.validate(report)

    def test_rejects_array_diff(self) -> None:
        report = valid_report()
        report["diff"] = []
        with self.assertRaisesRegex(ValueError, "diff must be"):
            VALIDATOR.validate(report)

    def test_rejects_mismatched_rule_value(self) -> None:
        report = valid_report()
        report["experiment"]["altered_value"] = 0.9
        with self.assertRaisesRegex(ValueError, "altered_value"):
            VALIDATOR.validate(report)

    def test_rejects_scenario_seed_mismatch(self) -> None:
        report = valid_report()
        report["scenario"]["seed"] = 7
        with self.assertRaisesRegex(ValueError, "scenario.seed"):
            VALIDATOR.validate(report)

    def test_rejects_report_without_rule_changes(self) -> None:
        report = valid_report()
        report["altered"]["rules"] = dict(report["baseline"]["rules"])
        report["experiment"]["changed_rules"] = []
        report["experiment"]["changed_rule"] = None
        report["experiment"]["baseline_value"] = None
        report["experiment"]["altered_value"] = None
        with self.assertRaisesRegex(ValueError, "at least one changed rule"):
            VALIDATOR.validate(report)

    def test_rejects_boolean_snapshot_seed(self) -> None:
        report = valid_report()
        report["baseline"]["meta"]["seed"] = True
        with self.assertRaisesRegex(ValueError, "baseline.meta.seed"):
            VALIDATOR.validate(report)

    def test_rejects_string_snapshot_tick(self) -> None:
        report = valid_report()
        report["baseline"]["meta"]["tick"] = "100"
        with self.assertRaisesRegex(ValueError, "baseline.meta.tick"):
            VALIDATOR.validate(report)

    def test_rejects_non_boolean_consistency_flags(self) -> None:
        report = valid_report()
        report["experiment"]["same_seed"] = 1
        with self.assertRaisesRegex(ValueError, "same_seed must be a boolean"):
            VALIDATOR.validate(report)

    def test_published_json_schema_matches_required_envelope(self) -> None:
        schema_path = Path(__file__).parent.parent / "schemas" / "wanxiang-experiment-report-v1.schema.json"
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        self.assertEqual(schema["properties"]["format"]["const"], "wanxiang-experiment-report-v1")
        self.assertIn("build", schema["required"])
        self.assertEqual(schema["properties"]["experiment"]["properties"]["changed_rules"]["minItems"], 1)

    def test_rejects_missing_build_metadata(self) -> None:
        report = valid_report()
        del report["build"]
        with self.assertRaisesRegex(ValueError, "build is required"):
            VALIDATOR.validate(report)

    def test_rejects_invalid_build_revision_type(self) -> None:
        report = valid_report()
        report["build"]["revision"] = 123
        with self.assertRaisesRegex(ValueError, "build.revision"):
            VALIDATOR.validate(report)

    def test_rejects_empty_seed(self) -> None:
        report = valid_report()
        report["experiment"]["seed"] = "  "
        with self.assertRaisesRegex(ValueError, "non-empty number or string"):
            VALIDATOR.validate(report)

    def test_accepts_multiple_rule_differences_when_primary_scenario_rule_is_present(self) -> None:
        report = valid_report()
        report["altered"]["rules"]["gravity"] = 1.2
        report["scenario"]["baseline_energy_density"] = 0.5
        report["scenario"]["altered_energy_density"] = 0.75
        report["experiment"]["changed_rule"] = None
        report["experiment"]["changed_rules"] = ["energy_density", "gravity"]
        report["experiment"]["baseline_value"] = None
        report["experiment"]["altered_value"] = None
        VALIDATOR.validate(report)


if __name__ == "__main__":
    unittest.main(verbosity=2)
