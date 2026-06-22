# Terminal 2D Platformer Engine — Project Plan (v2, post red-team)

> Status: **planning / pre-code**. v2 incorporates a 6-expert red-team review
> (principal eng, Linux kernel, Kitty-graphics, hardcore gamer, pro game dev,
> world-class Rust) and the user's strategic decisions.
> Updated: 2026-06-22.

## 1. North Star (revised)

A **local-first, deterministic 2D platformer engine** with **N++-class tight controls**,
rendered in the terminal via the **Kitty graphics protocol** at a smooth 60fps. The
engine has a **transport-agnostic seam** so the same game can *also* stream to remote
terminal clients as an **optional, explicitly "good-enough" mode** — never marketed as
frame-perfect.

**The core resolution of the central tension:** *tight feel is a LOCAL guarantee.*
The red team proved (with numbers) that codec/encode latency alone (2–4 frames) puts any
streamed thin client at ~80–130ms motion-to-photon — past the ~50ms precision-platformer
threshold. So **local play (kitty terminal + gamepad/keyboard) is where "N++-tight"
lives**; streaming is a secondary convenience that trades feel for reach.

First deliverable: a colored box with N++-tight movement on a real Kitty/Ghostty terminal.
Art comes later. The engine is the product, **but built game-first** (the local game pulls
the abstractions into the right shape; we do not gold-plate seams before the game is fun).

## 2. Locked decisions

| # | Decision | Choice |
|---|---|---|
| Strategy | tight vs streamed | **Local = tight guarantee; streaming = optional good-enough mode.** |
| D2/D3 | primary client + transport | **Terminal-native (Kitty graphics) primary.** Stock terminal carries graphics+keyboard only. Remote real-time path, if/when richer, uses **QUIC datagrams (timely-or-drop)** — never TCP/SSH for anything claiming to be responsive. |
| Feel | genre | **N++-class:** momentum/slippery, slopes & angled surfaces central, no dash. |
| D1 | latency/prediction | Thin-client/no-prediction for any streaming; **sim deterministic from Phase 2** so a future predicting (fat) client is a layer, not a rewrite. No WAN-tightness promises. |
| D4 | local terminal | **Kitty / Ghostty / foot** (full Kitty keyboard protocol → key-release events). Gamepad works on any terminal (bypasses it). |
| D5 | kitty keyboard | **Hand-roll the codec** (termwiz as reference); `rustix` for raw mode + `TIOCGWINSZ`. crossterm's event model is too shallow for clean release events. |
| D6 | server-auth for SP | **No.** Local-first, in-process, authority off. Streaming is a mode. |
| Lang | language | Rust, **Cargo workspace**, hand-rolled engine + focused crates. |
| Collision | model | **Faithful: swept circle-vs-tile-segment** (the real N++ model). |
| Runtime | default | **Local Kitty/Ghostty terminal is the default runtime.** Streaming is opt-in. |
| Streaming | reach | **LAN-only.** No WAN target (would only worsen feel for no gain). |

## 3. Verified platform reality (re-check before relitigating)

- ✅ Konsole: **no** Kitty keyboard protocol (KDE Bug 512065) → no key-release →
  **variable-jump-height physically impossible there**. Partial graphics (direct base64
  ok, shm unreliable). Use Kitty/Ghostty/foot for feel; gamepad-only on Konsole.
- ✅ Kitty keyboard protocol: Kitty, Ghostty, foot, WezTerm, Alacritty, iTerm2, rio.
- ✅ This dev box (Termux/Android, unrooted): `/dev/input` = **Permission denied** (DAC
  *and* SELinux MAC gates) → **no gamepad/evdev without root**. `/dev/shm` absent.
  Session is inside **tmux**. tmux breaks **both** Kitty graphics and the Kitty keyboard
  protocol → never the runtime; **Termux = dev/build/headless-test box only.**
- ✅ Desktop/server `/dev/input`: interactive desktop users get access via logind
  **uaccess ACLs**; a **daemon/SSH/service-account server has no seat** → needs the
  `input` group or a udev rule (`SUBSYSTEM=="input", MODE="0660", GROUP="input"`). gilrs
  needs **read+write** (rumble). **evdev is NOT focus-aware** — we must gate gamepad
  input on our own focus/pause state (optionally via terminal focus-reporting `CSI ?1004h`).
