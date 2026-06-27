# Plan: hand-drawn "level-4" graphics — engine pixel-art pipeline + animator art bible

## Context

`scamper` renders four fidelity tiers from one RGBA `Framebuffer`: Kitty pixels (4) →
Unicode half-blocks (3) → colored ASCII (2) → B&W mono (1). But there is **no real
pixel art** — every sprite is hand-authored ASCII-glyph art (`&'static [&'static str]`)
that the pixel tiers rasterize as cell-sized colored *blocks*. We want tier-4 to show
**hand-drawn, Disney-quality animation**, with the existing degrade to blocks/characters
staying flawless.

We are **not** drawing the art here. The headline deliverable is a giant **art-bible
spec** (`docs/ART_SPEC.md`, committed to the tree) the user sends to an animator,
enumerating every sprite/tile/effect, its animations, frame counts, timing, and art
direction. Alongside it we build the **engine pipeline** that will *consume* that art —
load per-frame PNGs, blit them at tier 4/3, and fall back to today's ASCII art when a
PNG is absent — so games keep working with zero art present and "light up" as frames
land. It must slot into **both** games (Super Munchii and Zoomies) through shared engine
code.

Decisions locked with the user:
- **Spec scope:** everything — ~37 character/item sprites (+ their anims), 11 tile kinds × 6 themes, ~15 effect clips.
- **Delivery format:** one RGBA PNG per frame, `assets/sprites/<id>/<anim>/NN.png`.
- **This pass:** spec doc **and** the engine pipeline (with ASCII fallback).
- **Both games** route through the shared engine path.

Key facts from exploration:
- The downsample is already flawless: lower tiers sample the same fb (`backend.rs` `sample()`, half-block ▀ = 2 px/cell; ascii/mono = luma ramp). Draw real pixels at tier 4 and tier 3 follows for free.
- `engine/src/png.rs` has an **encoder only** — we need a decoder.
- Sprites/anims are `'static` ASCII; `Sprite{ id, role, w, h, anims, palette }` (`sprite.rs`), `Anim{ name, fps, frames }`. Frame index = `clock / (NS_PER_SEC/fps) % n` on the deterministic tick clock (`sim.rs`).
- Both games build a sprite list then split on `backend.draws_overlay()`: pixel tiers call `draw_sprite_pixels` (cell-blocks), char tiers stamp glyph `Overlay`s. The cell grid is fixed (TILE=16, CELL_PX=4, CELL_PH=8) so tiles span whole cells in every tier (`backend_dimensional_parity` test).

---

## Part A — Engine pixel-art pipeline (shared; both games use it)

### A1. PNG decoder — `engine/src/png.rs`
Add `decode_rgba(bytes) -> Option<(usize, usize, Vec<u8>)>` (w, h, RGBA8). Support the
subset the animator will export: 8-bit RGB/RGBA, non-interlaced, zlib/DEFLATE IDAT, the
standard filters (None/Sub/Up/Average/Paeth). (We already DEFLATE-encode in `encode_rgba`;
add inflate + un-filter.) A test round-trips `encode_rgba` → `decode_rgba`.

### A2. Asset loader + cache — new `engine/src/pixels.rs` (`pub mod pixels`)
- One shared asset root: `$SCAMP_ASSETS` else `assets/` (run.sh runs from repo root). Munchii and shared creatures live once; both games read the same tree.
- `frames(id, anim) -> Option<&'static Frames>` — lazily load `<root>/sprites/<id>/<anim>/*.png` (sorted), decode each, **downscale once** to a memory cap (≈ 2× the on-screen footprint), cache in a `OnceLock<Mutex<HashMap<(String,String), Option<Arc<Frames>>>>>`. Absent dir → cached `None` (so the fallback path is hit with no repeated disk hits).
- `Frames { w, h, frames: Vec<Vec<u8>>, /* fps from a sidecar or a sensible default */ }`. Frame count is just how many PNGs exist — no hard-coded counts, so the animator can deliver any number per the spec.
- A parallel `tile(theme, kind) -> Option<&Frame>` for `<root>/tiles/<theme>/<kind>.png` (16-px, static), and the same mechanism serves effects (`effects/<name>/NN.png`) if present.

### A3. Framebuffer blit — `engine/src/framebuffer.rs`
`blit_rgba(&mut self, src_px, src_w, src_h, dst_x, dst_y, dst_w, dst_h)` — nearest-neighbor
scale the source frame into the destination rect, alpha-blending per pixel (reuse `blend`).
Scaling to the target rect (not 1:1) means size tiers / zoom in Super Munchii and any cell
size still work.

