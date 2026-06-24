# Plan: Munchii's Campaign — tile levels, a bestiary, and power-ups

> Status: **runtime spine playable.** Importer, Level IR, semantic tile art (all
> backends), and a playable runtime (camera / collision / hazard / goal / warp)
> are in. Munchii renders in-level. Bestiary + power-ups + compact sprite are next.
> Drafted 2026-06-22; updated 2026-06-24.
>
> **Layout note (2026-06-24):** the tree is now a Cargo workspace — the engine
> (incl. the shared sprite/tile/level/bestiary asset library) lives in `engine/`
> (crate `scamper`), and this game lives in `games/supermunchii/` (crate
> `supermunchii`). Paths below were updated where things actually moved; any
> remaining bare `src/…` in the speculative sections is now under `engine/src/…`.

## Progress log (what's done)

- **Importer** (`engine/src/level/import.rs`): `.tscn` → Level IR. Full scene classifier
  (145 source types → 0 unmapped), semantic tile kinds from the atlas legend
  (`atlas_kind`). Validated locally over all 305 levels (gitignored).
- **Level IR** (`engine/src/level/ir.rs`): line-oriented `*.lvl`, round-trip + ascii
  preview. Authored level shipped: `games/supermunchii/levels/yard-romp-1.lvl`.
- **Tile art** (`engine/src/level/art.rs`): a distinct 16×16 pattern per `TileKind`
  feeding all four tiers, 5 theme palettes, mono-distinctness enforced by test
  across every theme.
- **Viewers**: `supermunchii tiles` (grid) and the `tile-lab` binary (stepper, like
  sprite-lab); both in the `run.sh -i` menu.
- **Runtime** (`engine/src/level/world.rs` + `run_play` in `games/supermunchii/src/main.rs`): `LevelWorld`
  projects IR → solid `TileMap` + hazard/goal/warp/kind data; clamped side-scroll
  `camera`; `supermunchii play <level>` does collision, hazard/pit respawn, goal =
  LEVEL COMPLETE, pipe warps (`<id>@tx,ty`). Munchii renders via the sprite/pose
  system (overhangs the 1-tile hitbox — see next steps).
- **Menus**: play campaign (`games/supermunchii/levels/` picker), **browse imported levels**
  (navigates `imported/lvl/<game>/<world>` tree), record/replay, sprite/tile labs.