- ✅ shm `t=s` = POSIX **named** shm (`shm_open`, `/dev/shm` tmpfs), not memfd; cannot
  cross machines; absent on Android (Bionic uses ashmem). Opt-in local fast-path only.
- ✅ Kitty over SSH works (direct base64; no shm). TCP/SSH = head-of-line blocking → one
  lost segment stalls all later frame bytes → latency spikes. Fine for "lite" mode only.

## 4. Engine architecture

### 4.1 The game loop (fixed-timestep + interpolation — was missing in v1)

```
const SIM_DT = 1/60 s;                  // fixed, integer-friendly
let mut acc = 0; let mut prev = Instant::now();
loop {
    let now = Instant::now(); acc += now - prev; prev = now;
    if acc > 8*SIM_DT { acc = 8*SIM_DT; }        // clamp: no spiral-of-death
    while acc >= SIM_DT {
        prev_state = current_state;              // keep last state
        sim_step(SIM_DT);                        // deterministic, no wall-clock reads
        acc -= SIM_DT;
    }
    let alpha = acc / SIM_DT;
    render(lerp(prev_state, current_state, alpha));   // interpolate → smooth on any refresh
    sleep_until(next_deadline);                  // see 4.2
}
```
- **Sim = fixed 60Hz** (not 120/240 — higher costs determinism/re-tuning for no felt gain;
  swept collision handles tunneling). Substep only for pathological speeds.
- **Render interpolation is mandatory.** Phase 0 exit criterion = **smooth on a 144Hz
  panel** (60Hz hides judder). Requires `prev`/`current` sim state + interpolatable render
  transform distinct from sim transform.

### 4.2 Timing (was unspecified)

`CLOCK_MONOTONIC` always (`Instant`). Sleep to an **absolute** deadline via
`clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME)` (no drift accumulation; re-sleep on
EINTR), then **spin the last ~1ms** (`spin_loop()`) for sub-100µs accuracy (`spin_sleep`
crate). Margin configurable (0 on battery/Termux). Tail-latency tools on the **run target
only, capability-guarded, with a watchdog**: `nice -10`, then low-prio `SCHED_FIFO` +
`mlockall`. These **EPERM/no-op on Termux** — degrade gracefully. Once networking exists,
prefer `timerfd` in an epoll loop so tick/socket/gamepad fds are serviced together.

### 4.3 Determinism (Phase 2 requirement, not "someday")

Isolate all sim math behind a `sim_math` module. Default to **integer/fixed-point
sub-pixel** positions (good for N++ precision *and* the only robust cross-machine
determinism). If floats: `glam` `scalar-math` feature, route transcendentals through
`libm`, never `mul_add`/`+fma`, no fast-math. Same-binary replay is then free; cross-
machine determinism (for a future predicting client) is reachable. Enables record/replay.

### 4.4 Rendering — Kitty pipeline (corrected per Kitty expert)

Per-frame command (one tight write burst; home cursor first with `\033[H`):
```
\033_Ga=T,f=24,i=1,p=1,s=<Wpx>,v=<Hpx>,q=2,C=1,m=1;<base64 chunk>\033\\
\033_Gm=1;<chunk>\033\\        (middle; chunks = multiple of 4, ≤4096 b64 bytes)
\033_Gm=0;<final chunk>\033\\
```
- **`q=2` suppresses FAILURE responses** (NOT "all" — v1 was wrong; `q=1` suppresses OK).
  We never solicit OK, and the stdin parser **defensively discards any `_G` APC replies**
  (Konsole may reply regardless).
- **Pin `i=1, p=1`.** Same-id re-transmit is **self-cleaning** (terminal frees the prior
  image+placement) → no leak, no per-frame `a=d`. Fixed id+placement = in-place atomic
  replacement → this (not the delete policy) is what prevents **flicker/z-fighting**.
