#!/usr/bin/env python3
"""Validate a Wanxiang offline experiment report exported by the browser lab."""
from __future__ import annotations

import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any


FORMAT = "wanxiang-experiment-report-v1"
REQUIRED_SNAPSHOT_FIELDS = ("meta", "entities", "factions", "locations", "events", "resources", "rules")


def is_record(value: Any) -> bool:
    return isinstance(value, dict)


def validate(report: Any) -> None:
    if not is_record(report):
        raise ValueError("report must be a JSON object")
    if report.get("format") != FORMAT:
        raise ValueError(f"format must be {FORMAT}")
    if report.get("schema_version") != 1:
        raise ValueError("schema_version must be 1")
    for field in ("title", "generated_at", "generated_by", "source"):
        if not isinstance(report.get(field), str) or not report[field].strip():
            raise ValueError(f"{field} must be a non-empty string")
    try:
        timestamp = datetime.fromisoformat(report["generated_at"].replace("Z", "+00:00"))
    except ValueError as exc:
        raise ValueError("generated_at must be an ISO-8601 timestamp") from exc
    if timestamp.tzinfo is None:
        raise ValueError("generated_at must include a timezone")

    scenario = report.get("scenario")
    experiment = report.get("experiment")
    if not is_record(scenario):
        raise ValueError("scenario must be an object")
    if not is_record(experiment):
        raise ValueError("experiment must be an object")
    seed = experiment.get("seed")
    ticks = experiment.get("ticks")
    if isinstance(seed, bool) or not isinstance(seed, (int, float, str)) or (isinstance(seed, str) and not seed.strip()):
        raise ValueError("experiment.seed must be a non-empty number or string")
    if isinstance(ticks, bool) or not isinstance(ticks, int) or ticks < 0:
        raise ValueError("experiment.ticks must be a non-negative integer")
    for field in ("same_seed", "same_tick"):
        if not isinstance(experiment.get(field), bool):
            raise ValueError(f"experiment.{field} must be a boolean")

    snapshots: dict[str, dict[str, Any]] = {}
    for name in ("baseline", "altered"):
        snapshot = report.get(name)
        if not is_record(snapshot):
            raise ValueError(f"{name} must be an object")
        for field in REQUIRED_SNAPSHOT_FIELDS:
            if field not in snapshot:
                raise ValueError(f"{name}.{field} is required")
        if not is_record(snapshot["meta"]):
            raise ValueError(f"{name}.meta must be an object")
        for field in ("entities", "factions", "locations", "events"):
            if not isinstance(snapshot[field], list):
                raise ValueError(f"{name}.{field} must be an array")
        for field in ("resources", "rules"):
            if not is_record(snapshot[field]):
                raise ValueError(f"{name}.{field} must be an object")
        snapshots[name] = snapshot

    baseline, altered = snapshots["baseline"], snapshots["altered"]
    baseline_seed, altered_seed = baseline["meta"].get("seed"), altered["meta"].get("seed")
    baseline_tick, altered_tick = baseline["meta"].get("tick"), altered["meta"].get("tick")
    if baseline_seed != seed or altered_seed != seed:
        raise ValueError("both snapshot seeds must match experiment.seed")
    if baseline_tick != ticks or altered_tick != ticks:
        raise ValueError("both snapshot ticks must match experiment.ticks")
    expected_same_seed = str(baseline_seed) == str(altered_seed)
    expected_same_tick = float(baseline_tick) == float(altered_tick)
    if experiment.get("same_seed") != expected_same_seed:
        raise ValueError("experiment.same_seed does not match snapshot metadata")
    if experiment.get("same_tick") != expected_same_tick:
        raise ValueError("experiment.same_tick does not match snapshot metadata")

    changed_rules = experiment.get("changed_rules")
    if not isinstance(changed_rules, list) or any(not isinstance(key, str) or not key for key in changed_rules):
        raise ValueError("experiment.changed_rules must be an array of non-empty rule names")
    expected_changed = [
        key for key in dict.fromkeys([*baseline["rules"].keys(), *altered["rules"].keys()])
        if (key not in baseline["rules"] or key not in altered["rules"]
            or baseline["rules"][key] != altered["rules"][key])
    ]
    if changed_rules != expected_changed:
        raise ValueError("experiment.changed_rules does not match the paired snapshots")
    if not expected_changed:
        raise ValueError("report must contain at least one changed rule")
    changed_rule = experiment.get("changed_rule")
    if len(expected_changed) == 1 and changed_rule != expected_changed[0]:
        raise ValueError("experiment.changed_rule must identify the single changed rule")
    if len(expected_changed) != 1 and changed_rule is not None:
        raise ValueError("experiment.changed_rule must be null when multiple rules changed")
    if len(expected_changed) == 1:
        rule = expected_changed[0]
        if experiment.get("baseline_value") != baseline["rules"].get(rule):
            raise ValueError("experiment.baseline_value does not match the baseline rule")
        if experiment.get("altered_value") != altered["rules"].get(rule):
            raise ValueError("experiment.altered_value does not match the altered rule")
    elif experiment.get("baseline_value") is not None or experiment.get("altered_value") is not None:
        raise ValueError("baseline_value and altered_value must be null for multi-rule changes")

    if "seed" in scenario and scenario["seed"] != seed:
        raise ValueError("scenario.seed does not match experiment.seed")
    if "ticks" in scenario and scenario["ticks"] != ticks:
        raise ValueError("scenario.ticks does not match experiment.ticks")
    scenario_changed_rule = scenario.get("changed_rule")
    if scenario_changed_rule is not None and scenario_changed_rule not in expected_changed:
        raise ValueError("scenario.changed_rule is not present in the actual rule differences")
    if len(expected_changed) == 1 and scenario_changed_rule is not None and scenario_changed_rule != changed_rule:
        raise ValueError("scenario.changed_rule does not match experiment.changed_rule")
    if changed_rule == "energy_density":
        if "baseline_energy_density" in scenario and scenario["baseline_energy_density"] != baseline["rules"].get("energy_density"):
            raise ValueError("scenario.baseline_energy_density does not match baseline rule")
        if "altered_energy_density" in scenario and scenario["altered_energy_density"] != altered["rules"].get("energy_density"):
            raise ValueError("scenario.altered_energy_density does not match altered rule")

    if report.get("diff") is not None and not is_record(report["diff"]):
        raise ValueError("diff must be an object or null")
    interpretation = report.get("interpretation")
    if not is_record(interpretation):
        raise ValueError("interpretation must be an object")
    if not isinstance(interpretation.get("scope"), str) or not interpretation["scope"].strip():
        raise ValueError("interpretation.scope must describe the report's scope")
    if not isinstance(interpretation.get("replay"), str) or not interpretation["replay"].strip():
        raise ValueError("interpretation.replay must describe replay limitations")


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("artifacts/product-experiment-report.json")
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
        validate(report)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        print(f"Invalid Wanxiang experiment report: {exc}", file=sys.stderr)
        return 1
    print(f"Validated Wanxiang offline experiment report: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
