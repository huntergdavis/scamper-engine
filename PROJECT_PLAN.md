# Terminal 2D Platformer Engine — Project Plan (v3)

> Status: **planning / pre-code**. v3 narrows scope hard after a second red-team
> review (principal eng, Kitty-graphics, pro game dev w/ the real N++ sim, N++
> speedrunner, world-class Rust, Kitty power-user) and the user's decisions:
> single-player, keyboard-only, no audio, no network code (SSH is the transport).
> Updated: 2026-06-22.

## 1. North Star

A **single-player, N++-tight 2D platformer** that runs as an ordinary **local process**
doing stdin/stdout, rendering via the **Kitty graphics protocol** at a smooth 60fps, with
**keyboard-only** controls. The Kitty terminal is the target audience.

**The transport is SSH — we never write network code.** The game only ever reads stdin
and writes Kitty escapes to stdout. Running it over SSH into a remote Kitty terminal (or
sitting on the host already, in Kitty, running it locally) is *identical* from the game's
point of view; SSH carries the bytes. There is no socket code, no QUIC, no codec, no
streaming stack.

**Future (not now):** local "New Super Mario Bros"-style multiplayer where everyone on the
**same host** plays at once — implemented purely as **host-local IPC** (Unix socket +
optional `/dev/shm`/`/tmp` mmap ring), no root, no network. SSH just gets remote players a
shell on the host; once there, they're local.

First deliverable: a colored box with N++-tight movement on a real Kitty terminal — local
and over SSH. Art comes later. The engine is the product, **built game-first**: we ship a
tight local game, then extract generality from proven code, never gold-plate seams up front.

## 2. Locked decisions

| Topic | Decision |
|---|---|
| Players | **Single-player only** for now. Local same-host multiplayer is a future goal. |
| Input | **Keyboard only.** No gamepad (a pad on the client can't travel over SSH anyway; users map pad→keyboard externally). Kitty keyboard protocol for key down/up/held. |
| Audio | **None.** Cut entirely. |
| Networking | **None — ever, in our code.** SSH is the transport for "remote play"; host-local IPC for future multiplayer. |
| Rendering | Kitty graphics protocol; **direct base64** is the universal path (mandatory over SSH; shm impossible across machines). shm `t=s` is a *local-only* probed optimization. |
| Feel | **N++-class, faithful** to the real N++ physics model (see §4.6). |
| Determinism | **f64 + same-binary replay** (NOT fixed-point — N++ is an f64 game; fixed-point quantizes the feel and isn't needed). |
| Collision | **Faithful N++:** half-radius anti-tunnel sweep + up-to-32-iter closest-point depenetration vs oriented line + quarter-circle-arc segments on a 24px grid. |
| Lang/structure | Rust; **2 crates** (`engine` lib + `game` bin). Single-threaded game loop + one input thread. |

## 3. Platform reality (verified; re-check before relitigating)

- ✅ **SSH is transparent at the terminal layer.** Game = stdin/stdout; SSH tunnels Kitty
  escapes to the remote terminal. No networking in our code. **But over SSH the terminal
  is on another machine → `t=s` shm transfer is impossible → base64 is mandatory and
  bandwidth matters** (see §4.4 for the `z=`-layer mitigation).
- ✅ Kitty keyboard protocol (key release/held — required for variable jump) on Kitty,
  Ghostty, foot, WezTerm, Alacritty, iTerm2, rio. **Konsole lacks it** (legacy only).
- ✅ **Gamepad can't ride SSH** (it's not in the terminal stream) and needs `/dev/input`
  on the host — so keyboard-only is the *correct* fit for the SSH-first model, not a compromise.
- ✅ Dev box = Termux/Android, unrooted: `/dev/input` denied, `/dev/shm` absent, inside
  tmux (breaks Kitty graphics + keyboard). **Termux is the build + headless-test + PNG-dump
  box only**; play happens on a desktop Kitty (local or via SSH).
- ✅ Host-local IPC without root (future multiplayer): **Unix domain socket** (events),
  **mmap'd `/tmp` file** (tmpfs, most portable) or **POSIX `/dev/shm`** (shm_open, no root
  on desktop Linux) for a shared world/frame ring, `memfd_create`+fd-passing as the modern
  variant. All root-free, all local.