- `a=d,d=I,i=1` only on **resize/reconfigure**. `s=`/`v=` mandatory for `f=24`.
- **No protocol double-buffer exists**; atomicity = write the *whole* frame in one burst
  (partial transmissions aren't displayed). **Do NOT use Kitty animation (`a=f`/`a=a`)** —
  wrong tool for a live scrolling stream; do our own dirty-rect/compression later if needed.
- Transmit **RGB24** (drop alpha after compositing, –25% bytes). Output px size from
  `TIOCGWINSZ` (`ws_xpixel`/`ws_ypixel`) with fallback; handle 0 pixel-dims (common over SSH).
- `t=s` shm = capability-**probed** opt-in (fall back cleanly to base64); never platform-guessed.

### 4.5 Input — virtual controller

Canonical virtual controller (D-pad, A/B/X/Y, L1/R1, L2/R2 analog+threshold, L3/R3,
Left/Right sticks). Game reads `input.pressed(A)` / `input.axis_x()`. **Backends are an
`enum`** (closed set, polled together): keyboard, gamepad, mouse(stub), net. Bindings in
an **`arc-swap`** table (rebindable, hot-reloadable).
- Keyboard: hand-rolled Kitty keyboard codec — push enhancement flags (≥ disambiguate +
  **report-event-types** for releases) with **push/pop on the stack** (pop on exit/crash);
  probe support via `CSI ?u`; **legacy fallback** with explicit degradation logged.
- Gamepad: `gilrs` polled on the **input thread** (non-blocking `next_event`), hotplug
  (`Connected`/`Disconnected`), **own radial deadzone** in the virtual layer, loadable
  `gamecontrollerdb.txt`. Gate on focus/pause (evdev isn't focus-aware).
- **Input sampled at top-of-tick** and stamped; for streaming, stamp with frame-id so the
  server knows which frame an input was for (keeps prediction reachable).
- Prototype scope: D-pad + 2 face buttons + LeftStick wired; full surface designed, not built.

### 4.6 Physics & feel — N++-class

- **Collision (faithful, locked):** swept **circle-vs-tile-segment** continuous collision
  — the actual N++ model (momentum + slopes + curves). Sub-pixel fixed-point. Must handle:
  slopes/angled surfaces, **internal-edge merging** (no ghost-collision snags), one-way
  platforms (direction + prev-frame-feet aware), moving platforms (move solids → carry
  riders → resolve), corner correction with a **stated magnitude (±~4px)**, and correct
  **speed-clamp ordering** (clamp before sweep). Continuous, so no tunneling.
- **Player state machine:** just **Grounded / Airborne**. Everything else is timers/queries
  on the player (Celeste model), NOT states:
  `coyote_timer`, `jump_buffer_timer`, `var_jump_timer`/`var_jump_speed`,
  `wall_jump_lock_timer` (+ `force_move_x`), `air_jumps_remaining`, `apex_timer`.
  Wall-slide = query (`airborne && touching_wall && vel.y>0` → modify gravity).
- **Feel params (ms, rate-independent; all hot-reloadable):** coyote 80–100ms, jump buffer
  100–133ms, var-jump cut to 40–50% upward vel on release (≤~150–200ms after takeoff),
  apex hang ~50% gravity when |vel.y|<~30px/s for ~80–120ms, **fall gravity 1.5–2.0× rise**,
  wall-jump h-lock ~120ms + wall-slide max-fall well below terminal, **fast-fall** (hold
  down → higher max-fall), per-surface friction tags (N++ ice/normal), separate
  **turn-around decel** from friction, terminal velocity, **zero landing recovery**
  (any squash is visual only), **collision box ≠ render box** (forgiveness) decided now.

### 4.7 Concurrency (Rust — tokio is WRONG for the hot loop)

Dedicated OS threads; **tokio confined to the net edge**:
- **Sim+render on ONE thread** (no lock on the 60Hz-mutated world).
- **Encoder on its own thread** (RGBA→RGB→base64→chunk is pure CPU; mustn't stall sim).
- **Input thread** (keyboard/gilrs/net-in).
- **tokio runtime** only for quinn/QUIC when streaming exists; bridge via `flume` async.
- Communication: **channels** (`flume`/`crossbeam`), **not `Arc<Mutex<World>>`**. Sim owns
  the world; hands `Arc<FrameBuffer>` to encoder over a **bounded (cap 1–2)** channel so a
  slow sink applies backpressure / drops *visibly* (no silent caps). `FeelParams` via
  **`arc-swap`** (wait-free per-frame load; `notify` watcher swaps on file change).
- **Sinks = `Box<dyn FrameSink: Send>`**, one coarse `present(&FrameBuffer)` call/frame
  (vtable cost negligible). `AudioSink` consumer is the cpal RT callback → lock-free ring
  (`rtrb`), no alloc/lock in callback.
- **Transport seam is pull/credit + a control channel**, not bare push: the loop asks
  "render+encode frame N?"; sink signals backpressure, keyframe requests, resize, bandwidth.
  Frames+audio carry a shared **PTS/frame-id**. Validate the seam against a **hostile
  in-process sink** (injected latency/jitter/drop), not just "local sink unchanged."

### 4.8 Hot-path allocation (zero per-frame alloc)

Double-buffered recycled framebuffers. **Fuse** RGBA→strip-alpha→base64 into **one pass**
into a reused buffer (RGB24 is already 3-byte groups = base64 quanta; skip every 4th byte;
inject chunk escapes inline at 4096 b64 boundaries). Hand-rolled table-driven base64 (keep
`base64` crate as a test oracle). One `write_all` per frame to an owned `BufWriter` on the
fd. `image` only behind a feature for PNG-dump/sprites — never on the live path. SIMD later,
only after measuring.

## 5. Workspace layout (trait seam in a tokio-free core)

```
crates/
  engine-core/        traits (FrameSink/InputSource/AudioSink), FrameBuffer, InputState,
                      fixed-step clock, sim_math, glam re-export.  NO tokio/quinn.
  engine-physics/     deterministic integrator, swept circle-vs-segment collision
  engine-player/      Grounded/Airborne + timers, FeelParams
  engine-render/      blitter, Kitty encoder (fused RGB+b64), debug overlays
  engine-input/       virtual controller, binding table (arc-swap), backends (enum),
                      record/replay
  engine-audio/       mixer (deferred)
  transport-terminal/ LocalSink: Kitty over stdout
  transport-png/      PNG-dump sink            [feature: png-dump, dep: image]
  transport-quic/     quinn + tokio            [feature: quic]
  net-protocol/       serde + postcard wire types
  game-platformer/    THE binary; selects features
```
Features on the binary gate **whole crates**: `default=["terminal","gamepad"]`; optional
`quic`, `png-dump`, `shm`, `kitty-keyboard`. Keep async out of `engine-core` (CI guard).

## 6. Proposed crates

Keep: `glam`, `gilrs`, `image`(gated). Hot path: hand-rolled base64 (oracle: `base64`).
Keyboard: hand-roll + `rustix` (raw mode, `TIOCGWINSZ`). Add: **`flume`/`crossbeam-channel`**
(bus), **`arc-swap`** (FeelParams), **`spin_sleep`** (frame pacing), **`notify`** (config
watch), **`parking_lot`**, **`tracing`** (per-stage latency, honest logging), **`postcard`**
(input wire; over bincode), **`bytes`** (net fragments), **`rtrb`** (audio ring, deferred),
seeded PCG RNG. Networking (later): **`tokio`+`quinn`** (QUIC). `hecs` deferred (keep
components as small structs now so adoption is a storage swap). `str0m` only if a browser
client is ever added.

## 7. Observability (designed in from Phase 0)

Frame-id stamped pipeline (sim→render→encode→transmit→present); each stage logs a
timestamp via `tracing`. **Glass-to-glass latency** = histogram of (present − input) per
frame-id; per-stage budget vs the 16.6ms frame on a live HUD. You cannot tune what you
cannot measure — and glass-to-glass is the metric everyone gets wrong by measuring
socket-to-socket.

## 8. Tuning & debug tooling (the backbone — was missing)

- **Deterministic input record/replay** at the `InputSource` seam (Phase 2). Record the
  virtual-controller stream + seed; replay exactly → tweak a constant → replay *same
  inputs* → A/B the trajectory. This is what makes feel-tuning a science, not vibes.
- **Frame-step debugger** (pause, advance one sim tick, inspect state).
- **Debug overlays** (cheap — we own the framebuffer): swept path, collision shape, contact
  normals, tile grid, velocity vector, timer bars (coyote/buffer), input visualization +
  short history trail. **Ghost compare** a prior recording's path.
- **CI feel-invariants**: "max jump height == X±ε", "ledge-jump within coyote works", "no
  penetration across a battery of recorded high-speed inputs." Runs headless from Termux.

## 9. Phased build plan

- **Phase 0 — Skeleton + local render.** Workspace; fixed-step loop **with accumulator +
  interpolation**; `clock_nanosleep` ABSTIME + spin pacing; recycled RGBA framebuffer;
  corrected Kitty encoder (i=1/p=1, q=2, fused RGB+b64, chunked); terminal sink + PNG-dump
  sink; frame-id latency instrumentation. **Encoder byte-golden tests.** *Exit:* box moves
  **smoothly on a 144Hz Kitty/Ghostty terminal**; frames verifiable as PNGs in CI.
- **Phase 1 — Input.** Virtual controller (enum backends); hand-rolled Kitty keyboard codec
  + legacy fallback + capability probe; `gilrs` (own thread, hotplug, deadzone); analog
  axis; arc-swap bindings; **record/replay scaffolding**. *Exit:* box driven by keyboard AND
  gamepad via one code path; inputs recordable/replayable.
- **Phase 2 — Physics + collision (N++) + determinism.** Deterministic fixed-step sim
  (sub-pixel fixed-point, `sim_math` isolated); swept circle-vs-segment collision with
  slopes, one-ways, moving platforms, internal-edge merge, corner correction;
  Grounded/Airborne + timers. **Determinism + replay verified; physics golden tests; debug
  overlays; frame-step.** Test level with slopes/walls/gaps/ledges. *Exit:* box runs slopes
  & walls, no tunneling/ghost-collisions, identical replay.
- **Phase 3 — GAME FEEL (the milestone, LOCAL).** All feel params in ms via hot-reload;
  tuning HUD + input viz; dial in N++ momentum/slippery + wall-jump/slide + variable jump.
  **Validated against RECORDED inputs on a 144Hz Kitty terminal** (trustworthy because
  interpolated + replay-checked). *Exit:* feels N++-tight. **Project is shippable as a tight
  local game here**, independent of any streaming.
- **Phase 4 — Transport seam hardening.** FrameSink/InputSource/AudioSink as pull/credit +
  control channel + frame-id PTS. **Validate against a hostile in-process sink** (latency/
  jitter/drop). *Exit:* decoupling proven under adversarial timing, local sink unchanged.
- **Phase 5 — Streaming "lite" (good-enough, terminal-native, LAN-only).** Kitty graphics
  over SSH to a remote Kitty terminal on the LAN (graphics + keyboard). No WAN target.
  Measure glass-to-glass; document it as *not* frame-perfect. Optional QUIC-datagram
  transport for a leaner remote path. *Exit:* playable from a remote terminal on the LAN
  with measured, honestly-labeled latency.
- **Phase 6+ — Deferred (much later).** Audio (mixer + `cpal` + `rtrb` + jitter/clock-sync); richer
  clients (custom/native or browser+WebRTC) to carry sound+controller remotely; codec/
  compression + dirty-rect for bandwidth; a future fat/predicting client for WAN tightness
  (determinism already in place). Each behind its own feature/crate.

## 10. Top risks

1. **Conflating frame-rate with input-latency in streaming** — defused by strategy (local =
   tight, streaming = good-enough, never sold as frame-perfect). Keep the messaging honest.
2. **Phase 3 trustworthiness** — judder masked by 60Hz + feel tuned by inconsistent human
   input. Mitigated by mandatory interpolation (144Hz exit) + replay-based tuning.
3. **N++ collision complexity** — circle-vs-segment + slopes + moving/one-way platforms is
   the genuinely hard part; budget real time, lean on golden tests + overlays.
4. **Premature generality** — building the engine before the game. Mitigated by game-first
   framing: thin seams from day one, but no server/wire-protocol until the local game is fun.
5. **Determinism drift** (only if a predicting client is ever built) — fixed-point sim +
   isolated `sim_math` keep cross-machine determinism reachable.
6. **Termux-only blind spots** — graphical/gamepad paths can't run on the dev box; rely on
   PNG-dump + headless replay/golden tests as the primary CI signal; verify feel on desktop.

## 11. Resolved (was open)

- **Collision model:** faithful swept circle-vs-tile-segment. ✔
- **Streaming "lite" reach:** LAN-only; no WAN target. ✔
- **Default runtime:** local Kitty/Ghostty terminal; streaming is opt-in and deferred. ✔
- **Richer client** (remote sound+controller): Phase 6, much later. ✔
