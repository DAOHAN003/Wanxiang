#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
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
      { timeout: 5000 }
    );
    await page.locator("#timelineClear").click();

    await page.locator("#altBtn").click();
    await page.waitForFunction(
      () => document.querySelector("#loadStatus")?.className.includes("ok"),
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
      { timeout: 15000 }
    );
    await page.waitForFunction(
      () => {
        const button = document.querySelector("#moveCharacterNow");
        return !!button && !button.disabled;
      },
      { timeout: 10000 }
    );

    const characterId = await page.locator("#moveCharacter").inputValue();
    const destinationId = await page.locator("#moveDestination").inputValue();
    assert.ok(characterId, "a living resident should be selectable");
    assert.ok(destinationId, "a reachable location should be selectable");
    await page.locator("#moveCharacterNow").click();
    await page.waitForFunction(
      () => (document.querySelector("#moveOutput")?.textContent || "").includes('"moved": true'),
      { timeout: 15000 }
    );
    const movement = JSON.parse(await page.locator("#moveOutput").textContent());
    assert.equal(movement.character_id, characterId);
    assert.equal(movement.destination_id, destinationId);
    assert.ok(Array.isArray(movement.path) && movement.path.length >= 2,
      "travel result should include the planned route, not just change a field");
    assert.equal(movement.path[0] === destinationId, false, "travel path should start at the origin, not the destination");
    assert.equal(movement.path.at(-1), destinationId, "travel path should end at the selected destination");
    assert.equal(movement.persistent_checkpoint, true, "API should report checkpoint persistence");
    assert.match(await page.locator("#loadStatus").textContent(), /角色移动完成/,
      "live movement should refresh the browser from the API's returned snapshot");

    assert.deepEqual(pageErrors, [], "browser runtime should not report uncaught page errors");
    console.log(JSON.stringify({
      status: "passed",
      checks: [
        "seeded location data and 2.5D projection",
        "location selection and spatial inspector",
        "event timeline filtering",
        "baseline/altered snapshot controls",
        "browser CORS connection to live Rust API",
        "route travel through API and rendered snapshot refresh",
      ],
      character_id: characterId,
      destination_id: destinationId,
      route: movement.path,
    }, null, 2));
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error && error.stack ? error.stack : String(error));
  process.exitCode = 1;
});
