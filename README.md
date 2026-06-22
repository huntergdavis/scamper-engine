# Scamper

```
     .-""-.            .-""-.
    /      \          /      \
   |        |   /\   |        |
   |        |  /o o\ |        |
   |        | ( -- ) |        |
   |        |  \__/  |        |
    \       |        |       /
     \      |        |      /
      '.    |        |    .'
        '-._|        |_.-'
            '--------'
         m u n c h i i
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

## Run it

```sh
./run.sh                   # build (release) + play
# or
cargo run --release
```

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

## Test

```sh
cargo test
```

See `PROJECT_PLAN.md` for the full design and rationale.
