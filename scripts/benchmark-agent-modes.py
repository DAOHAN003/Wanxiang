#!/usr/bin/env python3
"""Compare live Agent retrieval modes on a paired, fixed task suite.

Run three isolated live_server instances with the same model/provider, seed,
starting tick and WANXIANG_TICK_MS=0, using
WANXIANG_AGENT_RETRIEVAL=no-memory|memory|causal. Calls a model provider and may
incur costs. All three server worlds are preflighted before the first Agent call.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import math
import json
from pathlib import Path
import statistics
import sys
import time
import urllib.error
import urllib.request
from typing import Any


TASKS = [
    {
        "name": "inspect_world_info",
        "category": "tool_grounding",
        "expected_tool": "world_info",
        "reference": "info",
        "answer_sources": {
            "seed": ["info", "seed"],
            "tick": ["info", "tick"],
            "characters": ["info", "characters"],
            "factions": ["info", "factions"],
            "events": ["info", "events"],
            "conserved": ["info", "conserved"],
        },
        "task": (
            "Call world_info exactly once. Return only a JSON object with exactly these keys: "
            "seed, tick, characters, factions, events, conserved. Copy their values from the "
            "world_info observation, do not estimate, and do not call any other tool. No Markdown."
        ),
    },
    {
        "name": "recall_prior_world_info",
        "category": "memory_recall",
        "expected_tool": None,
        "reference": "memory",
        "fixed_answer_values": {"available": True},
        "answer_sources": {
            "seed": ["info", "seed"],
            "tick": ["info", "tick"],
            "characters": ["info", "characters"],
            "factions": ["info", "factions"],
            "events": ["info", "events"],
            "conserved": ["info", "conserved"],
        },
        "task": (
            "Retrieve the most recent saved world_info observation from your memory from the "
            "earlier benchmark task. Do not call any world tool or use the live world. Return "
            "only a JSON object with exactly these keys: available, seed, tick, characters, "
            "factions, events, conserved. Set available=true only if the saved observation "
            "is actually present; if unavailable, set available=false and set all six remaining "
            "values to null. Copy saved values exactly. No Markdown."
        ),
    },
    {
        "name": "inspect_snapshot",
        "category": "tool_grounding",
        "expected_tool": "world_snapshot",
        "reference": "snapshot",
        "answer_sources": {
            "entity_count": ["snapshot", "meta", "total_entities"],
            "faction_count": ["snapshot", "meta", "total_factions"],
            "event_count": ["snapshot", "meta", "total_events"],
            "tick": ["snapshot", "meta", "tick"],
        },
        "task": (
            "Call world_snapshot exactly once. Return only a JSON object with exactly these keys: "
            "entity_count, faction_count, event_count, tick. Read entity_count, faction_count, "
            "and event_count from the corresponding entities.count, factions.count, and events.count "
            "fields; read tick from meta.tick. Do not use sample-array lengths and do not call "
            "any other tool. No Markdown."
        ),
    },
    {
        "name": "counterfactual_experiment",
        "category": "counterfactual_tool_use",
        "expected_tool": "world_experiment",
        "reference": "experiment",
        "expected_action_input": {"rule": "energy_density", "value": 0.95, "ticks": 8},
        "fixed_answer_values": {
            "ticks_simulated_per_branch": 8,
            "source_world_unchanged": True,
            "changed_rule": "energy_density",
            "altered_value": 0.95,
        },
        "answer_sources": {
            "source_tick": ["snapshot", "meta", "tick"],
        },
        "task": (
            "Call world_experiment exactly once with rule=energy_density, value=0.95, ticks=8. "
            "Return only a JSON object with exactly these keys: source_tick, ticks_simulated_per_branch, "
            "source_world_unchanged, changed_rule, altered_value. Copy values from the tool result. "
            "Never call world_step or any other tool. No Markdown."
        ),
    },
]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def request(base: str, path: str, method: str = "GET", body: dict[str, Any] | None = None,
            token: str | None = None, key: str | None = None) -> tuple[int, dict[str, Any], float]:
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if key:
        headers["Idempotency-Key"] = key
    payload = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    req = urllib.request.Request(base.rstrip("/") + path, data=payload, headers=headers, method=method)
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=180) as response:
            status, raw = response.status, response.read()
    except urllib.error.HTTPError as exc:
        status, raw = exc.code, exc.read()
    elapsed_ms = (time.perf_counter() - started) * 1000
    try:
        value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AssertionError(f"{base}{path} returned invalid JSON (HTTP {status}): {exc}") from exc
    if not isinstance(value, dict):
        raise AssertionError(f"{base}{path} returned non-object JSON")
    return status, value, elapsed_ms


def summarize(records: list[dict[str, Any]]) -> dict[str, Any]:
    latencies = [float(r["latency_ms"]) for r in records]
    ordered = sorted(latencies)
    p95 = ordered[max(0, min(len(ordered) - 1, math.ceil(0.95 * len(ordered)) - 1))] if ordered else 0
    count = len(records)

    def rate(field: str) -> float:
        return round(sum(r.get(field) is True for r in records) / count, 4) if count else 0.0

    tool_calls = [int(r.get("tool_steps", 0)) for r in records]
    return {
        "tasks": count,
        "success_rate": rate("success"),
        "task_correct_rate": rate("task_correct"),
        "answer_valid_rate": rate("answer_valid"),
        "answer_correct_rate": rate("answer_correct"),
        "tool_policy_pass_rate": rate("tool_policy_pass"),
        "evidence_consistency_rate": rate("evidence_consistent"),
        "source_unchanged_rate": rate("source_unchanged"),
        "memory_recall_answer_correct_rate": (
            round(
                sum(r.get("answer_correct") is True for r in records if r.get("task_category") == "memory_recall")
                / sum(1 for r in records if r.get("task_category") == "memory_recall"),
                4,
            )
            if any(r.get("task_category") == "memory_recall" for r in records) else None
        ),
        "tool_error_steps": sum(int(r.get("failed_tool_steps", 0)) for r in records),
        "average_tool_steps": round(statistics.mean(tool_calls), 3) if tool_calls else 0,
        "mean_latency_ms": round(statistics.mean(latencies), 3) if latencies else 0,
        "p95_latency_ms": round(p95, 3),
    }


FINAL_ACTIONS = {"", "null", "none", "finish", "final", "done", "stop", "end", "complete", "finish_task"}


def action_name(step: dict[str, Any]) -> str:
    return str(step.get("action") or "").strip().lower()


def is_tool_action(step: dict[str, Any]) -> bool:
    return action_name(step) not in FINAL_ACTIONS


def get_path(value: Any, path: list[str]) -> Any:
    current = value
    for part in path:
        if not isinstance(current, dict) or part not in current:
            raise KeyError(".".join(path))
        current = current[part]
    return current


def same_value(actual: Any, expected: Any) -> bool:
    """Strict JSON comparison, allowing harmless integer/float equivalence."""
    if isinstance(actual, bool) or isinstance(expected, bool):
        return type(actual) is bool and type(expected) is bool and actual is expected
    if isinstance(actual, (int, float)) and isinstance(expected, (int, float)):
        return math.isfinite(float(actual)) and math.isfinite(float(expected)) and math.isclose(
            float(actual), float(expected), rel_tol=1e-9, abs_tol=1e-9
        )
    return type(actual) is type(expected) and actual == expected


def parse_answer_object(answer: Any) -> dict[str, Any] | None:
    """Accept one pure JSON object, optionally inside one conventional JSON code fence."""
    if not isinstance(answer, str):
        return None
    candidate = answer.strip()
    fence = chr(96) * 3
    if candidate.startswith(fence + "json") and candidate.endswith(fence):
        candidate = candidate[len(fence + "json"): -len(fence)].strip()
    elif candidate.startswith(fence) and candidate.endswith(fence):
        candidate = candidate[len(fence): -len(fence)].strip()
    try:
        value = json.loads(candidate)
    except json.JSONDecodeError:
        return None
    return value if isinstance(value, dict) else None


def expected_answer(task: dict[str, Any], reference_state: dict[str, Any]) -> dict[str, Any]:
    expected = dict(task.get("fixed_answer_values", {}))
    for answer_key, source_path in task.get("answer_sources", {}).items():
        expected[answer_key] = get_path(reference_state, source_path)
    return expected


def observation_consistent(task: dict[str, Any], observation: Any,
                           reference_state: dict[str, Any]) -> bool:
    if not isinstance(observation, dict) or observation.get("error") is not None:
        return False
    reference = task["reference"]
    if reference == "info":
        info = reference_state.get("info")
        return isinstance(info, dict) and all(
            key in observation and same_value(observation[key], info[key])
            for key in task["answer_sources"]
        )
    if reference == "snapshot":
        snapshot = reference_state.get("snapshot")
        if not isinstance(snapshot, dict) or not isinstance(snapshot.get("meta"), dict):
            return False
        meta = snapshot["meta"]
        expected = {
            ("entities", "count"): meta.get("total_entities"),
            ("factions", "count"): meta.get("total_factions"),
            ("events", "count"): meta.get("total_events"),
            ("meta", "tick"): meta.get("tick"),
        }
        for (section, field), value in expected.items():
            try:
                if value is None or not same_value(get_path(observation, [section, field]), value):
                    return False
            except KeyError:
                return False
        return True
    if reference == "experiment":
        snapshot = reference_state.get("snapshot")
        if not isinstance(snapshot, dict) or not isinstance(snapshot.get("meta"), dict):
            return False
        start_tick = snapshot["meta"].get("tick")
        rules = snapshot.get("rules")
        if isinstance(start_tick, bool) or not isinstance(start_tick, int) or not isinstance(rules, dict):
            return False
        expected_fields = {
            "source_tick": start_tick,
            "ticks_simulated_per_branch": 8,
            "source_world_unchanged": True,
            "changed_rule": "energy_density",
            "altered_value": 0.95,
            "previous_value": rules.get("energy_density"),
        }
        for field, value in expected_fields.items():
            if field not in observation or not same_value(observation[field], value):
                return False
        for branch in ("baseline", "altered"):
            try:
                branch_tick = get_path(observation, [branch, "meta", "tick"])
                branch_seed = get_path(observation, [branch, "meta", "seed"])
            except KeyError:
                return False
            if not same_value(branch_tick, start_tick + 8):
                return False
            if not same_value(branch_seed, snapshot["meta"].get("seed")):
                return False
        return True
    if reference == "memory":
        # The recall task forbids tools; grade its answer against authoritative pre-task state.
        return True
    return False


def grade_task(task: dict[str, Any], result: dict[str, Any],
               reference_state: dict[str, Any], after_snapshot: dict[str, Any]) -> dict[str, Any]:
    steps = result.get("steps") if isinstance(result.get("steps"), list) else []
    steps = [step for step in steps if isinstance(step, dict)]
    tools = [step for step in steps if is_tool_action(step)]
    expected_tool = task.get("expected_tool")
    expected_calls = [step for step in tools if action_name(step) == expected_tool] if expected_tool else []
    unexpected_calls = [action_name(step) for step in tools if action_name(step) != expected_tool] if expected_tool else [
        action_name(step) for step in tools
    ]
    action_input_ok = True
    expected_input = task.get("expected_action_input")
    if expected_input is not None:
        action_input = expected_calls[0].get("action_input") if expected_calls else None
        action_input_ok = isinstance(action_input, dict) and all(
            key in action_input and same_value(action_input[key], value)
            for key, value in expected_input.items()
        )
    tool_policy_pass = (
        len(expected_calls) == (1 if expected_tool else 0)
        and not unexpected_calls
        and all(step.get("success") is True for step in expected_calls)
        and action_input_ok
    )
    raw_observation: Any = None
    if len(expected_calls) == 1 and expected_calls[0].get("success") is True:
        try:
            raw_observation = json.loads(str(expected_calls[0].get("observation", "")))
        except json.JSONDecodeError:
            raw_observation = None
    evidence_consistent = (
        observation_consistent(task, raw_observation, reference_state)
        if expected_tool else task.get("reference") == "memory"
    )
    answer_obj = parse_answer_object(result.get("answer"))
    expected = expected_answer(task, reference_state)
    answer_valid = answer_obj is not None and set(answer_obj) == set(expected)
    answer_correct = answer_valid and all(
        same_value(answer_obj[key], value) for key, value in expected.items()
    )
    source_unchanged = reference_state.get("snapshot") == after_snapshot
    runtime_success = result.get("success") is True
    failed_tool_steps = sum(1 for step in tools if step.get("success") is not True)
    task_correct = (
        runtime_success and answer_valid and answer_correct and tool_policy_pass
        and evidence_consistent and source_unchanged
    )
    return {
        "success": runtime_success,
        "task_correct": task_correct,
        "answer_valid": answer_valid,
        "answer_correct": answer_correct,
        "tool_policy_pass": tool_policy_pass,
        "evidence_consistent": evidence_consistent,
        "source_unchanged": source_unchanged,
        "tool_steps": len(tools),
        "failed_tool_steps": failed_tool_steps,
        "expected_tool": expected_tool,
        "observed_tool_calls": [action_name(step) for step in tools],
        "unexpected_tool_calls": unexpected_calls,
        "action_input_valid": action_input_ok,
        "answer_expected": expected,
        "answer_observed": answer_obj,
    }


def paired_comparison(reference_records: list[dict[str, Any]],
                      candidate_records: list[dict[str, Any]]) -> dict[str, Any]:
    """Report matched task/repetition differences without claiming statistical significance."""
    def index(records: list[dict[str, Any]]) -> dict[tuple[str, int], dict[str, Any]]:
        return {(record["task"], int(record["repetition"])): record for record in records}

    reference = index(reference_records)
    candidate = index(candidate_records)
    keys = sorted(set(reference) & set(candidate))
    outcome: dict[str, Any] = {"paired_tasks": len(keys)}
    for metric in ("task_correct", "answer_correct", "tool_policy_pass", "source_unchanged"):
        wins = losses = ties = 0
        for key in keys:
            a = reference[key].get(metric) is True
            b = candidate[key].get(metric) is True
            if b and not a:
                wins += 1
            elif a and not b:
                losses += 1
            else:
                ties += 1
        outcome[metric] = {"candidate_wins": wins, "candidate_losses": losses, "ties": ties}
        outcome[f"{metric}_rate_delta"] = round(
            (sum(candidate[key].get(metric) is True for key in keys) -
             sum(reference[key].get(metric) is True for key in keys)) / len(keys), 4
        ) if keys else 0.0
    outcome["mean_latency_ms_delta"] = round(
        statistics.mean(float(candidate[key]["latency_ms"]) - float(reference[key]["latency_ms"]) for key in keys), 3
    ) if keys else 0.0
    outcome["interpretation"] = (
        "Descriptive paired differences only; this does not establish statistical significance. Repeat across fresh worlds and seeds before drawing conclusions."
    )
    return outcome

def snapshot_metadata(state: dict[str, Any]) -> dict[str, Any]:
    """Extract and validate the exact world identity used to pair benchmark modes."""
    snapshot = state.get("snapshot")
    if not isinstance(snapshot, dict):
        raise AssertionError("state payload is missing snapshot")
    meta = snapshot.get("meta")
    info = state.get("info")
    if not isinstance(meta, dict) or not isinstance(info, dict):
        raise AssertionError("state payload is missing snapshot.meta or info")
    seed = meta.get("seed")
    tick = meta.get("tick")
    info_tick = info.get("tick")
    if isinstance(seed, bool) or not isinstance(seed, (int, str)) or seed == "":
        raise AssertionError(f"invalid snapshot seed: {seed!r}")
    if isinstance(tick, bool) or not isinstance(tick, int) or tick < 0:
        raise AssertionError(f"invalid snapshot tick: {tick!r}")
    if isinstance(info_tick, bool) or not isinstance(info_tick, int) or info_tick != tick:
        raise AssertionError(f"state tick {info_tick!r} differs from snapshot tick {tick!r}")
    canonical = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "seed": seed,
        "tick": tick,
        "snapshot_sha256": hashlib.sha256(canonical).hexdigest(),
        "snapshot": snapshot,
    }


def validate_paired_worlds(observations: dict[str, dict[str, Any]],
                           expected_seed: int | None = None,
                           expected_tick: int | None = None) -> dict[str, Any]:
    """Fail before any model call unless all modes start from the same seed and tick."""
    if set(observations) != {"no-memory", "memory", "causal"}:
        raise AssertionError("preflight requires exactly no-memory, memory and causal modes")
    baseline = observations["no-memory"]
    seed, tick = baseline["seed"], baseline["tick"]
    for mode, item in observations.items():
        if type(item["seed"]) is not type(seed) or item["seed"] != seed:
            raise AssertionError(f"seed mismatch before provider calls: no-memory={seed!r}, {mode}={item['seed']!r}")
        if type(item["tick"]) is not type(tick) or item["tick"] != tick:
            raise AssertionError(f"tick mismatch before provider calls: no-memory={tick!r}, {mode}={item['tick']!r}")
        if item["snapshot_sha256"] != baseline["snapshot_sha256"]:
            raise AssertionError(
                f"initial world snapshot mismatch before provider calls: no-memory and {mode} are not identical"
            )
    if expected_seed is not None and (type(seed) is not int or seed != expected_seed):
        raise AssertionError(f"expected seed {expected_seed}, got {seed!r}")
    if expected_tick is not None and tick != expected_tick:
        raise AssertionError(f"expected starting tick {expected_tick}, got {tick!r}")
    return {
        "seed": seed,
        "starting_tick": tick,
        "same_seed": True,
        "same_starting_tick": True,
        "modes": {
            mode: {"seed": item["seed"], "tick": item["tick"], "snapshot_sha256": item["snapshot_sha256"]}
            for mode, item in observations.items()
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-memory-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=no-memory")
    parser.add_argument("--memory-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=memory")
    parser.add_argument("--causal-url", required=True, help="server configured with WANXIANG_AGENT_RETRIEVAL=causal")
    parser.add_argument("--token", default=None)
    parser.add_argument("--expected-seed", type=int, default=None, help="optional additional assertion against the configured world seed")
    parser.add_argument("--expected-tick", type=int, default=None, help="optional additional assertion against the common starting tick")
    parser.add_argument("--provider-label", default="not-specified", help="operator-declared provider label; not verified by the server")
    parser.add_argument("--model-label", default="not-specified", help="operator-declared model label; do not put credentials here")
    parser.add_argument("--repetitions", type=int, default=1, help="number of times to run each task in every mode (1-50)")
    parser.add_argument("--confirm-provider-costs", action="store_true", help="explicitly confirm that Agent tasks call the configured provider and may incur costs")
    parser.add_argument("--preflight-only", action="store_true", help="validate readiness and paired initial worlds without making any Agent/provider calls")
    parser.add_argument("--output", type=Path, default=None, help="also write the complete JSON report to this path")
    args = parser.parse_args()
    if not args.preflight_only and not args.confirm_provider_costs:
        parser.error("provider-backed Agent requests may incur costs; pass --confirm-provider-costs after reviewing your budget, or use --preflight-only for a no-inference configuration check")
    if not 1 <= args.repetitions <= 50:
        parser.error("--repetitions must be between 1 and 50")
    modes = {
        "no-memory": args.no_memory_url,
        "memory": args.memory_url,
        "causal": args.causal_url,
    }
    started_at = utc_now()
    run_id = f"agent-eval-{time.time_ns()}"
    report: dict[str, Any] = {
        "benchmark": "wanxiang_agent_retrieval_modes",
        "schema_version": 1,
        "run_id": run_id,
        "started_at": started_at,
        "finished_at": None,
        "provider_calls_expected": not args.preflight_only,
        "may_incur_provider_cost": not args.preflight_only,
        "note": ("Preflight-only report: no Agent endpoint was called and no model inference was requested. " if args.preflight_only else "Results are provider/model dependent. This measures deterministic answer correctness, tool-policy compliance, evidence consistency, latency and world-state safety; it does not establish semantic truth or general intelligence."),
        "grading_protocol": "wanxiang-agent-exact-json-v1",
        "pairing": None,
        "configuration": {
            "task_names": [task["name"] for task in TASKS],
            "task_categories": {task["name"]: task["category"] for task in TASKS},
            "mode_names": list(modes),
            "expected_seed": args.expected_seed,
            "expected_tick": args.expected_tick,
            "provider_label": args.provider_label,
            "model_label": args.model_label,
            "provider_and_model_labels_are_operator_declared": True,
            "repetitions_per_task": args.repetitions,
            "expected_agent_task_requests": 0 if args.preflight_only else len(TASKS) * len(modes) * args.repetitions,
            "auto_tick_ms_required": 0,
        },
        "modes": {},
    }

    # Phase 1: preflight every mode and validate pairing before the first billable Agent call.
    health_by_mode: dict[str, dict[str, Any]] = {}
    observations: dict[str, dict[str, Any]] = {}
    for mode, base in modes.items():
        status, health, _ = request(base, "/health", token=args.token)
        if status != 200 or health.get("status") != "ok":
            raise AssertionError(f"{mode} health failed: HTTP {status} {health}")
        if not health.get("llm_configured"):
            raise AssertionError(f"{mode} server has no LLM configured")
        if isinstance(health.get("auto_tick_ms"), bool) or health.get("auto_tick_ms") != 0:
            raise AssertionError(f"{mode} needs WANXIANG_TICK_MS=0 to make source-world comparisons reproducible")
        persistence = health.get("persistence")
        if not isinstance(persistence, dict) or persistence.get("degraded") is not False:
            raise AssertionError(f"{mode} persistence is degraded or unreported: {persistence}")
        reported_mode = health.get("agent_retrieval_mode")
        if reported_mode and not (
            (mode == "no-memory" and reported_mode in ("no-memory", "none"))
            or reported_mode == mode
        ):
            raise AssertionError(f"{mode} URL reports retrieval mode {reported_mode}")
        state_status, state, _ = request(base, "/api/state", token=args.token)
        if state_status != 200:
            raise AssertionError(f"{mode} preflight state failed: HTTP {state_status} {state}")
        health_by_mode[mode] = health
        observations[mode] = snapshot_metadata(state)

    report["pairing"] = validate_paired_worlds(observations, args.expected_seed, args.expected_tick)
    report["configuration"]["retrieval_modes"] = {
        mode: health.get("agent_retrieval_mode", mode) for mode, health in health_by_mode.items()
    }

    if args.preflight_only:
        report["preflight"] = {
            "passed": True,
            "agent_tasks_run": 0,
            "provider_calls_made": 0,
            "server_checks": {
                mode: {
                    "health_status": health.get("status"),
                    "llm_configured": health.get("llm_configured"),
                    "auto_tick_ms": health.get("auto_tick_ms"),
                    "persistence_degraded": health.get("persistence", {}).get("degraded"),
                }
                for mode, health in health_by_mode.items()
            },
        }
        report["evaluation"] = {
            "preflight_passed": True,
            "agent_tasks_run": 0,
            "all_source_worlds_unchanged": True,
        }
        report["finished_at"] = utc_now()
        serialized = json.dumps(report, ensure_ascii=False, indent=2) + "\\n"
        if args.output is not None:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(serialized, encoding="utf-8")
        print(serialized, end="")
        if args.output is not None:
            print(f"Report written to {args.output}", file=sys.stderr)
        return 0

    all_source_worlds_unchanged = True
    for mode, base in modes.items():
        records: list[dict[str, Any]] = []
        expected_snapshot = observations[mode]["snapshot"]
        for repetition in range(1, args.repetitions + 1):
            for task in TASKS:
                st_before, before, _ = request(base, "/api/state", token=args.token)
                if st_before != 200:
                    raise AssertionError(f"{mode}/{task['name']} pre-state failed: {before}")
                before_snapshot = before.get("snapshot")
                if before_snapshot != expected_snapshot:
                    raise AssertionError(f"{mode}/{task['name']} world changed between preflight and task start")
                key = f"{run_id}-{mode}-{repetition}-{task['name']}"
                status, result, latency = request(
                    base, "/api/agent", "POST", {"task": task["task"]}, args.token, key
                )
                if status != 200:
                    raise AssertionError(f"{mode}/{task['name']} agent call failed: HTTP {status} {result}")
                st_after, after, _ = request(base, "/api/state", token=args.token)
                if st_after != 200:
                    raise AssertionError(f"{mode}/{task['name']} post-state failed: {after}")
                after_snapshot = after.get("snapshot")
                grade = grade_task(task, result, before, after_snapshot)
                record = {
                    "task": task["name"],
                    "task_category": task["category"],
                    "repetition": repetition,
                    "latency_ms": round(latency, 3),
                    **grade,
                    "error": result.get("error"),
                    "answer": result.get("answer"),
                }
                if record["source_unchanged"] is not True:
                    all_source_worlds_unchanged = False
                records.append(record)
        categories = sorted({record["task_category"] for record in records})
        report["modes"][mode] = {
            "summary": summarize(records),
            "summary_by_category": {
                category: summarize([record for record in records if record["task_category"] == category])
                for category in categories
            },
            "records": records,
        }

    report["paired_comparisons"] = {
        "memory_vs_no_memory": paired_comparison(
            report["modes"]["no-memory"]["records"], report["modes"]["memory"]["records"]
        ),
        "causal_vs_no_memory": paired_comparison(
            report["modes"]["no-memory"]["records"], report["modes"]["causal"]["records"]
        ),
        "causal_vs_memory": paired_comparison(
            report["modes"]["memory"]["records"], report["modes"]["causal"]["records"]
        ),
    }
    all_records = [record for mode_report in report["modes"].values() for record in mode_report["records"]]
    report["evaluation"] = {
        "completed": True,
        "all_source_worlds_unchanged": all_source_worlds_unchanged,
        "all_tasks_correct": all(record["task_correct"] is True for record in all_records),
        "correct_tasks": sum(record["task_correct"] is True for record in all_records),
        "total_tasks": len(all_records),
        "exit_status_note": "Task failures are evaluation outcomes; only source-world mutation makes the process exit non-zero.",
    }
    report["finished_at"] = utc_now()
    serialized = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(serialized, encoding="utf-8")
    print(serialized, end="")
    if args.output is not None:
        print(f"Report written to {args.output}", file=sys.stderr)
    return 0 if all_source_worlds_unchanged else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
