# Scamper

```
             __
 )         (( o==@      m u n c h i i
 |         (\_)
 /____________\
 \____________/
  n  n      n  n
```

A local-first **2D platformer engine for the terminal**, with tight,
N++-flavored movement and a **swappable graphics stack** — from crisp Kitty
pixels all the way down to bare black-and-white ASCII. Single binary, single
dependency (`libc`); everything else (base64, PNG, the Kitty protocol, math,
collision, input parsing, every renderer) is hand-rolled.

## Meet Munchii

Our mascot is **Munchii** — a portmanteau of *munchy* and *ascii* — modeled on
**Detective Munch**, a very good beagle. Munchii is who you steer around the
arena: nose to the ground, ears flapping, sniffing out every ledge and wall.
Whatever renderer you pick, that's Munchii on screen — whether drawn in Kitty
pixels or sketched from `#` and `@`.

> Status: **playable movement prototype** with four live-switchable graphics
> backends. Run, jump, double-jump, wall-slide and wall-jump around a box arena
> that fills your terminal window.

## What it looks like

Munchii in the box arena, the bare **mono** (black-and-white ASCII) tier — the
floor of the render ladder. `Tab` in-game steps up through colored half-blocks,
colored ASCII, and full Kitty pixels.

```
....................................................
....................................................
....                                            ....
....                                            ....
....                                            ....
....                                            ....
....                          __                ....
....              )         (( o==@             ....
....              |         (\_)                ....
....              /____________\                ....
....              \____________/                ....
....               n  n      n  n               ....
....................................................
....................................................
```
<sub>Generated with <code>scamp shot</code> — the engine rendering itself to text.</sub>

## Run it

```sh
./install.sh               # one-time: ensure Rust + build (Linux or Termux)
./run.sh                   # build (release) + play
# or
cargo run --release
```

`./install.sh` ensures a Rust toolchain (`pkg install rust` on Termux, rustup on
Linux) and builds the release binaries; `--link` also symlinks `scamp` onto your
PATH. Both `install.sh` and `run.sh` are cross-platform (`#!/usr/bin/env bash`).

The **kitty** backend (the default) needs a terminal that speaks the Kitty
graphics + keyboard protocols — **Kitty**, **Ghostty**, or **foot**. But the
other three backends are pure text and run in **any** terminal. (Not under
tmux for the Kitty backend.) Works locally or over SSH.

`SCAMP_DEBUG=1 ./run.sh` uses a faster-compiling debug build for iteration.

### Controls

| Action | Keys |
|---|---|
| Move | `A`/`D` or `←`/`→` |
| Fast-fall | `S` / `↓` |
| Jump / double-jump / wall-jump | `Space`, `Z`, `K`, `W`, or `↑` |
| Switch graphics backend | `Tab` |
| Help menu | `h` |
| Quit | `Q` or `Esc` (asks Y/N); `Ctrl-C` force-quits |

The test app is a **box arena that fills the terminal window** (any aspect
ratio, rebuilt live as you resize), with the bottom row reserved for a status
line. Every movement function is reachable inside the box. Munchii's color shows
state: **yellow** grounded, **orange** airborne, **cyan** wall-sliding; the red
line is a debug velocity vector.

The terminal is fully restored on quit, `Ctrl-C`, crash, or `SIGTERM`/`SIGHUP`.

### Graphics backends (`Tab` cycles)

Munchii looks different in each — same engine, same framebuffer, different
renderer. The rendering layer is fully decoupled behind one trait.

| Backend | Look | Needs |
|---|---|---|
| `kitty` | full-color pixel image (Kitty graphics protocol, sharp) | Kitty/Ghostty/foot |
| `text`  | colored Unicode half-block cells (`▀`) | any terminal |
| `ascii` | colored ASCII glyphs — retro art | any terminal |
| `mono`  | plain black-&-white ASCII — bare minimum | any terminal |

## Headless verification (no Kitty terminal needed)

Runs scripted movement scenarios with numeric asserts and dumps PNGs of key
moments — used for development on machines without a Kitty terminal:

```sh
./run.sh verify ./scratch    # writes 01_spawn.png … 06_arena_wall.png
```

There's also a graphics probe — `./run.sh gfxtest` — that draws one static image
and reports your terminal's size + protocol support.

## Record &amp; deterministic replay

Capture a playthrough and replay it tick-for-tick — the sim is **same-binary
deterministic** (a fixed 60 Hz timestep with all timing driven off a tick clock,
not the wall clock), so a recording reproduces exactly. Used to catch movement /
rendering regressions textually, headless, in CI. See `RECORD_REPLAY.md`.

```sh
scamp record <name>          # play + capture per-tick inputs; q saves
scamp replay <name>          # visual replay (Tab cycles backends, q quits)
scamp replay <name> --bless  # write golden mono_text keyframes (headless)
scamp replay <name> --check  # replay + diff vs golden, exit 1 on drift (CI)
scamp captures               # list captures
```

`./run.sh -i` has a **record a run** entry and a **replay browser** (play /
check / bless / rename / delete). Captures live in `~/.local/state/scamper/
captures`. A committed fixture (`fixtures/captures/ci-smoke.*`) is replayed by
`cargo test` as a regression guard.

## Levels (importer + IR)

The campaign uses an engine-native, line-oriented level format (`*.lvl`) — readable
and hand-authorable (see `levels/yard-romp-1.lvl`). An **offline** dev tool imports
Godot `.tscn` tile levels into it:

```sh
scamp import <in.tscn> <out.lvl>   # decode a Godot scene to our Level IR
scamp level-info <file.lvl>        # stats + an ascii map of a level
```

The importer is for **local** use only: imported third-party layouts are
gitignored (`*.tscn`, `imported/`) and never shipped — we ship our own authored
levels. The full design (bestiary, power-ups, runtime, camera) lives in
`CAMPAIGN_PLAN.md`.

## Test

```sh
cargo test
```

See `PROJECT_PLAN.md` for the full design and rationale.
