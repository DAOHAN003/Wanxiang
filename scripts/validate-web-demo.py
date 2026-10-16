#!/usr/bin/env python3
"""Validate the versioned comparison payload consumed by the Wanxiang web demo."""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


def is_record(value: Any) -> bool:
    return isinstance(value, dict)


def validate(data: Any) -> None:
    if not is_record(data):
        raise ValueError("comparison payload must be a JSON object")
    if data.get("schema_version") != 1:
        raise ValueError("schema_version must be 1")
    if data.get("generated_by") != "wanxiang_sdk::WorldModel":
        raise ValueError("generated_by must identify wanxiang_sdk::WorldModel")

    scenario = data.get("scenario")
    if not is_record(scenario):
        raise ValueError("scenario must be an object")
    for key in ("seed", "ticks", "changed_rule", "baseline_energy_density", "altered_energy_density"):
        if key not in scenario:
            raise ValueError(f"scenario.{key} is required")
    if scenario["changed_rule"] != "energy_density":
        raise ValueError("scenario.changed_rule must be energy_density")
    if scenario["baseline_energy_density"] == scenario["altered_energy_density"]:
        raise ValueError("the changed rule must have different baseline and altered values")

    snapshots = []
    for name in ("baseline", "altered"):
        snapshot = data.get(name)
        if not is_record(snapshot):
            raise ValueError(f"{name} must be an object")
        for key in ("meta", "entities", "factions", "events", "resources", "rules"):
            if key not in snapshot:
                raise ValueError(f"{name}.{key} is required")
        if not is_record(snapshot["meta"]):
            raise ValueError(f"{name}.meta must be an object")
        for key in ("entities", "factions", "events"):
            if not isinstance(snapshot[key], list):
                raise ValueError(f"{name}.{key} must be an array")
        if not is_record(snapshot["resources"]) or not is_record(snapshot["rules"]):
            raise ValueError(f"{name}.resources and {name}.rules must be objects")
        if snapshot["meta"].get("seed") != scenario["seed"]:
            raise ValueError(f"{name}.meta.seed does not match scenario.seed")
        if snapshot["meta"].get("tick") != scenario["ticks"]:
            raise ValueError(f"{name}.meta.tick does not match scenario.ticks")
        snapshots.append(snapshot)

    base_rules = snapshots[0]["rules"]
    altered_rules = snapshots[1]["rules"]
    if base_rules.get("energy_density") != scenario["baseline_energy_density"]:
        raise ValueError("baseline energy_density does not match scenario metadata")
    if altered_rules.get("energy_density") != scenario["altered_energy_density"]:
        raise ValueError("altered energy_density does not match scenario metadata")
    if base_rules.get("energy_density") == altered_rules.get("energy_density"):
        raise ValueError("baseline and altered energy_density must differ")


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("site/data/demo.json")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        validate(data)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        print(f"Invalid Wanxiang comparison payload: {exc}", file=sys.stderr)
        return 1
    print(f"Validated Wanxiang comparison payload: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