### A4. Render routing — shared helper, called by both games
Add `pixels::draw_sprite(fb, backend, id, anim, frame_i, dst_x, dst_y, dst_w, dst_h, glyph_lines, palette) -> ()`:
- If `backend.draws_overlay()` is **false** (Kitty/Text — the pixel tiers) **and** `pixels::frames(id, anim)` is `Some`: `fb.blit_rgba(frame, …)`. The half-block tier then samples those real pixels → flawless tier-4→3 transition.
- Otherwise fall back to the **current** behavior: `draw_sprite_pixels` (cell-blocks) for pixel tiers with no art, or the glyph `Overlay` for char tiers (2/1). The hand-authored ASCII art **stays** as the intentional low-fidelity representation — so the degrade ladder is: hand-drawn pixels (4) → half-block downsample (3) → ASCII glyphs (2/1).
- Frame index uses the existing tick-clock formula with the loaded frame count, so timing stays deterministic and per-anim.

**Slot-in for both games:** each game's draw loop currently owns a `Drawable = (lines, lx, ly, palette)` list and the `draws_overlay()` split.
- `games/supermunchii/src/lib.rs` `draw_play_frame` (the actor/munchii/projectile sprite list + the `if backend.draws_overlay()` branch).
- `games/zoomies/src/game.rs` `draw_frame` (its sprite list + same branch).
Extend each `Drawable` to also carry `(id, anim, frame_i)` for real sprites (markers/words pass `None` → always blocks), and route the pixel-tier branch through `pixels::draw_sprite`. Tiles route through a `pixels::tile` check inside `art::draw_tile`'s call sites (or a thin wrapper) so both games get pixel tiles. This is the only change to game code; the loading/blitting/caching all lives in the engine.

### A5. Flawless transition (the "level-4 → block/char" guarantee)
Nothing special per frame: tier 4 blits pixels into the fb; tier 3 samples them (half-blocks);
tiers 2/1 use glyph overlays. When fidelity changes (Zoomies hit, or Munchii's Tab) the fb is
built identically — only the backend's sampling changes — so the swap is seamless. The
existing `backend_dimensional_parity` invariant keeps everything cell-aligned; add a test that
a blitted sprite samples identically through Text vs Kitty at the shared cells.

---

## Part B — The animator spec: `docs/ART_SPEC.md` (the deliverable)

A single committed markdown "art bible". Structure:

1. **How the art is used** — the four tiers, that the animator draws **only the tier-4
   pixel frames** (engine auto-degrades to blocks at tier 3 and to the existing ASCII at
   2/1), RGBA with transparent background, the readability constraint (final on-screen
   size is small — design to read there; deliver a clean master we downscale).
2. **Delivery format & layout** — one PNG per frame at `assets/sprites/<id>/<anim>/NN.png`
   (zero-padded, in play order); tiles at `assets/tiles/<theme>/<kind>.png`; effects at
   `assets/effects/<name>/NN.png`. Exact per-asset pixel canvas (see below), transparency,
   naming, and a per-anim **fps + loop/once** table.
3. **Pixel canvas per asset** — the on-screen footprint, derived from the current cell
   size: `width_px = cells_w × 4`, `height_px = cells_h × 8` (e.g. Munchii 19×6 → **76×48
   px**; a 4×2 item → 16×16; tiles → 16×16). Give each the footprint **and** a recommended
   master size (draw larger, we downscale).
4. **Character roster** — every sprite from the inventory (Munchii + 24 creatures + 12
   items + projectiles). Per sprite: id, what it is, palette/style starting point (current
   RGB), and per animation: purpose, **recommended frame count for Disney quality**
   (e.g. idle 6–8, walk 8–12, jump 10–14 with anticipation/squash-stretch/follow-through
   called out), fps, loop type, and key-pose notes. Munchii's 7 anims (idle/walk/jump/
   crawl/wall-slide/happy/hurt) get the most direction since he stars in both games.
5. **Tiles** — the 11 `TileKind`s (Ground/Brick/CoinBrick/Question/Hidden/Pipe/Platform/
   Hazard/Deco/Spent/Crumble) at 16×16, seamless-tiling notes, animated where it sells it
   (Hazard liquid, Question shimmer, Crumble), times the **6 themes** (Overworld/Underground/
   Underwater/Castle/Snow/Rooftop) with each theme's palette as the starting point.
