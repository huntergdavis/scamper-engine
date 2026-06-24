# Milestone: Input Capture + Deterministic Replay

> Status: **landed (v1).** Tick-driven sim, capture/replay, golden keyframe
> snapshots, a committed CI fixture, and a `run.sh -i` browser are all in. The
> TUI file browser does play / check / bless / rename / delete. Updated:
> 2026-06-22.

## How to use it

```sh
supermunchii record <name>          # play + capture per-tick inputs; q saves the capture
supermunchii replay <name>          # visual replay (Tab cycles backends, q quits)
supermunchii replay <name> --bless  # headless: write golden mono_text keyframes
supermunchii replay <name> --check  # headless: replay + diff vs golden (exit 1 on drift)
supermunchii captures               # list captures (and which have golden snapshots)
```

`run.sh -i` exposes all of this: *record a run* (name-before-record prompt) and
*replay a run* (a browser over the captures dir — play / check / bless / rename /
delete). Captures live under `$XDG_STATE_HOME/scamper/captures` (`~/.local/state/
scamper/captures`); `SCAMP_CAPTURE_DIR` overrides it (used by the test fixtures).

## What landed vs. the plan below

- **Tick-driven sim** — `src/sim.rs` (`Sim`) advances exactly one `Player::step`
  per tick and derives **all** timing (effects, the wall-slide spark throttle,
  sprite-animation frame selection) from a tick clock (`tick * SIM_DT_NS`), never
  `now_ns()`. Wall-clock survives only for frame pacing, the FPS readout, and
  inter-tick render interpolation — none of which appear in a snapshot.
- **Capture format** — `src/capture.rs`: line-oriented text (`<name>.scap`), one
  `InputFrame` per tick (`axis jump_pressed jump_held down_held`) plus a header
  (`name`, `seed`, originating `WinSize`, `frames` count). Golden keyframes in
  `<name>.snap`. The arena is **frozen during recording** (resizes ignored) so a
  capture has a single geometry; replay rebuilds it from the stored `WinSize`.
- **Snapshots** — every 30 ticks (plus the final tick), `backend::mono_text`.
- **CI invariant** — `games/supermunchii/fixtures/captures/ci-smoke.{scap,snap}` is committed; the
  `committed_fixture_matches_golden` test replays it headless and asserts the
  keyframes match. Regenerate intentionally with
  `cargo test bless_fixtures -- --ignored`.

## Original design notes

## Why

Lets us refactor and extend the engine — physics, sprites, effects, backends —
without silently regressing movement or rendering. A recorded playthrough plus
text-snapshot keyframes turns "does it still feel/look right?" into a diff that
CI (and the headless `verify` harness) can answer.

This pairs with two things already in the tree:
- `backend::mono_text` — renders a frame to plain text (the `supermunchii shot` path).
  Reuse it for keyframe snapshots.
- the headless `verify` harness (scripted scenarios → PNG dumps to `./scratch`).
- the fixed-timestep f64 sim, which is intended to be **same-binary
  deterministic** (see `PROJECT_PLAN.md` §4.3).

## Determinism is the load-bearing requirement

Replay must be driven by **recorded sim ticks, not wall-clock**. Today several
systems read the clock directly (effect timing is wall-clock; the present loop
picks frames via `now_ns()`). For replay to reproduce a run, the recording must
capture per-tick inputs and the sim must advance by replayed ticks with no
wall-clock reads inside `sim_step`. Effects and frame selection during replay
derive from the tick counter, not `now_ns()`.

- A recording = `{ name, seed, arena dims, per-tick input frames }`.
- Faithful replay re-runs those exact inputs against the sim, tick for tick.
- Capture file format: start simple (a small binary or line-per-tick text
  format under an XDG-ish dir, e.g. `~/.local/state/scamper/captures/<name>`).

## Build criteria (user-specified — must satisfy)

**Capture UX**
- Name the capture **before** recording starts (prompt for a name, then record).
- `q` stops the capture *and* begins the normal exit sequence (same gated-quit
  path the game already uses).

**Replay UX — TUI file browser** (extends the `run.sh -i` arrow-key menu)
- Browse the captures directory and pick one to replay.
- **Rename** a capture from the browser.
- **Delete** a capture from the browser.

**Regression testing**
- During replay, take **text screenshots at keyframes** via `backend::mono_text`
  and diff them against committed expected snapshots — engine changes that alter
  behavior are caught textually, in CI, headless from Termux.

## Suggested sequence

1. **Tick-drive the sim for replay.** Add a replay clock source so `sim_step`
   advances by recorded ticks; route effect timing + frame selection through the
   tick counter when replaying (keep wall-clock for live play).
2. **Record.** Capture per-tick input frames + seed + arena dims to a file;
   name-before-record prompt; `q` finalizes and exits.
3. **Replay.** Feed a capture's tick stream back through the same loop.
4. **Keyframe snapshots.** At fixed tick intervals during replay, emit
   `mono_text` and (record mode) save / (test mode) diff against golden.
5. **TUI browser.** File browser in `run.sh -i`: list / play / rename / delete.
6. **CI invariant.** A committed capture + its golden snapshots, replayed
   headless, asserting the snapshots match.

## Done before this milestone (context)

- **Cross-backend dimensional parity** — Munchii and the walls are the same
  on-screen dimensions in every backend (kitty / text / ascii / mono), enforced
  by tests (`backend_dimensional_parity`, `cell_blocks_tile_without_gaps`).
  `build_arena` sizes the framebuffer and cell grid so a 16px tile spans a whole
  number of cells (4 wide × 2 tall). This matters for replay because hitboxing
  and the visual must agree across whichever backend a snapshot is taken in.

## Red-team backlog (latent, lower priority)

- `step_axis` can't depenetrate a pre-existing overlap — currently mitigated by
  clamping the hitbox to the arena interior in `munchii_box`.
- `top_glyph` equal-z tie-break is last-wins, so effects must use a non-zero z.
