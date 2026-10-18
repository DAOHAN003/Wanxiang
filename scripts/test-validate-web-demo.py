import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("validate-web-demo.py")
SPEC = importlib.util.spec_from_file_location("validate_web_demo", MODULE_PATH)
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


def snapshot(seed, tick, energy):
    return {
        "meta": {"seed": seed, "tick": tick},
        "entities": [],
        "factions": [],
        "locations": [],
        "events": [],
        "resources": {},
        "rules": {"energy_density": energy},
    }


def payload():
    return {
        "schema_version": 1,
        "generated_by": "wanxiang_sdk::WorldModel",
        "scenario": {
            "seed": 42,
            "ticks": 100,
            "changed_rule": "energy_density",
            "baseline_energy_density": 0.7,
            "altered_energy_density": 0.95,
        },
        "baseline": snapshot(42, 100, 0.7),
        "altered": snapshot(42, 100, 0.95),
        "diff": {},
    }


class ComparisonPayloadTests(unittest.TestCase):
    def test_accepts_valid_payload(self):
        VALIDATOR.validate(payload())

    def test_rejects_unknown_schema(self):
        data = payload()
        data["schema_version"] = 2
        with self.assertRaisesRegex(ValueError, "schema_version"):
            VALIDATOR.validate(data)

    def test_rejects_mismatched_seed(self):
        data = payload()
        data["altered"]["meta"]["seed"] = 7
        with self.assertRaisesRegex(ValueError, "seed"):
            VALIDATOR.validate(data)

    def test_rejects_mismatched_tick(self):
        data = payload()
        data["baseline"]["meta"]["tick"] = 99
        with self.assertRaisesRegex(ValueError, "tick"):
            VALIDATOR.validate(data)

    def test_rejects_unchanged_rule(self):
        data = payload()
        data["altered"]["rules"]["energy_density"] = 0.7
        data["scenario"]["altered_energy_density"] = 0.7
        with self.assertRaisesRegex(ValueError, "different"):
            VALIDATOR.validate(data)

    def test_rejects_missing_snapshot_field(self):
        data = payload()
        del data["baseline"]["events"]
        with self.assertRaisesRegex(ValueError, "events"):
            VALIDATOR.validate(data)


if __name__ == "__main__":
    unittest.main()