6. **Effects** — the ~15 clips (PUFF/DUST/SPARK/BANG/SHARDS/BOP/COIN/FEATHER/DASH/SPARKLE/
   CHEER/SNOW/LEAF/BUBBLE + word-pops): frame count, fps, tint, what each depicts.
7. **Per-game notes** — which assets each game uses (Super Munchii: full bestiary, items,
   5 themes, boss; Zoomies: Munchii as the rooftop runner, prickle/swooper hazards, steak
   pickup, Rooftop theme, the medical-cross/X markers), so the animator can prioritize.

The exhaustive inventory (names, sizes, current fps, frame counts) gathered in exploration
is the backbone; the spec adds canvas sizes, upgraded frame counts, and art direction.

---

## Asset layout (convention the spec + loader share)

```
assets/
  sprites/<id>/<anim>/00.png 01.png …      e.g. sprites/munchii/walk/00.png
  tiles/<theme>/<kind>.png                  e.g. tiles/rooftop/ground.png
  effects/<name>/NN.png                     e.g. effects/coin/00.png
```
Absent → engine falls back to ASCII/procedural art. `$SCAMP_ASSETS` overrides the root.

---

## Files

**New**
- `docs/ART_SPEC.md` — the art bible (the deliverable).
- `engine/src/pixels.rs` — asset loader/cache + `draw_sprite` / `tile` helpers; `pub mod pixels` in `engine/src/lib.rs`.

**Modified**
- `engine/src/png.rs` — add `decode_rgba` (inflate + un-filter).
- `engine/src/framebuffer.rs` — add `blit_rgba`.
- `games/supermunchii/src/lib.rs` — `draw_play_frame`: carry `(id, anim, frame_i)` on real-sprite drawables; route the pixel-tier branch + tiles through `pixels`.
- `games/zoomies/src/game.rs` — `draw_frame`: same routing for its sprites + tiles.

**Reused as-is:** `backend.rs` samplers (the downsample is already flawless), `sim.rs` tick clock (timing), the existing ASCII `Sprite`/`Anim` art (the fallback / low-fi tiers), `kitty.rs` `present_rgba`.

---

## Verification

1. **Build/tests green** with **no art present** — both games render exactly as today (fallback path), `cargo test` passes, `cargo run -p arcade` plays both games unchanged.
2. **png round-trip** unit test: `encode_rgba` → `decode_rgba` reproduces pixels; decode a couple of real PNG fixtures (8-bit RGB and RGBA).
3. **pixels loader** test: a temp `assets/sprites/x/y/00.png,01.png` loads 2 frames, sorted; a missing dir caches `None`; `blit_rgba` scales + alpha-composites into a `Framebuffer` (assert pixels).
4. **Flawless transition** test: blit a known frame, then assert the Text (half-block) sampler and the Kitty path agree at the shared cells (no shimmer / box artifacts), and that a transparent sprite leaves the scene behind it intact.
5. **End-to-end (manual)**: drop a test PNG into `assets/sprites/munchii/walk/` and confirm Munchii shows the hand-drawn frame at Kitty, a clean half-block downscale at Text, and the ASCII glyph at ascii/mono — in **both** games.
6. **Spec sanity**: `docs/ART_SPEC.md` enumerates every id in `sprite::ALL` + Munchii + all `TileKind`×`Theme` + every effect (cross-check counts against the inventory).

---

## Risks / red-team

- **Decoder scope creep.** A full PNG decoder is large. Mitigation: support only the export
  subset (8-bit, non-interlaced, the 5 filters) and fail-soft to fallback on anything else;
  validate against the animator's actual exporter early. (Alternative if it balloons: a tiny
  vendored `png`/`miniz` is off-limits per the no-external-deps style, so we hand-roll the
  inflate — bounded but real work.)
- **Memory/perf.** Downscale-on-load to a cap + cache per (id,anim); blit is nearest-neighbor.
  Bounded by the handful of on-screen sprites. Log what's loaded under `--debug`.
- **Two code paths persist.** Char tiers (2/1) stay on glyph overlays by design (that's the
  intended low-fi look + Zoomies' health-bar degrade), so we are *not* unifying everything to
  fb-sampling. The pixel path only augments tiers 4/3.
- **Footprint vs hand-drawn detail.** Final sizes are tiny (Munchii ~76×48); the spec must set
  expectations (design for readability at size, deliver a downscalable master). Flagged in §1/§3.
- **Spec is huge.** ~37 sprites × anims + 66 tile/theme combos + effects. Acceptable — it's the
  point — but organized by section with a per-game priority note so the animator can stage work.