## 4. Engine design

### 4.1 Game loop — fixed-timestep + interpolation

```
const SIM_DT = 1/60 s;
let mut acc=0; let mut prev=Instant::now();
loop {
    let now=Instant::now(); acc += now-prev; prev=now;
    if acc > 8*SIM_DT { acc = 8*SIM_DT; }          // clamp: no spiral-of-death
    while acc >= SIM_DT {
        let p = feel_params.load();                // arc-swap load ONCE, top-of-tick (see 4.7)
        prev_state = current_state;
        sim_step(&mut current_state, &inputs_this_tick, &p);   // pure, no wall-clock reads
        acc -= SIM_DT;
    }
    render(lerp(prev_state, current_state, acc/SIM_DT));   // interpolate → smooth on any refresh
    sleep_until(next_deadline);
}
```
- **Sim = fixed 60Hz** (N++ is a 60Hz integer-frame game; all its windows are frame counts).
- **Render interpolation mandatory.** Phase 0 exit = **smooth on a 144Hz panel** (60Hz
  hides judder). Render state is distinct from sim state.
- Interpolation costs ~1 frame of display latency — budget it; offer a `--no-interp` flag
  for input-latency testing.

### 4.2 Timing

`CLOCK_MONOTONIC` (`Instant`). Sleep to an **absolute** deadline via
`clock_nanosleep(…, TIMER_ABSTIME)` (no drift; re-sleep on EINTR), spin the last ~1ms
(`spin_sleep`) for accuracy; margin configurable (0 on battery). `nice`/`SCHED_FIFO`/
`mlockall` are run-target-only, capability-guarded, watchdog'd; they no-op on Termux.

### 4.3 Determinism — f64, same-binary replay (NOT fixed-point)

