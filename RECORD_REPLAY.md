# Milestone: Input Capture + Deterministic Replay

> Status: **planned / not started.** This is the next milestone after
> cross-backend dimensional parity (landed in `db1015b`). Pick up here on the
> next machine. Updated: 2026-06-22.

## Why

Lets us refactor and extend the engine — physics, sprites, effects, backends —
without silently regressing movement or rendering. A recorded playthrough plus
text-snapshot keyframes turns "does it still feel/look right?" into a diff that
CI (and the headless `verify` harness) can answer.

This pairs with two things already in the tree:
- `backend::mono_text` — renders a frame to plain text (the `scamp shot` path).
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
