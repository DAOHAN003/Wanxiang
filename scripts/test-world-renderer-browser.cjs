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


    // Exercise the resource-gather panel against the real, running Rust API.
    await page.waitForFunction(() => {
      const button = document.querySelector("#gatherResourceNow");
      return !!button && !button.disabled;
    }, undefined, { timeout: 10000 });
    const gatherCharacterId = await page.locator("#gatherCharacter").inputValue();
    const gatherResource = await page.locator("#gatherResource").inputValue();
    assert.ok(gatherCharacterId && gatherResource, "a living resident with inventory should be selectable");
    const beforeGather = await page.evaluate(async ({ characterId, resource }) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const character = state.snapshot.entities.find((item) => String(item.id) === String(characterId));
      const location = state.snapshot.locations.find((item) => String(item.id) === String(character.location_id));
      const faction = state.snapshot.factions.find((item) => String(item.id) === String(character.faction_id));
      const row = state.snapshot.resources.resource_ledger.find((item) => item.resource === resource);
      return {
        tick: state.info.tick, locationId: location.id, factionId: faction.id,
        stock: Number(location.inventory[resource]),
        price: Number(location.market_prices && location.market_prices[resource]),
        factionStock: Number((faction.resources.find((item) => item[0] === resource) || [resource, 0])[1]),
        totalStock: Number(row.total_stock),
      };
    }, { characterId: gatherCharacterId, resource: gatherResource });
    assert.ok(Number.isFinite(beforeGather.price) && beforeGather.price > 0, "snapshot should include a positive market quote");
    assert.ok(beforeGather.stock >= 1, "selected resource should have at least one unit");
    await page.locator("#gatherResourceNow").click();
    await page.waitForFunction(() => {
      try { return JSON.parse(document.querySelector("#gatherOutput")?.textContent || "{}").gathered === true; }
      catch (_) { return false; }
    }, undefined, { timeout: 10000 });
    const gatherResult = JSON.parse(await page.locator("#gatherOutput").textContent());
    assert.equal(gatherResult.report.character_id, gatherCharacterId);
    assert.equal(gatherResult.report.resource, gatherResource);
    assert.equal(gatherResult.persistent_checkpoint, true);
    const afterGather = await page.evaluate(async ({ resource, locationId, factionId }) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const location = state.snapshot.locations.find((item) => String(item.id) === String(locationId));
      const faction = state.snapshot.factions.find((item) => String(item.id) === String(factionId));
      const row = state.snapshot.resources.resource_ledger.find((item) => item.resource === resource);
      return {
        tick: state.info.tick, stock: Number(location.inventory[resource]),
        factionStock: Number((faction.resources.find((item) => item[0] === resource) || [resource, 0])[1]),
        totalStock: Number(row.total_stock),
      };
    }, { resource: gatherResource, locationId: beforeGather.locationId, factionId: beforeGather.factionId });
    assert.equal(afterGather.tick, beforeGather.tick, "gathering should leave world time unchanged");
    assert.ok(Math.abs(afterGather.stock - (beforeGather.stock - gatherResult.report.amount)) < 1e-7, "location inventory should decrease");
    assert.ok(Math.abs(afterGather.factionStock - (beforeGather.factionStock + gatherResult.report.amount)) < 1e-7, "faction stock should increase");
    assert.ok(Math.abs(afterGather.totalStock - beforeGather.totalStock) < 1e-7, "gathering should conserve total stock");
    await page.locator("#ticksPerStep").fill("1");
    await page.locator("#stepLive").click();
    await page.waitForFunction(async (oldTick) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      return Number(state.info.tick) > oldTick;
    }, afterGather.tick, { timeout: 10000 });
    const afterRegen = await page.evaluate(async (resource) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const row = state.snapshot.resources.resource_ledger.find((item) => item.resource === resource);
      return { totalStock: Number(row.total_stock) };
    }, gatherResource);
    assert.ok(afterRegen.totalStock > afterGather.totalStock, "one live tick should regenerate resource stocks");

    // Exercise the new live market buy/sell controls and verify both resource ledgers.
    const tradeSetup = await page.evaluate(async () => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const character = state.snapshot.entities.find((item) => String(item.id) === String(document.querySelector("#gatherCharacter").value));
      const location = state.snapshot.locations.find((item) => String(item.id) === String(character?.location_id));
      const faction = state.snapshot.factions.find((item) => String(item.id) === String(character?.faction_id));
      const factionResources = Object.fromEntries(faction.resources || []);
      const resources = Object.keys(location?.inventory || {}).sort();
      for (const resource of resources) {
        if (Number(location.inventory[resource]) < 1 || !(Number(location.market_prices?.[resource]) > 0)) continue;
        const paymentResource = resources.find((key) => key !== resource
          && Number(location.inventory[key]) >= 2
          && Number(factionResources[key] || 0) >= 2);
        if (paymentResource) {
          return {
            characterId: character.id, locationId: location.id, factionId: faction.id,
            resource, paymentResource, tick: state.info.tick,
            totals: Object.fromEntries(state.snapshot.resources.resource_ledger
              .filter((row) => [resource, paymentResource].includes(row.resource))
              .map((row) => [row.resource, Number(row.total_stock)])),
          };
        }
      }
      return null;
    });
    assert.ok(tradeSetup, "world should expose a stocked resource and a distinct payment resource for a market trade");
    await page.locator("#marketTradeResource").selectOption(tradeSetup.resource);
    await page.locator("#marketTradePayment").selectOption(tradeSetup.paymentResource);
    await page.locator("#marketTradeAmount").fill("0.1");
    await page.locator("#marketTradeSide").selectOption("buy");
    await page.waitForFunction(() => {
      const button = document.querySelector("#marketTradeNow");
      return !!button && !button.disabled;
    }, undefined, { timeout: 5000 });
    await page.locator("#marketTradeNow").click();
    await page.waitForFunction(() => {
      try { return JSON.parse(document.querySelector("#marketTradeOutput")?.textContent || "{}").traded === true; }
      catch (_) { return false; }
    }, undefined, { timeout: 10000 });
    const buyResult = JSON.parse(await page.locator("#marketTradeOutput").textContent());
    assert.equal(buyResult.report.side, "buy");
    assert.equal(buyResult.report.resource, tradeSetup.resource);
    assert.equal(buyResult.report.payment_resource, tradeSetup.paymentResource);
    assert.equal(Number(buyResult.tick), Number(tradeSetup.tick), "market buy must not advance world time");

    const checkTradeLedgers = async (expectedTick) => page.evaluate(async ({ resource, paymentResource, expectedTick }) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const rows = Object.fromEntries(state.snapshot.resources.resource_ledger.map((row) => [row.resource, Number(row.total_stock)]));
      return { tick: Number(state.info.tick), resourceTotal: rows[resource], paymentTotal: rows[paymentResource] };
    }, { resource: tradeSetup.resource, paymentResource: tradeSetup.paymentResource, expectedTick });
    let afterTrade = await checkTradeLedgers(tradeSetup.tick);
    assert.equal(afterTrade.tick, Number(tradeSetup.tick), "market buy should not move the clock");
    assert.ok(Math.abs(afterTrade.resourceTotal - tradeSetup.totals[tradeSetup.resource]) < 1e-7, "buy must conserve commodity stock");
    assert.ok(Math.abs(afterTrade.paymentTotal - tradeSetup.totals[tradeSetup.paymentResource]) < 1e-7, "buy must conserve payment-resource stock");

    await page.locator("#marketTradeSide").selectOption("sell");
    await page.waitForFunction(() => {
      const button = document.querySelector("#marketTradeNow");
      return !!button && !button.disabled;
    }, undefined, { timeout: 5000 });
    await page.locator("#marketTradeNow").click();
    await page.waitForFunction(() => {
      try { return JSON.parse(document.querySelector("#marketTradeOutput")?.textContent || "{}").traded === true
        && JSON.parse(document.querySelector("#marketTradeOutput").textContent).report.side === "sell"; }
      catch (_) { return false; }
    }, undefined, { timeout: 10000 });
    const sellResult = JSON.parse(await page.locator("#marketTradeOutput").textContent());
    assert.equal(sellResult.report.side, "sell");
    afterTrade = await checkTradeLedgers(tradeSetup.tick);
    assert.equal(afterTrade.tick, Number(tradeSetup.tick), "market sell should not move the clock");
    assert.ok(Math.abs(afterTrade.resourceTotal - tradeSetup.totals[tradeSetup.resource]) < 1e-7, "sell must conserve commodity stock");
    assert.ok(Math.abs(afterTrade.paymentTotal - tradeSetup.totals[tradeSetup.paymentResource]) < 1e-7, "sell must conserve payment-resource stock");

    // Configure a staffed production site through the UI, then verify a real tick consumes input.
    const productionSetup = await page.evaluate(async () => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const factions = Object.fromEntries(state.snapshot.factions.map((faction) => [String(faction.id), faction]));
      for (let index = 0; index < state.production_sites.length; index++) {
        const site = state.production_sites[index];
        if (Number(site.workers_available) < 1) continue;
        const faction = factions[String(site.faction_id)];
        const inputRow = faction?.resources?.find((row) => Array.isArray(row) && Number(row[1]) >= 1);
        if (!inputRow) continue;
        const resources = Object.fromEntries(faction.resources || []);
        return {
          siteIndex: index, factionId: site.faction_id, locationId: site.location_id,
          inputResource: String(inputRow[0]), inputBefore: Number(inputRow[1]),
          outputBefore: Number(resources["浏览器制品"] || 0), tick: Number(state.info.tick),
        };
      }
      return null;
    });
    assert.ok(productionSetup, "a staffed faction-owned production site should be available in the live world");
    await page.locator("#productionSite").selectOption(String(productionSetup.siteIndex));
    await page.locator("#productionInputResource").selectOption(productionSetup.inputResource);
    await page.locator("#productionOutputResource").fill("浏览器制品");
    await page.locator("#productionWorkers").fill("1");
    await page.waitForFunction(() => {
      const button = document.querySelector("#productionOrderNow");
      return !!button && !button.disabled;
    }, undefined, { timeout: 5000 });
    await page.locator("#productionOrderNow").click();
    await page.waitForFunction(() => {
      try { return JSON.parse(document.querySelector("#productionOutput")?.textContent || "{}").configured === true; }
      catch (_) { return false; }
    }, undefined, { timeout: 10000 });
    const configuredOrder = JSON.parse(await page.locator("#productionOutput").textContent());
    assert.equal(configuredOrder.order.input_resource, productionSetup.inputResource);
    assert.equal(Number(configuredOrder.tick), productionSetup.tick, "configuring a production order must not advance time");

    await page.locator("#ticksPerStep").fill("1");
    await page.locator("#stepLive").click();
    await page.waitForFunction(async (oldTick) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      return Number(state.info.tick) > oldTick;
    }, productionSetup.tick, { timeout: 10000 });
    const productionAfter = await page.evaluate(async ({ factionId, inputResource }) => {
      const endpoint = document.querySelector("#apiUrl").value.replace(/\/+$/, "");
      const state = await (await fetch(endpoint + "/api/state")).json();
      const faction = state.snapshot.factions.find((item) => String(item.id) === String(factionId));
      const resources = Object.fromEntries(faction.resources || []);
      return { inputStock: Number(resources[inputResource] || 0), outputStock: Number(resources["浏览器制品"] || 0),
        orders: state.production_orders || [] };
    }, { factionId: productionSetup.factionId, inputResource: productionSetup.inputResource });
    assert.ok(Math.abs(productionAfter.inputStock - (productionSetup.inputBefore - 1)) < 1e-7,
      "one staffed worker must consume one input unit per tick");
    assert.ok(Math.abs(productionAfter.outputStock - (productionSetup.outputBefore + 1)) < 1e-7,
      "one staffed worker must produce one output unit per tick");
    assert.ok(productionAfter.orders.some((order) => order.order_id === configuredOrder.order.order_id),
      "production order must persist through the next tick");

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
        "live resource-gather UI, dynamic market quotes and ledger conservation",
        "location inventory regeneration after a live tick",
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