Isolate sim math behind a `sim_math` module: `type Scalar = f64`, thin `sqrt/atan2/normalize`
wrappers via **`libm`** (not platform libm). **No `mul_add`/FMA** (CI lint + no fast-math),
**no wall-clock in sim**, single-threaded sim, fixed iteration caps (the 32-iter and 45-frame
caps are integers). Result: **bit-identical same-binary replay** — all the record/replay
tuning and CI invariants need. Cross-machine bit-identity is explicitly **out of scope**
(it would require a softfloat sim; not worth it, and N++ itself doesn't guarantee it). f64
gives far more sub-pixel precision than any fixed-point we'd pick, so we lose nothing.

### 4.4 Rendering — Kitty pipeline (corrections locked from prior review)

Per-frame command, assembled into one reused buffer, written in **one `write_all` to the
raw tty fd** (no `BufWriter` — it'd double-copy; no vmsplice):
```
\033_Ga=T,f=24,i=1,p=1,s=<Wpx>,v=<Hpx>,q=2,C=1,m=1;<base64>\033\\
\033_Gm=1;<chunk>\033\\        (chunks: multiple of 4, ≤4096 b64 bytes)
\033_Gm=0;<final>\033\\
```
- **`q=2` suppresses FAILURE responses only** (`q=1` = suppress OK). We never solicit OK and
  the input parser **defensively discards any `_G` APC replies** on stdin.
- **Pin `i=1, p=1`.** Same-id re-transmit is self-cleaning (terminal frees prior image+
  placement) → no leak, no per-frame `a=d`; fixed id+placement = in-place atomic replacement
  → this prevents flicker/z-fighting. `a=d,d=I,i=1` only on resize.
- Atomicity = write the *whole* frame in one burst (partial transmissions aren't shown). **No
  protocol double-buffer; no Kitty animation (`a=f`/`a=a`).**
- Transmit **RGB24** (drop alpha, −25% bytes). Size from `TIOCGWINSZ` (+ `CSI 14t`/`16t`
  fallback; handle 0 dims, common over SSH).
- **Bandwidth (matters because base64 rides the SSH pipe):** internal render resolution
  ~**960×540** (≈1.5MB raw/frame), integer-scaled by the terminal; pace to 60fps. **Big
  SSH win to adopt early: static background as a separate placement at `z=-1` transmitted
  ONCE, only the dynamic foreground (`f=32` RGBA) streamed each frame** — Kitty-native
  dirty-rect for free; turns a full-frame stream into a small-region stream. SSH's own
  compression helps further.
- `t=s` shm = probed, **local-only** optimization (impossible over SSH). Probe per-terminal
  via `a=q`, never by name. Keyboard probe must check the **release-events flag bit**, not
  just protocol presence (Ghostty vs Kitty differ).

### 4.5 Terminal lifecycle & hygiene (PROMOTED to first-class — was the biggest gap)

A single idempotent, reentrant `TerminalGuard`:
- **Setup (order):** save cursor → **alt-screen (`\033[?1049h`)** → hide cursor → raw mode
  (`rustix`) → push Kitty keyboard flags → focus reporting (`\033[?1004h`).
- **Teardown (reverse, run-once):** focus off → **pop keyboard flags** → **delete images
  (`\033_Ga=d,d=A\033\\`)** → leave alt-screen → show cursor → cooked mode.
- **Wire to EVERY exit path:** RAII `Drop` (normal quit); `std::panic::set_hook` that tears
  down *before* printing the backtrace (and `catch_unwind` around threads so a sim panic
  doesn't bypass cleanup); **SIGINT/SIGTERM/SIGHUP** (`signal-hook`) flip an atomic quit;
  **SIGTSTP/SIGCONT** (Ctrl-Z/`fg`) tear down on stop, re-setup + re-transmit on continue.
- **Resize (SIGWINCH):** re-`TIOCGWINSZ` (cells *and* pixels — font-size change keeps cells
  but changes pixels), realloc framebuffers (the one allowed realloc outside the hot path),
  `a=d,i=1` the stale image, recompute viewport, re-transmit. **Debounce** the SIGWINCH storm
  (~100ms quiescence). Policy: **fixed logical resolution, letterboxed/scaled** (consistent
  feel; resizing never changes how much level you see).
- **Min size:** if the terminal is too small, show a "resize to ≥ N×M" pause overlay.
- **Logging goes to a file** (`$XDG_STATE_HOME`), **never** stdout (corrupts the stream) or
  stderr (scribbles the alt-screen). `tracing` → file subscriber only while the TUI owns the screen.
- **Capability messaging:** on a terminal lacking the keyboard protocol, tell the user up
  front ("variable-height jumps need Kitty/Ghostty/foot") instead of feeling silently broken.

### 4.6 Physics & feel — faithful N++

Constants below are from the community-authoritative reverse-engineered sim
(`nsim.py` / SimonV42/nclone, treated as frame-accurate); **verify against source before
locking**. All are per-60Hz-frame, in pixels. Ninja `RADIUS = 10`.

- **Dual gravity (NOT apex-hang, NOT velocity-cut):** `GRAVITY_JUMP = 0.01111` applies while
  jump is held AND rising AND `jump_duration ≤ 45 frames`; otherwise `GRAVITY_FALL = 0.06667`
  (**exactly 6× stronger**). Variable height comes from *when gravity switches*, not from
  chopping velocity. Releasing jump or hitting 45 frames flips to fall gravity.
- **Horizontal: input-accel vs multiplicative drag (asymptotic top speed):**
  `GROUND_ACCEL = 0.06667`, `AIR_ACCEL = 0.04444` (air = 2/3 ground). Drag multipliers per
  frame: `DRAG_REGULAR = 0.99332`, `DRAG_SLOW = 0.86177`. Ground friction `0.94593`, wall
  friction `0.91134`. **`MAX_HOR_SPEED = 3.333` (200px/s) clamps INPUT ACCELERATION ONLY**
  (skip the accel if it would exceed max) — drag/slopes/launchpads may push total velocity
  *over* max and are NOT clamped. (This is what makes slope/launch speed tech work — never
  clamp total velocity.)
- **Slopes (central):** running/falling downhill *adds* a downhill-boost accel (≈
  `GROUND_ACCEL/2` projected along the slope); uphill uses N++'s bespoke `fric_force`
  projection formula (port it exactly). Landing on a slope **redirects** velocity along the
  tangent (preserves the tangential component), never zeroes it.
- **Jumps = impulses applied to velocity AND position same frame, with per-surface vectors:**
  flat floor `(0,-2)`; slope variants branch on uphill/downhill × input; wall: slide-jump
  `(2/3,-1)` vs regular `(1,-1.4)` scaled by wall normal. **Same-wall re-jump allowed** (wall
  climbing is core tech). No double-jump, no air-jump, no dash (`air_jumps = 0`).
- **Wall slide** = multiplicative damping of *downward* velocity while pressed into the wall
  (not a fixed reduced gravity).
- **Momentum preserved through ALL state transitions** (takeoff/landing never zero velocity;
  zero landing recovery — any squash is visual only).
- **Impact death:** `MAX_SURVIVABLE_IMPACT = 6`, scaled by surface normal (slopes survive
  faster landings). Core to how N++ plays. Plus crush death.
- **Launch pads:** boost `2·|normal| + 2`, special-cased to 1.7 for pure-vertical.
- **State machine = N++'s real states**, not Grounded/Airborne: `0 immobile, 1 running,
  2 ground-sliding, 3 jumping, 4 falling, 5 wall-sliding` (+ dead/etc). The run↔slide
  distinction gates which friction/accel branch fires — load-bearing for feel.
- **Forgiveness windows in FRAMES (small — N++ is crisp, not Celeste-forgiving):** jump
  buffer 5, wall buffer 5, floor/coyote 5, launchpad 4. (The circle collider already gives
  natural corner forgiveness — don't double-dip with Celeste-style ±px corner correction.)

### 4.7 Collision — faithful N++ (half-radius sweep + iterative depenetration)

Per-frame order: `integrate → collide_vs_tiles → post_collision → think`.
- **Anti-tunnel sweep with HALF radius (5px):** sweep the half-circle along the frame's
  displacement, advance to earliest intersection time. A cheap clamp, NOT the resolver.
- **Iterative depenetration, up to 32 passes:** each pass find the *single globally-closest*
  collidable point, push out along contact normal by `RADIUS − dist`, project velocity onto
  the surface **only if moving into it** (`dot < 0`); repeat to convergence or 32. **Accumulate
  floor/ceiling normals across passes, normalize once in `post_collision`** (stable resting,
  no jitter on concave floors).
- **Geometry:** tiles on a **24px grid** decompose into **oriented line segments** (one-sided
  → this is how internal edges / ghost-collisions are avoided; back-facing hits de-prioritized)
  **and quarter-circle arc segments (r=24)** for rounded tiles. Segment ends tested as circles
  (circle-vs-circle) plus the body. **A tile→segment decomposition table is its own spec.**
- **Determinism note:** this needs sqrt/normalize → f64 (§4.3), not fixed-point. The real sim
  has hardcoded epsilon nudges at exact corner geometry — budget a contact-epsilon policy and
  expect corner-case divergence as a known risk.

### 4.8 Concurrency — simplest correct (2–3 threads, no async)

- **Game thread:** drains input channel → fixed-step sim → interpolate → blit → **fused
  RGBA→RGB24→base64 in one zero-alloc pass into a reused buffer** → one `write_all`. All
  inline (encode is ~1–2ms at 540p; a separate encoder thread would only add a frame of
  latency for no local benefit).
- **Input thread:** blocking `read()` on stdin (Kitty keyboard bytes), parses to virtual
  controller, pushes events over `std::sync::mpsc`/`crossbeam-channel`.
- **No tokio, no flume bus, no encoder thread, no `Arc<Mutex<World>>`.** The sim owns the world.
- **Add complexity only on a trigger:** encoder thread when encode+write >8ms/frame; a bus
  when ≥2 live consumers (future multiplayer view-clients); IPC (Unix socket + shm ring) at
  the multiplayer phase. Until then, keep `render_encode_write` a concrete monomorphic fn —
  introduce a sink trait only when a second sink exists.

### 4.9 Live-tuning ↔ deterministic replay

`FeelParams` via **`arc-swap`**, loaded **once at top-of-tick** (never inside the integrator).
Hot-reload (`notify` file watcher) does NOT mutate the sim directly — it **enqueues a
`ParamChange` event onto the recorded input timeline** at the next tick boundary. A recording
= `seed + per-tick (input, optional ParamChange)`. **Two replay modes:** *faithful* (apply
recorded ParamChanges → reproduces the session bit-for-bit) and *counterfactual* (pin one
param set, replay the same controller inputs → the A/B tuning workhorse). CI invariants run
counterfactual against a committed param snapshot.

## 5. Workspace (2 crates)

```
crates/
  engine/   lib. modules: math, clock, physics, player, render (blitter + Kitty encoder +
            TerminalGuard), input (virtual controller, keyboard backend, record/replay),
            debug (overlays). PNG-dump = a #[cfg(feature="png-dump")] module (image optional).
  game/     bin. owns the loop, thread wiring, CLI, config; selects features.
```
Modules (not crates) do the layering while APIs churn during feel-tuning. Grow crates only
when a boundary stabilizes and earns its compile-firewall (e.g. split `engine-physics` once
its golden suite is heavy; transports/IPC when multiplayer is real).

## 6. Crates

Keep: `glam` (math; `scalar-math` for determinism), `image` (PNG-dump, feature-gated),
hand-rolled base64 (oracle: `base64` crate). Add: `rustix` (raw mode, `TIOCGWINSZ`, signals),
`libm` (deterministic transcendentals), `spin_sleep` (frame pacing), `arc-swap` (FeelParams),
`notify` (config watch), `crossbeam-channel` (input bus) or `std::mpsc`, `signal-hook`
(SIG handling), `directories`/`etcetera` (XDG paths), `tracing` + file subscriber (latency
logging), seeded PCG (`rand_pcg`) for any sim randomness. **Dropped vs v2:** tokio, quinn,
webrtc/str0m, postcard, bytes, gilrs, cpal, rtrb, parking_lot (no contended hot-path locks),
`hecs` (still deferred; keep components as small structs).

## 7. Observability

Frame-id stamped pipeline (sim→render→encode→present); per-stage timestamps via `tracing` to
file. **Glass-to-glass** = histogram of (present − input). **Measure terminal present time,
not just encode time** — over SSH and against the terminal's own vsync is where unaccounted
latency hides. Per-stage budget vs 16.6ms on a debug HUD.

## 8. Tuning & debug tooling (the backbone)

- **Deterministic input record/replay** at the input seam (Phase 1). Faithful + counterfactual.
- **Velocity vector + numeric vx/vy/speed readout + motion trail** — *Phase 1*, not Phase 3:
  momentum and slope speed-gain are invisible on a static box; you tune N++ by reading speed
  numbers against targets.
- **Collision circle + slope segments + contact normal + movement-STATE (0–5) display** —
  early Phase 2. N++ feel bugs are usually "wrong state / wrong friction branch."
- **Frame-step debugger** showing the 32-iteration depenetration (corner snags happen mid-loop).
- **Ghost compare** vs a prior recording — and, crucially, vs an **external N++ ground-truth
  trajectory** (import real N++ replay inputs); self-comparison can't tell you it's "N++-tight."
- **CI feel-invariants** as whole-trajectory L2 distance against golden traces (not scalar
  "max jump height" thresholds — that's a Celeste metric). Runs headless from Termux.

## 9. Phased build plan

- **Phase 0 — Skeleton + render + CLEAN LIFECYCLE.** Workspace (2 crates); fixed-step loop
  w/ interpolation; `clock_nanosleep` pacing; recycled framebuffer; corrected Kitty encoder
  (i=1/p=1, q=2, fused zero-alloc RGB+base64, one write_all); **`TerminalGuard` + panic hook
  + SIGINT/TERM/HUP/TSTP/CONT + SIGWINCH resize (debounced) + alt-screen + image cleanup +
  min-size + capability message + file logging**; PNG-dump sink; frame-id latency HUD.
  Encoder byte-golden test + a teardown-sequence test. *Exit:* box moves **smoothly on a
  144Hz Kitty, local AND over SSH**, and Ctrl-C / `panic!` / Ctrl-Z / resize all leave a
  clean terminal.
- **Phase 1 — Input + tuning visibility.** Virtual controller; **hand-rolled Kitty keyboard
  codec** (push/pop flags, release-event flag check, legacy fallback + capability probe);
  record/replay; **velocity vector + numeric readout + trail overlay**; XDG config + keybind
  file (format + location decided here). *Exit:* box driven by keyboard, inputs record/replay
  exactly, speed is readable on screen.
- **Phase 2 — N++ physics + collision.** Deterministic f64 sim (`sim_math` isolated); tile
  →segment decomposition (lines + arcs, 24px grid); half-radius sweep + 32-iter depenetration
  w/ normal accumulation; N++ states + drag/gravity/accel constants; impact death; launch
  pads. Collision/normal/state overlays; frame-step. **Test level built from N++'s real tile
  set** (45°, 1:2, 2:1 slopes, convex+concave arcs, inner/outer 90° corners, 1-tile gap &
  pillar at max speed, tall wall, wall-above-slope, high drop, launchpad, downhill→flat).
  *Exit:* faithful collision, identical replay, no tunneling/ghost-snags/jitter.
- **Phase 3 — GAME FEEL (the milestone, ship line).** Tune to N++ feel via counterfactual
  replay + ghost-compare vs real N++ traces; dual-gravity, drag, slope tech, wall-jump
  climbing, frame-tight buffers. A minimal shell: title → play → pause (auto-pause on focus
  loss) → quit. *Exit:* feels N++-tight against ground-truth; a stranger can launch, play, quit.
- **Phase 3.5 — Ship it.** `cargo install`; documented min Kitty/Ghostty/foot versions;
  README + run-over-SSH instructions; `--help/--version/--check/--record/--replay/--config`;
  config/replays/saves in XDG dirs. **This is the real v1 deliverable.**
- **Phase 4+ — Future (deferred).** Local same-host multiplayer (NSMB-style): authoritative
  sim + per-session view-clients over a **Unix socket** (+ optional `/dev/shm`/`/tmp` mmap
  ring), no root, no network. Then content (levels, art/sprites), `z=`-layer bg optimization
  if not already done, gamepad-on-host if ever wanted.

## 10. Top risks

1. **N++ fidelity** — §4.6/§4.7 are reverse-engineered; the exact-corner epsilon behavior
   even the experts approximate. Verify constants vs source; budget corner-case divergence;
   use external-N++ ground-truth ghosts to know if it's actually tight.
2. **Terminal lifecycle correctness** — panic/signal/resize/teardown is the thing TUIs get
   wrong; it's now a Phase-0 exit criterion with tests, not prose.
3. **SSH bandwidth** — base64 full-frame at 60fps is heavy on a network pipe; adopt the
   `z=`-layer static-bg trick and a modest internal resolution; measure over a real SSH link.
4. **Premature generality** — avoided by 2 crates + single-threaded loop + concrete fns;
   add seams only when a second consumer (multiplayer) exists.
5. **Phase-3 trust** — judder masked by 60Hz / tuning by inconsistent human input; mitigated
   by mandatory interpolation (144Hz exit) + replay-based tuning + ground-truth ghosts.

## 11. Resolved / out of scope

- Audio: **cut.** Gamepad: **cut for now** (keyboard-only; pad→keyboard is the user's job).
- Networking: **never in our code** — SSH is the transport; future multiplayer is host-local IPC.
- Determinism: **f64 same-binary replay**; cross-machine bit-identity out of scope.
- Collision/feel: **faithful N++** per §4.6–4.7.
- Streaming/QUIC/codec/PTS/transport-seam: **deleted** (were artifacts of a network model we don't have).
