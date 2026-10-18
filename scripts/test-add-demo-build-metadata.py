#!/usr/bin/env python3
"""Unit tests for static demo build metadata annotation."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("add-demo-build-metadata.py")
SPEC = importlib.util.spec_from_file_location("add_demo_build_metadata", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def demo_payload() -> dict:
    return {
        "generated_by": "wanxiang_sdk::WorldModel",
        "scenario": {"seed": 42, "ticks": 100},
        "baseline": {"meta": {"seed": 42, "tick": 100}},
        "altered": {"meta": {"seed": 42, "tick": 100}},
    }


class BuildMetadataTests(unittest.TestCase):
    def test_attaches_github_revision_without_mutating_simulation_snapshots(self) -> None:
        data = demo_payload()
        annotated = MODULE.annotate(
            data, repository="owner/Wanxiang", revision="abc123", workflow="Rust CI"
        )
        self.assertEqual(annotated["build"]["source"], "github-actions")
        self.assertEqual(annotated["build"]["revision"], "abc123")
        self.assertEqual(annotated["baseline"], {"meta": {"seed": 42, "tick": 100}})
        self.assertEqual(annotated["altered"], {"meta": {"seed": 42, "tick": 100}})

    def test_local_build_is_explicit_when_revision_is_missing(self) -> None:
        annotated = MODULE.annotate(demo_payload(), repository=None, revision=None, workflow=None)
        self.assertEqual(annotated["build"]["source"], "local")
        self.assertIsNone(annotated["build"]["revision"])

    def test_rejects_payload_not_generated_by_sdk(self) -> None:
        data = demo_payload()
        data["generated_by"] = "some-other-source"
        with self.assertRaisesRegex(ValueError, "refusing to annotate"):
            MODULE.annotate(data, repository=None, revision="abc123", workflow="Rust CI")

    def test_rejects_payload_without_paired_snapshots(self) -> None:
        data = demo_payload()
        del data["altered"]
        with self.assertRaisesRegex(ValueError, "paired baseline and altered"):
            MODULE.annotate(data, repository=None, revision="abc123", workflow="Rust CI")


if __name__ == "__main__":
    unittest.main(verbosity=2)
