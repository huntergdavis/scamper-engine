# Scamper

A local-first **2D platformer engine for the terminal**, rendering via the
**Kitty graphics protocol** at 60fps with tight, N++-flavored movement. Single
binary, single dependency (`libc`); everything else (base64, PNG, the Kitty
protocol, math, collision, input parsing) is hand-rolled.

See `PROJECT_PLAN.md` for the full design and rationale.

> Status: **v1 movement prototype.** A controllable character ("the Scamp") in a
> sandbox level with run, jump, double-jump, wall-slide, and wall-jump.

## Run it

You need a terminal that speaks the **Kitty graphics + keyboard protocols** —
**Kitty**, **Ghostty**, or **foot**. (Konsole renders graphics but lacks the
keyboard protocol, so variable-height jumps won't work there. It does *not* run
under tmux.) Works locally or over SSH into such a terminal.

```sh
./run.sh                   # build (release) + play, in a Kitty/Ghostty/foot window
# or
cargo run --release
```

`SCAMP_DEBUG=1 ./run.sh` uses a faster-compiling debug build for iteration.

### Controls

| Action | Keys |
|---|---|
| Move | `A`/`D` or `←`/`→` |
| Fast-fall | `S` / `↓` |
| Jump / double-jump / wall-jump | `Space`, `Z`, `K`, `W`, or `↑` |
| Quit | `Q`, `Esc`, or `Ctrl-C` |

The test app is a **box arena that fills the terminal window** (any aspect
ratio, rebuilt live as you resize), with the bottom row reserved for a status
line. Run along the floor, jump and double-jump, and slide / wall-jump off the
side walls — every movement function is reachable inside the box. Player color
shows state: **yellow** grounded, **orange** airborne, **cyan** wall-sliding;
the red line is a debug velocity vector.

The terminal is fully restored on quit, Ctrl-C, crash, or `SIGTERM`/`SIGHUP`.

## Headless verification (no Kitty terminal needed)

Runs scripted movement scenarios with numeric asserts and dumps PNGs of key
moments — used for development on machines without a Kitty terminal:

```sh
./run.sh verify ./scratch    # writes 01_spawn.png … 06_arena_wall.png
```

## Test

```sh
cargo test
```
