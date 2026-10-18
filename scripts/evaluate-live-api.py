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
    event_snapshot = travelled.get("snapshot", {})
    travel_events = [
        event for event in event_snapshot.get("events", [])
        if isinstance(event, dict)
        and str(character["id"]) in event.get("actors", [])
        and "移动到" in str(event.get("description", ""))
    ]
    visible_hops = {str(event.get("location")) for event in travel_events}
    missing_hops = [str(hop) for hop in expected_hops if str(hop) not in visible_hops]
    snapshot_meta = event_snapshot.get("meta", {})
    events_truncated = snapshot_meta.get("events_truncated") is True

    # Live API snapshots intentionally retain only the latest event window. A long trip can
    # evict early per-hop events, but the visible travel events must still be a contiguous
    # suffix ending at the final hop, and truncation must be explicitly reported.
    visible_indices = [index for index, hop in enumerate(expected_hops)
                       if str(hop) in visible_hops]
    check(visible_indices and visible_indices[-1] == len(expected_hops) - 1,
          f"final travel hop event is missing from the live event window: "
          f"expected final hop {expected_hops[-1]}, visible hops {sorted(visible_hops)}")
    check(visible_indices == list(range(visible_indices[0], len(expected_hops))),
          f"travel events are missing inside the retained route suffix: "
          f"expected {expected_hops}, visible indices {visible_indices}")
    if missing_hops:
        check(events_truncated,
              f"travel events missing for route hops {missing_hops} without an explicit truncation marker")
    else:
        check(not events_truncated or visible_indices == list(range(len(expected_hops))),
              "untruncated travel response did not include every route-hop event")
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
    expected_after_step_tick = expected_final_tick + 1
    check(int(step_a.get("tick", -1)) == expected_after_step_tick,
          f"single idempotent step returned the wrong tick: expected {expected_after_step_tick}, got {step_a.get('tick')}")
    status, after_step, _ = request(args.base_url, "/api/state", token=args.token)
    check(int(after_step["info"]["tick"]) == expected_after_step_tick,
          f"single idempotent step did not advance exactly once after travel: expected {expected_after_step_tick}, got {after_step['info']['tick']}")
    after_persistence = after_step.get("persistence")
    check(isinstance(after_persistence, dict) and after_persistence.get("degraded") is False,
          f"persistence degraded after a successful step: {after_persistence}")
    measurements.append({"name": "idempotency-exactly-once", "first_latency_ms": round(ms_a, 3),
                         "repeat_latency_ms": round(ms_b, 3)})

    status, gather_before, _ = request(args.base_url, "/api/state", token=args.token)
    check(status == 200, "could not read state before gathering resources")
    gather_snapshot_before = gather_before.get("snapshot", {})
    gather_character = next((entity for entity in gather_snapshot_before.get("entities", [])
                             if entity.get("alive") is True and entity.get("faction_id")
                             and entity.get("location_id")), None)
    check(gather_character is not None, "resource-gather test needs a living resident with a faction and location")
    gather_location = next((location for location in gather_snapshot_before.get("locations", [])
                            if str(location.get("id")) == str(gather_character["location_id"])), None)
    check(gather_location is not None, "resident location is missing from snapshot")
    gather_inventory = gather_location.get("inventory", {})
    gather_candidate = next(((key, float(value)) for key, value in sorted(gather_inventory.items())
                             if isinstance(value, (int, float)) and float(value) >= 1.0), None)
    check(gather_candidate is not None, "resident location has no gatherable resource stock")
    gather_resource, gather_stock = gather_candidate
    gather_before_row = next((row for row in gather_snapshot_before.get("resources", {}).get("resource_ledger", [])
                              if row.get("resource") == gather_resource), None)
    check(gather_before_row is not None, "resource aggregate ledger omitted the location's stock")
    gather_tick_before = int(gather_before.get("info", {}).get("tick", -1))
    gather_body = {"character_id": gather_character["id"], "resource": gather_resource, "amount": 1.0}
    gather_key = f"wanxiang-eval-gather-{time.time_ns()}"
    status_gather, gathered, gather_ms = request(args.base_url, "/api/gather", "POST", gather_body,
                                                  args.token, gather_key)
    check(status_gather == 200 and gathered.get("gathered") is True,
          f"resource gather failed: HTTP {status_gather} {gathered}")
    gather_report = gathered.get("report", {})
    check(gather_report.get("resource") == gather_resource and gather_report.get("amount") == 1.0,
          f"resource gather report does not match the request: {gather_report}")
    check(int(gathered.get("tick", -1)) == gather_tick_before,
          "resource gathering must not advance world time")
    check(gathered.get("persistent_checkpoint") is True, "resource gather did not report checkpoint persistence")
    gather_snapshot_after = gathered.get("snapshot", {})
    location_after = next((location for location in gather_snapshot_after.get("locations", [])
                           if str(location.get("id")) == str(gather_location["id"])), None)
    gather_after_row = next((row for row in gather_snapshot_after.get("resources", {}).get("resource_ledger", [])
                             if row.get("resource") == gather_resource), None)
    check(location_after is not None and gather_after_row is not None,
          "resource gather response omitted updated inventory or aggregate ledger")
    check(abs(float(location_after.get("inventory", {}).get(gather_resource, -1.0)) - (gather_stock - 1.0)) < 1e-7,
          "resource gather did not debit the location stock")
    check(abs(float(gather_after_row.get("location_stock", 0.0)) - (float(gather_before_row.get("location_stock", 0.0)) - 1.0)) < 1e-7,
          "resource ledger location total did not fall by the gathered amount")
    check(abs(float(gather_after_row.get("faction_stock", 0.0)) - (float(gather_before_row.get("faction_stock", 0.0)) + 1.0)) < 1e-7,
          "resource ledger faction total did not rise by the gathered amount")
    check(abs(float(gather_after_row.get("total_stock", 0.0)) - float(gather_before_row.get("total_stock", 0.0))) < 1e-7,
          "gathering must conserve each resource across location and faction stores")
    status_gather_repeat, gathered_repeat, gather_repeat_ms = request(args.base_url, "/api/gather", "POST", gather_body,
                                                                      args.token, gather_key)
    check(status_gather_repeat == 200 and gathered_repeat == gathered,
          "duplicate gather Idempotency-Key must replay the original response without double transfer")
    measurements.append({"name": "location-resource-transfer", "latency_ms": round(gather_ms, 3),
                         "repeat_latency_ms": round(gather_repeat_ms, 3), "resource": gather_resource,
                         "tick_unchanged": True, "resource_total_conserved": True})

    # Buy and sell through the same endpoint; both the commodity and payment-resource
    # ledgers must remain conserved, and a repeated idempotency key must not trade twice.
    trade_snapshot = gather_snapshot_after
    trade_location = next((location for location in trade_snapshot.get("locations", [])
                           if str(location.get("id")) == str(gather_location["id"])), None)
    trade_faction = next((faction for faction in trade_snapshot.get("factions", [])
                          if str(faction.get("id")) == str(gather_character["faction_id"])), None)
    check(trade_location is not None and trade_faction is not None,
          "market trade snapshot omitted the character location or faction")
    trade_faction_resources = {str(item[0]): float(item[1]) for item in trade_faction.get("resources", [])
                               if isinstance(item, list) and len(item) == 2}
    trade_inventory = trade_location.get("inventory", {})
    trade_resource = next((key for key, value in sorted(trade_inventory.items())
                           if key != gather_resource and isinstance(value, (int, float))
                           and float(value) >= 1.0 and key in trade_faction_resources), gather_resource)
    trade_price = float(trade_location.get("market_prices", {}).get(trade_resource, 0.0))
    trade_payment_resource = next((key for key, value in sorted(trade_inventory.items())
                                   if key != trade_resource and isinstance(value, (int, float))
                                   and float(value) >= 2.0 and trade_faction_resources.get(key, 0.0) >= 2.0), None)
    if trade_resource == gather_resource:
        trade_price = float(trade_location.get("market_prices", {}).get(trade_resource, 0.0))
    check(trade_price > 0.0 and trade_payment_resource is not None,
          "market trade test needs two distinct stocked resources and a positive quote")
    trade_rows_before = {row["resource"]: float(row["total_stock"])
                         for row in trade_snapshot.get("resources", {}).get("resource_ledger", [])}
    trade_tick_before = int(gathered.get("tick", -1))
    trade_amount = 0.1
    buy_body = {"character_id": gather_character["id"], "side": "buy",
                "resource": trade_resource, "payment_resource": trade_payment_resource, "amount": trade_amount}
    buy_key = f"wanxiang-eval-trade-buy-{time.time_ns()}"
    status_buy, bought, buy_ms = request(args.base_url, "/api/trade", "POST", buy_body, args.token, buy_key)
    check(status_buy == 200 and bought.get("traded") is True,
          f"market buy failed: HTTP {status_buy} {bought}")
    buy_report = bought.get("report", {})
    check(buy_report.get("side") == "buy" and buy_report.get("resource") == trade_resource
          and buy_report.get("payment_resource") == trade_payment_resource,
          f"market buy report does not match request: {buy_report}")
    check(int(bought.get("tick", -1)) == trade_tick_before, "market buy must not advance world time")
    buy_snapshot = bought.get("snapshot", {})
    buy_rows = {row["resource"]: float(row["total_stock"])
                for row in buy_snapshot.get("resources", {}).get("resource_ledger", [])}
    for resource_key in (trade_resource, trade_payment_resource):
        check(abs(buy_rows.get(resource_key, 0.0) - trade_rows_before.get(resource_key, 0.0)) < 1e-7,
              f"market buy did not conserve resource total for {resource_key}")
    status_buy_repeat, bought_repeat, buy_repeat_ms = request(
        args.base_url, "/api/trade", "POST", buy_body, args.token, buy_key)
    check(status_buy_repeat == 200 and bought_repeat == bought,
          "duplicate market-buy Idempotency-Key must replay the original response")

    sell_amount = 0.05
    sell_body = {"character_id": gather_character["id"], "side": "sell",
                 "resource": trade_resource, "payment_resource": trade_payment_resource, "amount": sell_amount}
    sell_key = f"wanxiang-eval-trade-sell-{time.time_ns()}"
    status_sell, sold, sell_ms = request(args.base_url, "/api/trade", "POST", sell_body, args.token, sell_key)
    check(status_sell == 200 and sold.get("traded") is True,
          f"market sell failed: HTTP {status_sell} {sold}")
    check(int(sold.get("tick", -1)) == trade_tick_before, "market sell must not advance world time")
    sell_rows = {row["resource"]: float(row["total_stock"])
                 for row in sold.get("snapshot", {}).get("resources", {}).get("resource_ledger", [])}
    for resource_key in (trade_resource, trade_payment_resource):
        check(abs(sell_rows.get(resource_key, 0.0) - trade_rows_before.get(resource_key, 0.0)) < 1e-7,
              f"market sell did not conserve resource total for {resource_key}")
    measurements.append({"name": "market-buy-sell", "buy_latency_ms": round(buy_ms, 3),
                         "buy_repeat_latency_ms": round(buy_repeat_ms, 3),
                         "sell_latency_ms": round(sell_ms, 3), "resources_conserved": True,
                         "tick_unchanged": True})

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