- **Importer: scene inheritance + theme inference (2026-06-24).** The importer now
  resolves Godot inherited scenes (`instance=ExtResource(base)`) by loading the base
  and overlaying derived nodes, and infers the level theme from the scene's
  `theme="…"` (mapping Godot's 16 themes → our 5), with a name-based fallback;
  `--theme` is now an optional override. All **306** source levels import (0 failures).
- **Play polish (2026-06-24).** Fixed tile flicker on the character tiers (snap the
  camera to whole cells on non-pixel-exact backends — `Backend::pixel_exact`); added
  auto-advance to the next sibling level on completion (debugging aid). Crash
  debugging: panics now log message + backtrace to `scamp.log` (`dbg::install_panic_logger`).

## Next steps (pick up here)

1. **Compact Munchii sprite (DECISION PENDING).** Big Munchii overhangs the
   1-tile hitbox badly in-level. Options: (a) author a compact ~1–2 tile beagle
   sprite for platforming (recommended), or (b) keep big Munchii + zoom the camera.
   Owner chose to evaluate by playtest first — revisit after running it.
2. **Jump/feel tuning against real geometry** — assert standard 4-tile gap / 4-high
   clearance with the default `FeelParams`; tune if needed (CAMPAIGN_PLAN §5).
3. **Sprite registry + first creatures** — generalize `munchii.rs` into a
   per-creature registry (glyph frames + palette), preview in a lab; author the
   first few (`boneling`, `rollo`, `kibble`, `big_kibble`) before mass-producing.
4. **Entities in the runtime** — render + step creatures/items (currently the IR
   carries them but `run_play` ignores them); collisions (pounce, collect).
5. **Breakable blocks behavior** — IR already marks `brick breakable=1` +
   `question contains=…`; wire bonk-from-below / smash-while-big in `run_play`.
6. **Red-team the runtime** — `run_play` itself hasn't had a review/playtest pass
   (only `LevelWorld`/`camera` are unit-tested). One-way platforms are solid for
   now; warp targets only exist on authored levels (imported pipes have none).

> Build/run: `./run.sh` → menu. Headless tests: `cargo test`. Imported levels are
> local-only (gitignored); re-import on a new machine with `supermunchii import`.

## Decisions (locked)

- **IP path: A.** The `.tscn` → Level-IR importer is a **dev-only tool**. Imported
  Nintendo-derived layouts are tested **locally and never committed** (gitignored:
  `*.tscn`, `imported/`). We **ship our own authored levels** in the IR format.
- **Importer: Rust parser** (`engine/src/level/import.rs`), no Godot toolchain dependency.
- **Level IR is line-oriented text** (`*.lvl`), not JSON. Same rationale as the
  capture files: no serde dependency (single-dep ethos), diff-friendly, trivially
  hand-authorable and hand-parseable. The JSON sketch in §4 is superseded by §4a.
- **Tile-data byte layout (verified):** `tile_map_data = PackedByteArray("base64")`;
  decoded = `u16` version, then 12-byte cells
  `x:i16, y:i16, source_id:u16, atlas_x:u16, atlas_y:u16, alt:u16` (little-endian).

## 1. Goal & shape

Play a full campaign of side-scrolling platform levels as Munchii: run, jump,
collect, dodge gentle critters, grab power-ups, reach the end-of-level marker.
Level *geometry* is imported from the fan-made "Super Mario Bros. Remastered"
Godot project (a large library of hand-built `.tscn` tile levels); **everything
the player sees and fights — creatures, power-ups, blocks, the boss, the
goal — is our own original, non-violent, dog-themed creation.** Munchii doesn't
stomp things to death; he pounces and they pop into treats.

Reuses what already exists: the tick-driven deterministic `Sim`
([RECORD_REPLAY.md]), the four-tier backend trait (kitty / text / ascii / mono),
the one-sprite-many-tiers rendering (glyph frames + a per-creature palette,
rasterized to blocks for the pixel tiers), the framebuffer, and — critically —
the capture/replay **golden-snapshot harness** as the regression net for every
new system.

### Non-goals (v1)
- No level editor, no online, no challenge/boo-race/endless modes.
- No bespoke per-creature Kitty pixel art at first — all four tiers derive from
  one glyph sprite + palette (the current Munchii approach). Hand-drawn hi-fi
  Kitty art is an *optional later* layer.
- We do not ship or depend on Nintendo/Godot assets at runtime (see §2).

## 2. Legal / IP — the load-bearing red-team item

This must be settled **before** any execution, and re-examined every red-team
pass. Three distinct buckets:

1. **Creatures, power-ups, blocks, boss, music, names, art — ours.** No reuse of
   Nintendo characters, sprites, audio, or names. The bestiary in §7 is original.
   This is the safe, fun part and the bulk of the creative work.
2. **The Godot `.tscn` files / TileSet atlases — a third party's recreation of
   Nintendo's copyrighted level designs.** We must not redistribute them, and
   shipping verbatim Nintendo level *geometry* (the exact 1-1 layout, etc.) is a
   real copyright/trademark risk even reskinned. Options, to decide with the user:
   - **(A) Importer-as-tool, private fixtures.** Build the loader; use a handful
     of imported levels only as *local, uncommitted* test input while developing.
     Ship the engine + our own authored levels, not Nintendo layouts. **Lowest
     risk; recommended.**
   - **(B) Transform, don't copy.** Use imports to seed a procedural/remixer that
     mutates geometry enough to be its own thing. Murky; still derivative.
   - **(C) Author original levels** in our own format (the engine's real value),
     using the importer only to bootstrap/learn. Most work, cleanest IP.
   - **(D) Get explicit permission/licence** from the fan project (itself
     unlicensed-by-Nintendo), which doesn't cure the underlying Nintendo IP.
   Recommendation: **build the importer (A), keep imported Nintendo layouts out
   of the repo, and treat authored/original levels as the shippable campaign.**
3. **Format compatibility is fine.** Reading a file format is not infringement;
   the risk is in the *content*. So the loader is safe to build regardless.

> Decision needed from you: which of A–D. The rest of this plan is written so the
> engine work is identical either way — only *which levels we commit/ship*
> changes.

## 3. Architecture overview (new + changed systems)

```
.tscn (Godot)  --offline importer-->  Level IR (our JSON)  --runtime loader-->  World
                                                                                   |
   Sim (tick-driven)  <-->  Entities (Munchii + creatures + items + blocks)  <-->  TileWorld
                                                                                   |
                                            Camera (scrolling viewport)  -->  Backend (4 tiers)
```

New or substantially-changed modules (proposed):
- `engine/src/level/import.rs` — **offline** `.tscn` → Level IR converter (dev-only).
- `engine/src/level/ir.rs` — the engine-native Level IR + JSON (de)serialization.
- `engine/src/level/world.rs` — extend `world.rs`: typed tiles (not just `solid`),
  multiple layers, level bounds, entity spawn table, theme/BGM id.
- `src/entity/*` — entity model + behaviors (creatures, items, blocks, boss).
- `src/sprites.rs` — generalize `munchii.rs` into a sprite/animation **registry**
  keyed by creature id; each entry = glyph frames + palette fn (all tiers free).
- `src/camera.rs` — scrolling viewport over a large tile world + visible-range
  culling; the present path renders a window, not the whole arena.
- `src/sim.rs` — extend `Sim` to step entities deterministically each tick.
- `main.rs` — a `play <level>` / campaign flow alongside the existing demo.

## 4. The level format & the importer

### What we observed
- `Tiles` / `DecoTiles` are Godot 4 `TileMapLayer`-style nodes whose
  `tile_map_data` is a `PackedByteArray`: a little-endian stream of per-cell
  records — `(cell_x:i16, cell_y:i16, source_id:i16, atlas_x:u16, atlas_y:u16,
  alternative:u16)` after a `u16` format header. **Exact byte layout must be
  verified** against the Godot version the repo targets (task: confirm record
  size/field order from Godot's `TileMapLayer` source before trusting the parse).
- Each `(source_id, atlas_x, atlas_y)` indexes a **TileSet** (`.tres`) that we
  also need, to know which atlas cell means ground vs brick vs pipe vs decoration.
  The importer must read the TileSet to build an `atlas → TileKind` table.
- Entities are `[node ...]` instances of child scenes (`Goomba.tscn`, etc.) with
  a `position` (pixels) and occasional exported properties. Container nodes
  (`Enemies`, `Blocks`) group them. Player start, `Checkpoint`, `EndFlagpole`,
  `PipeArea` warps, and a BGM json reference are nodes/props too.
- Coordinates are pixels on a 16px tile grid (matches our `TILE = 16.0`). Player
  start can be negative; level width ~3100px (~195 tiles) for 1-1.

### Importer strategy
Two viable parsers — decide in red-team:
- **Rust parser** in `engine/src/level/import.rs`: `.tscn` is INI-like text; parse
  sections, base64-decode `tile_map_data`, decode the cell stream, map atlas→kind
  via the TileSet, emit Level IR JSON. Self-contained, no Godot dependency.
- **Godot headless export script** (GDScript) that loads each scene and dumps our
  IR. Trivially correct (Godot decodes its own data) but adds a Godot toolchain
  dependency for the offline step only.

Recommendation: **Rust parser**, so the whole pipeline is one toolchain; fall
back to a Godot dump script if the binary tile format proves version-fragile.
Either way the importer is **offline/dev-only** — the runtime engine only ever
reads our IR, never `.tscn`.

### Level IR (engine-native, JSON)
```jsonc
{
  "id": "yard-1-1", "theme": "overworld", "bgm": "romp",
  "width_tiles": 195, "height_tiles": 15,
  "spawn": [2, 11], "bounds": {"left":0,"right":195},
  "tiles": [ /* run-length or sparse (x,y,kind) — kind: ground|brick|coinbrick|
               question|hidden|pipe|platform|hazard|deco|... */ ],
  "entities": [ {"type":"boneling","x":22,"y":11},
                {"type":"question","x":16,"y":7,"contains":"big_kibble"},
                {"type":"pipe","x":28,"y":9,"warp":"yard-1-1a@3,11"} ],
  "goal": {"type":"flag","x":198,"y":3},
  "checkpoints": [[100,11]]
}
```
TileKind and entity-type vocabularies are ours; the importer maps Godot
atlas/scene names onto them via a translation table we author once.

### 4a. Level IR — the actual on-disk format (`*.lvl`, line-oriented)

Supersedes the JSON sketch above. Header lines, then tile spans, then entities.
Coordinates are **tiles** (origin normalized to 0,0 by the importer). Comments
(`#`) and blank lines are ignored.

```
scamper-level v1
id yard-romp-1
theme overworld
size 48 15
spawn 2 11
goal flag 46 3
# tiles: single "T x y kind"  or horizontal run "R x y len kind"
R 0 13 48 ground
T 8 9 question
R 12 9 3 brick
# entities: "E type x y [k=v ...]"
E boneling 22 12
E pipe 28 11 warp=yard-romp-1a@3,12
# optional checkpoints: "C x y"
C 24 11
```

- `kind` ∈ `ground|brick|coinbrick|question|hidden|pipe|platform|hazard|deco`.
- **Tile kinds are semantic** (`atlas_kind`), derived from the project's shared
  `Tiles` tileset. The themed tilesets only re-texture one atlas layout, so the
  source→kind map is theme-independent:
  | atlas source | texture | → kind |
  |---|---|---|
  | 0 (row 5) | terrain, `one_way` tiles | `platform` (semisolid) |
  | 0 (other) | terrain | `ground` |
  | 1 | embedded solid blocks (scenes coll.) | `ground` |
  | 2 | `Liquids.png` (lava / deep water) | `hazard` |
  | 4, 5 | conveyor belts | `platform` |
  | 3, 6 | deco / edge-connection visuals | `deco` |
  The deco layer is forced to `deco` by node name. Validated over all 305 levels:
  ~21.5k `ground`, ~2k `hazard`, ~1.4k `deco`, ~1.2k `platform` spans.
- **Known refinement:** terrain layers don't carry a foreground/background flag,
  so a purely-decorative background terrain tile still imports as solid `ground`
  (errs toward solid — collision-safe). Distinguishing requires per-layer metadata.
- The importer maps an instanced scene's basename (`Goomba` → `boneling`, …) via
  a scene→type table; unmapped → `unknown:<Name>` (kept, flagged, never silently
  dropped). `Player`→spawn, `EndFlagpole`→goal, `Checkpoint`→`C`.

### 4b. Import coverage (validated against all 305 levels, local)

The importer's classifier (`classify_scene`) was run over every level in all four
games (SMB1 / SMBANN / SMBLL / SMBS): **22k tile-spans, ~4.7k entity placements,
145 distinct source types → 0 unmapped.** Each source scene resolves to one of:

- **Entity** — a creature/item/block/platform in our vocabulary (below).
- **Warp** — pipe/teleport/warp triggers → an entity of type `warp`.
- **Exit** — underground/underwater/ending doors → the `goal` (or a spare `exit`).
- **Drop** — engine plumbing kept out of the IR: backgrounds, drop-shadow
  renderers, generator-stoppers, race/challenge logic, pick-a-path nodes, camera
  `*Limit` markers, `*Area` zones (water/wind/gravity — revisited as features
  later), pure-visual castle decos, and sub-area level links (scenes named like
  `1-1a` / `8-4`).

**Our entity vocabulary** (original Munchii designs; the source names are only
translation keys). Creatures: `boneling`, `rollo`/`rollo_sun`, `flutterbug`,
`hoppa`, `pincher`, `dandi`/`dandi_sun`, `hardhat`, `stick_squirrel`, `sticker`,
`zoomdisc`/`zoomdisc_launcher`, `sudsfish`/`sudsfish_sun`, `moppet`, `puffer`,
`rattle`, `pop`, `sprinkler_bar`, `drip`, `log`, `blowdryer`, `fan`,
`baron_whiskers`, `spawner`. Items: `kibble`, `big_kibble`, `bubble_bone`,
`zoomies_treat`, `lucky_squeaky`, `flutter_collar`. Blocks: `question`, `brick`,
`pswitch`, `ivy`. Platforms: `platform` (`move=falling|lift|sideways|vertical|
travel|cloud`), `trampoline`. Features: `warp`, `bath_plug`, `rescued_pup`.

**Breakable blocks (per your note).** Block entities carry the props the runtime
needs: `brick` → `breakable=1` (+ a `contains` for coin/power bricks); `question`
→ `contains=<item>` and `hidden=1` for invisible ones; poison blocks →
`contains=poison`. The actual smash-from-below / smash-while-big behavior is a
**runtime** milestone (§10 step 3) — the IR now records *what's breakable and
what it holds* so wiring it up is just behavior, not re-derivation.

## 5. Physics & feel retune (the "make the jump make sense" work)

The demo blows Munchii up to a 19-cell-wide sprite filling the box. Tile
platforming needs a **compact Munchii** on the 16px grid:
- **Hitbox:** small Munchii ≈ 12×16px (≈¾×1 tile — the engine's native
  `Player::new` default!); big Munchii ≈ 12×24–28px (1×~1.75 tiles).
- **Sprite:** a new compact sprite set (~2–3 glyph rows small, ~4 rows big),
  separate from the big demo sprite. The demo arena keeps the big sprite.
- **Tuning targets** (define, then solve `FeelParams` to hit them; all in tiles):
  | Quantity | Target |
  |---|---|
  | Max jump height (full hold) | ~4 tiles (clear a 4-high obstacle) |
  | Min jump height (tap) | ~1.5 tiles |
  | Running jump distance | ~4–5 tile gap clearable at top speed |
  | Run top speed | ~ level "fast" pace (tunable) |
  | Coyote / buffer | keep current generous values |
- The importer should let us **assert** these against representative geometry
  (e.g. the standard 4-tile gap and 4-high pipe) so a feel change that breaks a
  jump is caught. Determinism is preserved — retuning is just constants.
- Tile size stays `TILE = 16`. Big change is **camera + variable hitbox +
  power-state height**, not the integrator.

## 6. Rendering & camera

- Levels are ~200 tiles wide; we render a **scrolling viewport**. `camera.rs`
  tracks Munchii (classic "push the screen when past the midline", clamped to
  level bounds), exposes the visible tile/pixel rect.
- The present path renders only the visible columns of the tile world + entities
  whose AABB intersects the viewport (culling). The four backends are unchanged
  in spirit — they still get a framebuffer (pixel tiers) or a cell grid +
  overlays (character tiers); we just feed them a camera-windowed scene.
- Parallax background layers (theme-colored) optional v1; flat themed sky first.
- All four fidelity tiers must render every tile kind and creature — same
  glyph-sprite-to-blocks pipeline we already use, extended to the registry.

## 7. The Munchii bestiary (original, non-violent reskins)

A coherent world: Munchii the beagle romps through yard → house → park → the
Bath House (dogs dread baths). Foes are critters and chores; he *pounces* and
they *pop into treats* — nobody gets hurt. Each maps a Mario archetype only as a
behavioral template; the design, art, and names are ours.

| Archetype (behavior template) | Munchii creature | Behavior | "Defeat" (non-violent) |
|---|---|---|---|
| Goomba (walks, turns at wall) | **Boneling** — a toddling chew-bone | walks forward, flips at walls | pounce → pops into a biscuit |
| Green shell (walks off ledges) | **Rollo** — a roly-poly pillbug | walks, falls off ledges | pounce → curls into a ball you can nudge |
| Red shell (stays on ledges) | **Rollo (sun)** | walks, won't step off | same curl-and-nudge |
| Paratroopa (hops/flies) | **Flutterbug** — a bouncy june-bug | bounces or flies a path | pounce → loses wings, becomes a Rollo |
| Piranha plant (in pipe) | **Dandi** — a snapping dandelion | rises/lowers from pipes | avoid; sneezes pollen, harmless if timed |
| Buzzy beetle (fireproof) | **Hardhat acorn** | like Boneling, bubble-proof | pounce → curls like Rollo |
| Spiny / Lakitu | **Burr** dropped by **Puffer** (a raincloud pup) | Puffer drifts above, drops Burrs | bonk Puffer with a Sudsball; Burrs are dodge-only |
| Hammer Bro | **Stick Squirrel** | tosses arcing sticks, hops | bonk with Sudsball → drops an acorn treat |
| Bullet Bill | **Zoomdisc** — a flung frisbee | flies straight from a launcher | pounce mid-air → pops |
| Cheep Cheep (water) | **Sudsfish** — a soap-bubble fish | swims/leaps in Bath levels | pounce/avoid → pops into bubbles |
| Bloober (squid) | **Moppet** — a wet-mop octopus | pulsing chase in water | avoid; bonk → flops away |
| Podoboo (lava bubble) | **Pop** — a marshmallow ember | leaps from the hot tub, arcs back | dodge-only |
| Firebar | **Sprinkler bar** — rotating water jets | spins; getting wet = a hit | dodge-only |
| Bowser (boss) | **Baron Whiskers** — a giant grumpy tomcat | paces the tub-ledge, swipes & lobs hairballs | pull the **bath plug** (the axe/bridge) → he drops in the bath, hilariously |

States/AI are simple deterministic FSMs (walk, turn, fall, curl/roll, hop, fly
path, rise/lower, throw-on-timer, boss pattern) — all tick-stepped so replay +
golden snapshots cover them.

## 8. Power-ups (original, dog-themed)

| Archetype | Munchii power-up | Effect |
|---|---|---|
| Coin | **Kibble** | collectible; 100 = extra life |
| Super Mushroom | **Big Kibble** | small → big Munchii (taller hitbox, takes a hit to shrink) |
| Fire Flower | **Bubble Bone** (chew toy) | lets big Munchii lob **Sudsballs** that bonk critters into treats (non-violent projectile) |
| Star | **Zoomies Treat** | brief invincible sprint — *the zoomies*; critters pop on contact |
| 1-Up | **Lucky Squeaky** | extra life |
| (block contents) | from **Question** blocks / **Coin-brick** | bonk from below to release Kibble / a power-up / nothing |

Power state machine on Munchii: `small → big → bubble`, dropping a tier when hit
(then brief invuln blink), mirroring the classic three-tier model but framed as
gear (collar/bandana) changes, not damage.

## 9. Reuse the determinism harness as the test net

Every milestone is guarded by the capture/replay golden snapshots we just built:
- Record a scripted playthrough of a representative level → bless mono-text
  keyframes → CI `--check` catches any physics/AI/render regression headlessly.
- Entity AI and the boss must be **tick-deterministic** (no wall-clock, no RNG —
  or a seeded RNG captured in the recording's existing `seed` field, which we
  reserved for exactly this).
- Add fixtures per theme (overworld, underground, water, castle/bath) so each
  tile kind + creature class has snapshot coverage.

## 10. Milestones (each independently shippable & snapshot-tested)

1. **Level IR + loader (no Godot yet).** Define IR + JSON; hand-author one tiny
   level; load it; render static tiles with the scrolling camera. Golden snaps.
2. **Compact Munchii + physics retune.** Small hitbox, camera follow, jump-tuning
   asserts against standard gaps/heights. Demo arena untouched.
3. **Tiles with behavior.** Typed tiles: ground/brick/question/pipe/platform/
   hazard; bonk-from-below, breakable bricks, coin pop. Snapshots per kind.
4. **Collectibles + power state.** Kibble, Big Kibble, power tiers, hit/shrink.
5. **Creatures I (ground).** Boneling, Rollo (+curl/roll), pounce resolution.
6. **Creatures II (air/water/throwers).** Flutterbug, Dandi, Puffer/Burr, Stick
   Squirrel, Sudsfish, Moppet, Zoomdisc; Bubble Bone / Sudsball projectiles.
7. **Hazards + boss.** Sprinkler bar, Pop, Baron Whiskers + bath-plug ending.
8. **The `.tscn` importer (offline).** Verify tile-byte format, TileSet mapping,
   entity translation table → IR. Per §2, imported Nintendo layouts stay local.
9. **Campaign flow.** World/level select, checkpoints, lives, transitions, BGM
   hooks. Author (or import, per §2) the level set; full play-through.

## 11. Open decisions (for you)
- **§2 IP path (A–D).** Gates what levels we commit/ship. Recommend A.
- Importer: **Rust parser** vs Godot dump script. Recommend Rust.
- Bestiary/power-up names & tone — the table above is a first pass; yours to veto.
- Theme arc (yard/house/park/bath house) — keep or re-theme?
- Scope of v1 campaign: one world (1-x) end-to-end first, or breadth across
  themes? Recommend **one world, all the way through**, then widen.

## 12. Red-team backlog (seed)
- ~~Confirm the Godot `tile_map_data` byte layout & TileSet decode.~~ **Done** —
  byte layout verified (see Decisions); validated over all 306 levels.
  ~~Scene inheritance~~ **resolved** (importer loads bases + overlays derived nodes).
  Still open: a round-trip test (import → IR → render → compare to a screenshot).
- Determinism of entity AI (tick-only, seeded RNG in the capture).
- Camera + variable hitbox interactions with the existing wall/ground probes.
- Performance: culling for ~200-tile levels across all four tiers (the kitty tier
  re-encodes an image each frame — keep the viewport image bounded like today).
- Big-Munchii in 1-tile gaps (depenetration), pipe-warp edge cases, off-screen
  entity sim (freeze vs. simulate), boss arena bounds.
- IP review of every committed level and every creature/name, repeatedly.

[RECORD_REPLAY.md]: ./RECORD_REPLAY.md
