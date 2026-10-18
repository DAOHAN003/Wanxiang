#!/usr/bin/env python3
"""Deterministic smoke/evaluation suite for the running Wanxiang HTTP API."""
from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from typing import Any


def request(base: str, path: str, method: str = "GET", body: dict[str, Any] | None = None,
            token: str | None = None, idempotency_key: str | None = None) -> tuple[int, dict[str, Any], float]:
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if idempotency_key:
        headers["Idempotency-Key"] = idempotency_key
    data = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    req = urllib.request.Request(base.rstrip("/") + path, data=data, headers=headers, method=method)
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            status = resp.status
            payload = resp.read()
    except urllib.error.HTTPError as exc:
        status = exc.code
        payload = exc.read()
    elapsed_ms = (time.perf_counter() - started) * 1000
    try:
        parsed = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AssertionError(f"{path} returned non-JSON payload (HTTP {status}): {exc}") from exc
    if not isinstance(parsed, dict):
        raise AssertionError(f"{path} returned a non-object JSON payload")
    return status, parsed, elapsed_ms


def check(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:8787")
    parser.add_argument("--token", default=None)
    parser.add_argument("--agent", action="store_true", help="also run a provider-backed world_info task")
    args = parser.parse_args()
    measurements: list[dict[str, Any]] = []

    status, health, ms = request(args.base_url, "/health", token=args.token)
    check(status == 200 and health.get("status") == "ok", f"health failed: HTTP {status} {health}")
    persistence = health.get("persistence")
    check(isinstance(persistence, dict) and persistence.get("degraded") is False,
          f"new server should report healthy persistence: {persistence}")
    measurements.append({"name": "health", "latency_ms": round(ms, 3),
                         "persistence_degraded": persistence.get("degraded")})

    status, malformed_stream, ms = request(
        args.base_url, "/api/llm/stream", "POST", {"prompt": ""}, args.token
    )
    check(status == 400, f"empty stream prompt should fail before provider lookup, got HTTP {status}")
    measurements.append({"name": "sse-input-validation", "latency_ms": round(ms, 3)})

    status, empty_session, ms = request(
        args.base_url, "/api/sessions", "POST", {"task": ""}, args.token
    )
    check(status == 400, f"empty session task should return HTTP 400, got {status}")
    measurements.append({"name": "session-input-validation", "latency_ms": round(ms, 3)})
    if not health.get("llm_configured", False):
        status, disabled_session, ms = request(
            args.base_url, "/api/sessions", "POST",
            {"task": "Session route availability check; do not execute tools."}, args.token
        )
        check(status == 503, f"disabled Agent session should return HTTP 503, got {status}")
        measurements.append({"name": "session-disabled-fails-closed", "latency_ms": round(ms, 3)})
    if not health.get("llm_configured", False):
        status, disabled_stream, ms = request(
            args.base_url, "/api/llm/stream", "POST",
            {"prompt": "SSE route availability check; do not execute world tools."}, args.token
        )
        check(status == 503, f"disabled LLM stream should report HTTP 503, got {status}")
        measurements.append({"name": "sse-disabled-fails-closed", "latency_ms": round(ms, 3)})

    status, initial, ms = request(args.base_url, "/api/state", token=args.token)
    check(status == 200 and initial.get("backend") == "wanxiang_sdk::WorldModel", "state did not come from Rust WorldModel")
    snapshot = initial.get("snapshot")
    check(isinstance(snapshot, dict) and isinstance(snapshot.get("meta"), dict), "state snapshot missing metadata")
    initial_tick = int(initial["info"]["tick"])
    measurements.append({"name": "state", "latency_ms": round(ms, 3), "tick": initial_tick})

    status, invalid, ms = request(args.base_url, "/api/step", "POST", {"ticks": 101}, args.token)
    check(status == 400, f"invalid step should return HTTP 400, got {status}")
    status, after_invalid, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_invalid["info"]["tick"]) == initial_tick, "invalid step mutated world tick")
    measurements.append({"name": "invalid-step-no-mutation", "latency_ms": round(ms, 3)})

    rules = snapshot.get("rules", {})
    current = float(rules.get("energy_density", 0.5))
    intervention = 0.35 if abs(current - 0.95) < 1e-9 else 0.95
    experiment = {"rule": "energy_density", "value": intervention, "ticks": 12}
    status, result_a, ms = request(args.base_url, "/api/experiment", "POST", experiment, args.token)
    check(status == 200, f"experiment failed: HTTP {status} {result_a}")
    status, result_b, ms_repeat = request(args.base_url, "/api/experiment", "POST", experiment, args.token)
    check(status == 200, f"repeat experiment failed: HTTP {status} {result_b}")
    check(result_a.get("baseline") == result_b.get("baseline"), "repeated baseline branch is not deterministic")
    check(result_a.get("altered") == result_b.get("altered"), "repeated altered branch is not deterministic")
    check(result_a.get("diff") == result_b.get("diff"), "repeated diff is not deterministic")
    check(result_a.get("source_world_unchanged") is True, "experiment reports source world changed")
    for label in ("baseline", "altered"):
        ledger = result_a[label].get("resources", {}).get("conservation", {})
        check(ledger.get("conserved") is True, f"{label} branch failed conservation check")
    measurements.append({"name": "experiment-repeat-determinism", "latency_ms": round(ms, 3),
                         "repeat_latency_ms": round(ms_repeat, 3), "ticks_per_branch": 12})

    status, after_experiment, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_experiment["info"]["tick"]) == initial_tick, "experiment mutated the live world's tick")

    # Test a real, exported route by selecting a living resident and one of its authored neighbors.
    live_locations = {
        str(location.get("id")): location
        for location in snapshot.get("locations", [])
        if isinstance(location, dict) and location.get("id") is not None
    }
    live_entities = snapshot.get("entities", [])
    move_choice = None
    for entity in live_entities:
        if not isinstance(entity, dict) or not entity.get("alive") or not entity.get("id"):
            continue
        origin_id = entity.get("location_id")
        origin = live_locations.get(str(origin_id))
        if origin is None:
            continue
        neighbors = [str(location_id) for location_id in origin.get("connected_ids", [])
                     if str(location_id) in live_locations and str(location_id) != str(origin_id)]
        if neighbors:
            move_choice = (entity, origin, sorted(neighbors)[0])
            break
    check(move_choice is not None,
          "live world needs at least one living resident on a location with an authored route")
    character, origin, destination_id = move_choice
    move_key = f"wanxiang-eval-move-{time.time_ns()}"
    move_request = {"character_id": str(character["id"]), "destination_id": destination_id}
    status_move, moved, move_ms = request(
        args.base_url, "/api/move", "POST", move_request, args.token, move_key
    )
    check(status_move == 200 and moved.get("moved") is True,
          f"route-aware movement failed: HTTP {status_move} {moved}")
    check(int(moved.get("tick", -1)) == initial_tick,
          "movement must not advance world time")
    moved_snapshot = moved.get("snapshot", {})
    moved_entity = next((e for e in moved_snapshot.get("entities", [])
                         if str(e.get("id")) == str(character["id"])), None)
    check(moved_entity is not None and str(moved_entity.get("location_id")) == destination_id,
          "movement response did not export the character's new location")
    check(any(
        isinstance(event, dict)
        and "移动到" in str(event.get("description", ""))
        and str(character["id"]) in event.get("actors", [])
        and str(event.get("location")) == destination_id
        for event in moved_snapshot.get("events", [])
    ), "movement event was not present in the returned snapshot")
    status_repeat, moved_repeat, repeat_ms = request(
        args.base_url, "/api/move", "POST", move_request, args.token, move_key
    )
    check(status_repeat == 200 and moved_repeat == moved,
          "duplicate movement idempotency key must replay the original response")
    measurements.append({
        "name": "route-aware-movement-and-idempotency",
        "latency_ms": round(move_ms, 3),
        "repeat_latency_ms": round(repeat_ms, 3),
        "character_id": str(character["id"]),
        "origin_id": str(origin["id"]),
        "destination_id": destination_id,
        "tick_unchanged": True,
    })

    # An unconnected destination must be rejected without changing location or appending an event.
    before_invalid_move_status, before_invalid_move, _ = request(
        args.base_url, "/api/state", token=args.token
    )
    check(before_invalid_move_status == 200, "could not read state before invalid movement")
    current_entity = next((e for e in before_invalid_move["snapshot"].get("entities", [])
                           if str(e.get("id")) == str(character["id"])), None)
    check(current_entity is not None, "moved character missing from state snapshot")
    current_location = live_locations.get(destination_id)
    current_neighbors = {str(x) for x in (current_location or {}).get("connected_ids", [])}
    invalid_destination = next((key for key in sorted(live_locations)
                                 if key != destination_id and key not in current_neighbors), None)
    check(invalid_destination is not None, "test world needs a non-adjacent location for rejection coverage")
    status_bad_move, bad_move, bad_move_ms = request(
        args.base_url, "/api/move", "POST",
        {"character_id": str(character["id"]), "destination_id": invalid_destination},
        args.token, f"wanxiang-eval-invalid-move-{time.time_ns()}"
    )
    check(status_bad_move == 400 and bad_move.get("moved") is False,
          f"non-route move should fail closed, got HTTP {status_bad_move}: {bad_move}")
    status_after_bad_move, after_bad_move, _ = request(
        args.base_url, "/api/state", token=args.token
    )
    check(status_after_bad_move == 200, "could not read state after invalid movement")
    after_char = next((e for e in after_bad_move["snapshot"].get("entities", [])
                       if str(e.get("id")) == str(character["id"])), None)
    check(after_char is not None and after_char.get("location_id") == destination_id,
          "rejected movement changed the character location")
    check(after_bad_move["snapshot"].get("events") == before_invalid_move["snapshot"].get("events"),
          "rejected movement must not append an event")
    measurements.append({
        "name": "invalid-route-no-mutation",
        "latency_ms": round(bad_move_ms, 3),
        "origin_id": destination_id,
        "rejected_destination_id": invalid_destination,
    })

    # Exercise shortest-path travel as distinct from the single-edge move endpoint.
    travel_start = destination_id
    route_queue: list[tuple[str, list[str]]] = [(travel_start, [travel_start])]
    route_seen = {travel_start}
    reachable_paths: list[list[str]] = []
    while route_queue:
        current_id, path = route_queue.pop(0)
        if current_id != travel_start:
            reachable_paths.append(path)
        current_location = live_locations.get(current_id, {})
        for neighbor in sorted(str(x) for x in current_location.get("connected_ids", [])
                               if str(x) in live_locations):
            if neighbor not in route_seen:
                route_seen.add(neighbor)
                route_queue.append((neighbor, path + [neighbor]))
    check(reachable_paths, "the movement test world must have at least one reachable destination")
    candidate_path = max(reachable_paths, key=lambda path: (len(path), tuple(path)))
    travel_target = candidate_path[-1]
    travel_key = f"wanxiang-eval-travel-{time.time_ns()}"
    travel_request = {"character_id": str(character["id"]), "destination_id": travel_target}

    # The planning endpoint is the source of truth for weighted route costs and is read-only.
    status_plan, planned_envelope, plan_ms = request(
        args.base_url, "/api/plan-travel", "POST", travel_request, args.token
    )
    check(status_plan == 200 and isinstance(planned_envelope.get("plan"), dict),
          f"travel planning failed: HTTP {status_plan} {planned_envelope}")
    travel_plan = planned_envelope["plan"]
    expected_path = travel_plan.get("path")
    check(isinstance(expected_path, list) and len(expected_path) >= 2,
          f"travel plan returned no multi-hop route: {travel_plan}")
    check(expected_path[0] == destination_id and expected_path[-1] == travel_target,
          f"travel plan route endpoints mismatch: {expected_path}")
    check(travel_plan.get("can_afford") is True,
          f"test journey should be affordable: {travel_plan}")
    check(int(planned_envelope.get("tick", -1)) == initial_tick,
          "read-only travel planning unexpectedly advanced world time")
    status_after_plan, state_after_plan, _ = request(
        args.base_url, "/api/state", token=args.token
    )
    check(status_after_plan == 200, "could not read state after travel planning")
    check(int(state_after_plan.get("info", {}).get("tick", -1)) == initial_tick,
          "travel planning unexpectedly changed the world's tick")
    planned_travel = state_after_plan.get("snapshot", {}).get("travel", {})
    original_travel = before_invalid_move.get("snapshot", {}).get("travel", {})
    check(planned_travel.get("supplies_remaining") == original_travel.get("supplies_remaining"),
          "read-only travel planning unexpectedly consumed supplies")

    status_travel, travelled, travel_ms = request(
        args.base_url, "/api/travel", "POST", travel_request, args.token, travel_key
    )
    check(status_travel == 200 and travelled.get("moved") is True,
          f"simulated travel failed: HTTP {status_travel} {travelled}")
    travel_report = travelled.get("travel", {})
    check(travelled.get("path") == expected_path,
          f"travel execution differs from the read-only plan: expected {expected_path}, got {travelled.get('path')}")
    check(travel_report.get("estimated_ticks") == travel_plan.get("estimated_ticks"),
          "travel tick estimate differs from its read-only plan")
    check(travel_report.get("route_cost") == travel_plan.get("route_cost"),
          "travel route cost differs from its read-only plan")
    check(travel_report.get("supplies_consumed") == travel_plan.get("supplies_required"),
          "travel supply consumption differs from its read-only plan")
    expected_final_tick = initial_tick + int(travel_report.get("elapsed_ticks", 0))
    check(int(travelled.get("tick", -1)) == expected_final_tick and expected_final_tick > initial_tick,
          f"simulated travel must advance world time by elapsed travel ticks ({expected_final_tick})")
    final_entity = next((e for e in travelled.get("snapshot", {}).get("entities", [])
                         if str(e.get("id")) == str(character["id"])), None)
    check(final_entity is not None and str(final_entity.get("location_id")) == travel_target,
          "travel snapshot does not show the final destination")
    expected_hops = expected_path[1:]
    travel_events = [
        event for event in travelled.get("snapshot", {}).get("events", [])
        if isinstance(event, dict)
        and str(character["id"]) in event.get("actors", [])
        and "移动到" in str(event.get("description", ""))
    ]
    for hop in expected_hops:
        check(any(str(event.get("location")) == hop for event in travel_events),
              f"travel event missing for route hop {hop}")
    status_travel_repeat, travelled_repeat, travel_repeat_ms = request(
        args.base_url, "/api/travel", "POST", travel_request, args.token, travel_key
    )
    check(status_travel_repeat == 200 and travelled_repeat == travelled,
          "duplicate travel idempotency key must replay the original result")
    measurements.append({
        "name": "multi-hop-shortest-path-travel",
        "latency_ms": round(travel_ms, 3),
        "repeat_latency_ms": round(travel_repeat_ms, 3),
        "route": expected_path,
        "hop_count": len(expected_path) - 1,
        "planning_latency_ms": round(plan_ms, 3),
        "planned_travel_is_read_only": True,
        "travel_ticks": travel_report.get("elapsed_ticks"),
        "travel_supplies_consumed": travel_report.get("supplies_consumed"),
    })

    idempotency_key = f"wanxiang-eval-step-{time.time_ns()}"
    status_a, step_a, ms_a = request(args.base_url, "/api/step", "POST", {"ticks": 1},
                                    args.token, idempotency_key)
    status_b, step_b, ms_b = request(args.base_url, "/api/step", "POST", {"ticks": 1},
                                    args.token, idempotency_key)
    check(status_a == 200 and status_b == 200, f"idempotent step failed: {status_a}, {status_b}")
    check(step_a.get("tick") == step_b.get("tick"), "duplicate Idempotency-Key advanced world twice")
    status, after_step, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_step["info"]["tick"]) == initial_tick + 1, "single idempotent step did not advance exactly once")
    after_persistence = after_step.get("persistence")
    check(isinstance(after_persistence, dict) and after_persistence.get("degraded") is False,
          f"persistence degraded after a successful step: {after_persistence}")
    measurements.append({"name": "idempotency-exactly-once", "first_latency_ms": round(ms_a, 3),
                         "repeat_latency_ms": round(ms_b, 3)})

    if args.agent:
        task = "Use world_info to inspect the current seed, tick and conservation status. Do not advance the world. Report only those observed values."
        status, agent, ms = request(args.base_url, "/api/agent", "POST", {"task": task}, args.token)
        check(status == 200, f"agent route unavailable: HTTP {status} {agent}")
        check(isinstance(agent.get("steps"), list), "agent response omitted auditable tool steps")
        measurements.append({"name": "agent-world-info", "latency_ms": round(ms, 3),
                             "success": agent.get("success"), "steps": len(agent.get("steps", []))})

    print(json.dumps({"status": "passed", "base_url": args.base_url, "checks": measurements,
                      "final_tick": int(after_step["info"]["tick"])}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, ValueError, KeyError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
