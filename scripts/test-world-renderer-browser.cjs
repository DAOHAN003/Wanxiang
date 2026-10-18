#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright");

async function main() {
  const siteUrl = process.env.WANXIANG_SITE_URL || "http://127.0.0.1:8000";
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));

  try {
    await page.goto(siteUrl, { waitUntil: "networkidle", timeout: 30000 });
    await page.waitForFunction(
      () => (document.querySelector("#loadStatus")?.textContent || "").includes("Rust 核心快照已加载"),
      undefined,
      { timeout: 30000 }
    );

    const locationNodes = page.locator("#locationGraph .location-node");
    assert.ok((await locationNodes.count()) >= 2, "browser page should render real seeded location entities");
    assert.match(await page.locator("#locationGraphState").textContent(), /2.5D 本地坐标投影/,
      "location graph should use exported local coordinates and elevation");
    await locationNodes.first().click();
    assert.notEqual((await page.locator("#locationSelection").textContent()).trim(), "整个地点网络",
      "selecting a location should open the location inspector");
    const inspectorText = await page.locator("#locationDetails").textContent();
    for (const field of ["空间坐标", "高程", "地形标签"]) {
      assert.ok(inspectorText.includes(field), `location inspector should show ${field}`);
    }

    const timelineBars = page.locator("#eventTimeline .timeline-bar");
    assert.ok((await timelineBars.count()) > 0, "timeline should draw tick bins");
    await timelineBars.first().click();
    await page.waitForFunction(
      () => (document.querySelector("#timelineCaption")?.textContent || "").includes("已筛选 tick"),
      undefined,
      { timeout: 5000 }
    );
    await page.locator("#timelineClear").click();

    await page.locator("#altBtn").click();
    await page.waitForFunction(
      () => document.querySelector("#loadStatus")?.className.includes("ok"),
      undefined,
      { timeout: 5000 }
    );
    await page.locator("#baseBtn").click();

    // Exercise the actual browser → HTTP API → WorldModel → snapshot → renderer loop.
    await page.locator("#connectLive").click();
    await page.waitForFunction(
      () => {
        const badge = document.querySelector("#liveBadge")?.textContent || "";
        return badge.includes("CONNECTED") || badge.includes("SERVER AUTO-TICK");
      },
      undefined,
      { timeout: 15000 }
    );
    await page.waitForFunction(
      () => {
        const button = document.querySelector("#moveCharacterNow");
        return !!button && !button.disabled;
      },
      undefined,
      { timeout: 10000 }
    );

    const characterId = await page.locator("#moveCharacter").inputValue();
    const destinationId = await page.locator("#moveDestination").inputValue();
    assert.ok(characterId, "a living resident should be selectable");
    assert.ok(destinationId, "a reachable location should be selectable");

    const beforePreview = await page.evaluate(async (id) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      return {
        tick: state.info.tick,
        supplies: state.snapshot.travel.supplies_remaining,
        location: state.snapshot.entities.find((entity) => entity.id === id)?.location_id,
      };
    }, characterId);
    await page.locator("#previewTravelNow").click();
    await page.waitForFunction(
      () => (document.querySelector("#travelPlanOutput")?.textContent || "").includes('"can_afford"'),
      undefined,
      { timeout: 10000 }
    );
    const previewEnvelope = JSON.parse(await page.locator("#travelPlanOutput").textContent());
    const preview = previewEnvelope.plan;
    assert.equal(preview.character_id, characterId);
    assert.equal(preview.destination_id, destinationId);
    assert.equal(preview.path.at(-1), destinationId);
    assert.equal(preview.can_afford, true, "browser should show an affordable route plan");
    assert.ok(preview.estimated_ticks > 0 && preview.supplies_required > 0,
      "travel plan should include time and supply estimates");
    const afterPreview = await page.evaluate(async (id) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      return {
        tick: state.info.tick,
        supplies: state.snapshot.travel.supplies_remaining,
        location: state.snapshot.entities.find((entity) => entity.id === id)?.location_id,
      };
    }, characterId);
    assert.deepEqual(afterPreview, beforePreview,
      "planning travel must not advance time, spend supplies, or change the character location");

    await page.locator("#moveCharacterNow").click();
    await page.waitForFunction(
      () => (document.querySelector("#moveOutput")?.textContent || "").includes('"moved": true'),
      undefined,
      { timeout: 15000 }
    );
    const movement = JSON.parse(await page.locator("#moveOutput").textContent());
    assert.equal(movement.character_id, characterId);
    assert.equal(movement.destination_id, destinationId);
    assert.ok(Array.isArray(movement.path) && movement.path.length >= 2,
      "travel result should include the planned route, not just change a field");
    assert.equal(movement.path[0] === destinationId, false, "travel path should start at the origin, not the destination");
    assert.equal(movement.path.at(-1), destinationId, "travel path should end at the selected destination");
    assert.ok((await page.locator("#locationGraph .route-highlight").count()) >= movement.path.length - 1,
      "rendered location network should highlight every traversed edge");
    assert.equal(await page.locator("#locationGraph .route-origin").count(), 1,
      "rendered route should identify the origin");
    assert.equal(await page.locator("#locationGraph .route-destination").count(), 1,
      "rendered route should identify the destination");
    assert.equal(movement.persistent_checkpoint, true, "API should report checkpoint persistence");
    assert.ok(movement.travel && movement.travel.elapsed_ticks > 0,
      "travel should advance simulation time and return its deterministic travel report");
    assert.equal(movement.travel.elapsed_ticks, movement.travel.estimated_ticks,
      "completed travel should consume the estimated route duration");
    assert.ok(movement.travel.supplies_consumed > 0,
      "travel should consume the abstract supply budget");
    assert.ok(movement.world_travel && movement.world_travel.supplies_remaining < 1000,
      "snapshot should expose the updated travel budget");
    assert.match(await page.locator("#loadStatus").textContent(), /旅行完成/,
      "live travel should refresh the browser from the API's returned snapshot");

    // Exercise the separate one-edge movement endpoint as well as multi-hop travel.
    const directMove = await page.evaluate(async (id) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const character = state.snapshot.entities.find((entity) => String(entity.id) === String(id));
      const origin = state.snapshot.locations.find((location) => String(location.id) === String(character?.location_id));
      const destinationId = (origin?.connected_ids || []).map(String).filter((value) => value !== String(origin.id)).sort()[0] || null;
      return { tick: state.info.tick, originId: origin?.id, destinationId };
    }, characterId);
    assert.ok(directMove.destinationId, "the resident should have a directly connected destination");
    await page.locator("#moveDestination").selectOption(directMove.destinationId);
    await page.waitForFunction(
      () => { const button = document.querySelector("#moveCharacterStep"); return !!button && !button.disabled; },
      undefined,
      { timeout: 5000 }
    );
    await page.locator("#moveCharacterStep").click();
    await page.waitForFunction(
      () => { try { return JSON.parse(document.querySelector("#moveOutput")?.textContent || "{}").kind === "move"; } catch (_) { return false; } },
      undefined,
      { timeout: 10000 }
    );
    const directMovement = JSON.parse(await page.locator("#moveOutput").textContent());
    assert.equal(directMovement.moved, true, "direct movement should use the /api/move endpoint");
    assert.equal(directMovement.character_id, characterId);
    assert.equal(directMovement.destination_id, directMove.destinationId);
    assert.equal(directMovement.persistent_checkpoint, true, "direct movement should persist the world checkpoint");
    const afterDirectMove = await page.evaluate(async (id) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      return { tick: state.info.tick, location: state.snapshot.entities.find((entity) => String(entity.id) === String(id))?.location_id };
    }, characterId);
    assert.deepEqual(afterDirectMove, { tick: directMove.tick, location: directMove.destinationId },
      "direct movement should change location without advancing world time");

    await page.locator("#locationZoomIn").click();
    assert.notEqual(await page.locator("#locationGraph").getAttribute("viewBox"), "0 0 1000 420",
      "zoom control should change the SVG viewport");
    await page.locator("#locationZoomReset").click();

    fs.mkdirSync(path.join(process.cwd(), "artifacts"), { recursive: true });
    await page.screenshot({
      path: path.join(process.cwd(), "artifacts", "world-lab-e2e.png"),
      fullPage: true,
      animations: "disabled",
    });

    // Stress the actual SVG renderer with a predictable 5k-location payload.
    const perf = await page.evaluate(() => {
      const locations = Array.from({ length: 5000 }, (_, i) => ({
        id: "perf_" + String(i).padStart(5, "0"),
        name: "地点 " + i,
        environment: "abstract",
        danger_level: (i % 100) / 100,
        resource_richness: (i % 10) / 10,
        is_secret_realm: false,
        terrain: "abstract",
        travel_cost: 1,
        position: { x: i % 100, y: Math.floor(i / 100), elevation: (i % 7) / 2 },
        connected_ids: ["perf_" + String((i + 1) % 5000).padStart(5, "0")],
      }));
      const start = performance.now();
      renderLocationGraph({ locations, entities: [] });
      const elapsedMs = performance.now() - start;
      return {
        location_count: locations.length,
        rendered_nodes: document.querySelectorAll("#locationGraph .location-node").length,
        rendered_edges: document.querySelectorAll("#locationGraph .location-edge").length,
        elapsed_ms: Math.round(elapsedMs * 100) / 100,
        status_text: document.querySelector("#locationGraphState")?.textContent || "",
      };
    });
    assert.equal(perf.location_count, 5000);
    assert.ok(perf.rendered_nodes <= 240,
      "large worlds must apply the configured SVG node cap instead of rendering all entities");
    assert.ok(perf.status_text.includes("5000") && perf.status_text.includes("已限制渲染规模"),
      "large-world status should make the render cap explicit");
    fs.writeFileSync(
      path.join(process.cwd(), "artifacts", "world-lab-performance.json"),
      JSON.stringify({ status: "passed", ...perf }, null, 2) + "\n",
      "utf8",
    );

    assert.deepEqual(pageErrors, [], "browser runtime should not report uncaught page errors");
    console.log(JSON.stringify({
      status: "passed",
      checks: [
        "seeded location data and 2.5D projection",
        "location selection and spatial inspector",
        "event timeline filtering",
        "baseline/altered snapshot controls",
        "browser CORS connection to live Rust API",
        "read-only travel preview with unchanged live state",
        "travel time, route risk, supplies and checkpoint persistence",
        "travel path highlight and zoom controls",
        "5k-location SVG render cap and performance sample",
        "browser screenshot artifact",
      ],
      character_id: characterId,
      destination_id: destinationId,
      route: movement.path,
      travel: movement.travel,
      performance: perf,
    }, null, 2));
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error && error.stack ? error.stack : String(error));
  process.exitCode = 1;
});
