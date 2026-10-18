#!/usr/bin/env python3
"""Contract checks for the data-driven Wanxiang browser renderer."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HTML = (ROOT / "site" / "index.html").read_text(encoding="utf-8")
EXPORT = (ROOT / "wanxiang_world" / "src" / "export.rs").read_text(encoding="utf-8")
VALIDATOR = (ROOT / "scripts" / "validate-web-demo.py").read_text(encoding="utf-8")

required_html = [
    'id="worldGraph"',
    'id="locationGraph"',
    'id="eventTimeline"',
    'function renderWorldGraph(w)',
    'function renderLocationGraph(w)',
    'loc.position',
    'location-height',
    '高程',
    '地形标签',
    'function renderEventTimeline(w)',
    'function drawFactionRelationEdges(root,w,positions)',
    "w.resources.trade_links",
    "Array.isArray(w.wars)",
    "loc.connected_ids",
    "e.location_id",
    "location-resident",
    "存活角色标记",
    "点选势力可筛选其成员",
    "不表示地图位置或距离",
    'aria-label="真实地点实体及其连接关系网络"',
    'id="movementControls"',
    'id="moveCharacterNow"',
    'id="moveCharacterStep"',
    'async function runCharacterStepMove()',
    'async function runCharacterMove()',
    'liveRequest("/api/move"',
    'liveRequest("/api/travel"',
    'function reachableLocationIds(locations,startId)',
    'WorldPosition',
    'function populateMovementControls(w)',
    'route-highlight',
    'route-origin',
    'route-destination',
]
for token in required_html:
    if token not in HTML:
        raise SystemExit(f"Renderer contract missing from site/index.html: {token}")

required_export = [
    "pub struct LocationExport",
    "pub locations: Vec<LocationExport>",
    "connected_ids",
    'dump!("locations.json", &export.locations)',
    "location_export_is_deterministic_and_preserves_connections",
    "position: Option<wanxiang_types::WorldPosition>",
    "terrain: Option<String>",
]
for token in required_export:
    if token not in EXPORT:
        raise SystemExit(f"Location export contract missing: {token}")

if '("meta", "entities", "factions", "locations", "events", "resources", "rules")' not in VALIDATOR:
    raise SystemExit("Web demo validator must require the locations snapshot array")

print("Validated interactive world/faction/location/event renderers and location export contract.")
