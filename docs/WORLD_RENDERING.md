# Wanxiang world rendering

The browser renderer consumes the existing Rust `WorldExport` snapshot. It currently offers three complementary views:

- **World/faction structure:** faction nodes and character nodes are laid out deterministically for comparison; selecting a faction highlights its members and selecting a character opens an inspector.
- **Faction relationships:** trade and war edges are drawn only when the snapshot contains relationship records whose endpoints resolve to exported factions. Tooltips expose source fields such as trade volume, reciprocity, war status, intensity, and tick.
- **Location network:** the new `locations` array exports each location's id, name, environment, danger, resource richness, secret-realm flag, and declared `connected_ids`. The viewer displays actual location connections and character `location_id` assignments.
- **Event timeline:** recorded events are aggregated into tick bins; selecting a bin filters the corresponding event list.

## Rendering versus geography

The current world schema has no latitude/longitude or other authored spatial coordinates. The graph layout is generated for legibility and repeatability, not as a physical map. It must not be interpreted as geographic distance, travel time, or a simulated route. A location without `connected_ids` remains an unconnected node; the viewer deliberately does not invent road links.

## Validation

`python scripts/test-world-renderer.py` checks that the UI entry points, source data relationships, and export contract remain connected. CI also runs `node --check` on the inline browser application script, and the Rust tests check same-seed location export determinism and connection references.
