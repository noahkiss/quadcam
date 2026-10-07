# Sim: design

Status: draft for review, 2026-10-07.
Baseline: `main` at release 0.6.4, with WP6 (switch map, radio joystick reader) on its branch.
Target: a built-in FPV simulator, built in work packages (section 11). No code before the owner
reviews this doc.

QuadCam knows the pilot's real gear: the radio, the quad's `diff all`, its blackbox logs and its
radio logs. The sim uses that knowledge. It flies the quad you own, on the rates the quad has,
with a flight model fitted from that quad's own logs, through the radio you fly with. It is
small-quad first (65-75 mm 1S whoops), and general enough for larger quads.

A throwaway spike (a patched copy of the MIT web sim `notfeylo/propwash` in a Tauri shell, with
a native HID bridge) proved the idea and taught most of what this doc specifies. The decision
that follows from it: QuadCam builds its own sim from a design, not a copy of propwash.
propwash stays a reference and a test bed. QuadCam has no association with it: no fork, no
upstream contributions, and no link or branding tie. Where QuadCam ports its MIT code, it keeps
propwash's copyright and permission notice (10.2).

This doc is the contract for the build. It names the crate, the threads, the engine choice and
how to measure it, the physics and flight-controller models, the quad profiles and how they are
fitted and validated, input and calibration, worlds, audio, the UI, the licensing of every
borrowed idea, the work packages and the releases.

---

## Contents

1. [Goals and non-goals](#1-goals-and-non-goals)
2. [Architecture](#2-architecture)
3. [Engine evaluation](#3-engine-evaluation)
4. [Physics model](#4-physics-model)
5. [Flight controller model](#5-flight-controller-model)
6. [Quad profiles](#6-quad-profiles)
7. [Input and calibration](#7-input-and-calibration)
8. [Worlds and scale](#8-worlds-and-scale)
9. [Audio and UI](#9-audio-and-ui)
10. [Licensing](#10-licensing)
11. [Build plan: work packages and releases](#11-build-plan-work-packages-and-releases)
12. [Open questions for the owner](#12-open-questions-for-the-owner)

---

## 1. Goals and non-goals

### 1.1 Goals

| Goal | Measure |
|---|---|
| Whoop-first, but general | A 65 mm and a 75 mm 1S whoop fly from fitted profiles. A 5-inch and a 7-inch preset fly from the same model with other parameters |
| Real feel | Each fitted profile passes the validation harness (6.4): hover throttle, punch, roll response and sag within stated bands of the quad's own logs |
| Your quad, your rates | Rates, throttle curve, arm, angle and turtle switches come from the quad's `diff all` and the radio's model. No retyping |
| Smooth 120 Hz on Apple Silicon | On a 120 Hz ProMotion display, frame pacing meets the bar in 3.3. The physics never slows down when a frame is late |
| Radio only | The sim reads the radio natively over USB HID. No keyboard flying, no gamepad path |
| Quick start | Plug the radio in, pick the quad, fly. FPV view by default. No battery-connect step |
| Deterministic and testable | The same input stream gives bit-identical flights. The physics runs headless in CI |

### 1.2 Non-goals

- Multiplayer, leaderboards, ghosts and a track editor. Out of scope until after 1.0.
- Gamepads and keyboard flying. A sim for learning on a gamepad exists elsewhere.
- Running real Betaflight. The sim models a Betaflight-style controller (section 5). Real
  Betaflight is GPL. It could run later only as a separate downloaded process (10.2).
- Photoreal graphics before steady frame pacing. Pacing comes first (3.3).
- A gravity slider as a fix for "floaty". Floaty comes from wrong mass, drag, thrust, camera or
  scale. The design fixes those instead (4.11, 8.1).
- Damage models. A whoop survives almost every crash. The sim models bounces and turtle, not
  broken props.

### 1.3 Rules this design keeps

| Rule | Where it comes from | What it means for the sim |
|---|---|---|
| Look at GPL code; never copy it | `AGENTS.md`, Rules | WebFPV, Betaflight, SimITL and `blackbox_decode` are idea-only. Formulas come from public docs and physics (section 10) |
| One `api` method table | `AGENTS.md`, Rules | Sim features are `Core` methods first, then rows in `api`, then CLI and MCP (2.6) |
| Keep the MCP surface small | `AGENTS.md`, MCP server | Sim actions join `quadcam_gear` and `quadcam_gear_edit`. No new tool |
| Publishable repo | `AGENTS.md` | Profiles, calibrations and rooms built from a user's home are user data in the gear folder. Test fixtures are scrubbed log excerpts |
| Never touch real devices in tests | `AGENTS.md`, Rules | The sim uses `radio_hid`'s `NoHid` and `FakeHid` under cargo. No real HID, audio or window in CI |
| Never take focus while testing | `AGENTS.md`, Rules | Prototypes and harnesses run headless or offscreen. The owner flies by hand |
| Native Mac look | Owner preference | The sim's settings, calibration and launch screens are QuadCam's own React UI, with native patterns |

---

## 2. Architecture

### 2.1 Overview

```text
QuadCam app                         Sim host (2.4)
-----------                         --------------
React UI: Sim page, launch,         HID thread
  calibration, settings               -> input ring (timestamped reports)
Core: profiles, calibrations,       physics thread (quadcam-sim, 2 kHz)
  settings, gear store, backups       input -> link model -> FC -> motors -> aero
        ^                             -> rigid body -> collisions
        |  control channel            -> snapshot (triple buffer)
        +-------------------------> audio thread   <- snapshots
           (settings, status)       render loop    <- snapshots, interpolated
```

- **`quadcam-sim`** is a new Rust crate in the workspace (`src-tauri/sim/`). It holds the physics,
  the flight controller, the rate math, the input model, the profile format and the fitting. It
  has no Tauri, no window and no audio dependency. It builds and tests on its own.
- A **sim host** runs the crate's threads and draws. The host depends on the engine (2.4). The
  app's `Core` owns everything stored: profiles, calibrations, settings, worlds installed.
- Data that changes at 2 kHz never crosses a process or an IPC boundary on its way to the
  renderer.

### 2.2 The physics thread

| Property | Design |
|---|---|
| Step | Fixed, 2 kHz default (`dt` = 0.5 ms). 1, 2 or 4 kHz selectable for tests. S1 runs a convergence test (4.12) and may lower the default to 1 kHz |
| FC rate | The FC runs every physics step, so at 2 kHz by default. Real whoop FCs run their PID loop at 2-8 kHz |
| Integrator | Semi-implicit Euler for the rigid body and motor speeds. Quaternion renormalised each step. RK4 only if the convergence test shows Euler drifts at 2 kHz |
| Thread | A dedicated thread at the highest user QoS (`QOS_CLASS_USER_INTERACTIVE`). It sleeps until the next step deadline with `mach_wait_until`, then runs every step that is due |
| Real time, never slow-motion | Sim time follows the wall clock. When the thread falls behind by more than 25 ms (a stall: a GC pause in a WebView host, the system busy), it drops the backlog: sim time jumps forward to now, the state does not advance through the gap, and the drop is logged with its size. Wall time is never stretched |
| Determinism | No wall-clock reads inside a step. One seeded RNG per subsystem (prop wash, sensor noise, link model), drawn every step whether used or not, so a replay never depends on the path. `f64` throughout; no fused multiply-add differences across targets, since the only target is arm64 macOS |
| Budget | Measured in the spike's JavaScript: about 50 µs per 1 kHz step, the same in contact as in free flight. Rust should be well under that. The bar: p99 step cost under 100 µs at 2 kHz, in contact and in free flight (S1 bench) |

**Why this fixes the spike's complaints.** propwash ran its physics inside the render loop:
up to 40 steps per frame, then slow motion; sticks read once per frame, so every step in a frame
saw the same stick value. Physics felt tied to frame rate, because it was. Here the physics
owns its clock, and the renderer only reads.

### 2.3 Input path and render handoff

- **Input.** The HID thread reads every report the radio sends and stamps it with
  `mach_absolute_time` on arrival. It pushes `(t, axes, buttons)` into a lock-free ring. The
  physics thread takes, for each step, the newest sample with `t` at or before the step's time
  (sample and hold, as a real receiver does at its packet rate). The link model (7.6) sits
  between the ring and the FC, outside the deterministic core: the core consumes the post-link
  stream, and a recording stores that stream, so replays stay exact.
- **WP6's watch throttles frames to one per 16 ms** for the controls page. The sim needs every
  report: S4 adds an unthrottled, timestamped subscriber to `radio_hid` beside the throttled
  one. The report parsing stays one function.
- **Render handoff.** After each step the physics thread writes a snapshot (pose, velocities,
  motor speeds, battery, FC state, OSD values, contact events) into a triple buffer with its
  sim time. The render loop reads the two newest snapshots and interpolates the pose to
  `present_time − one step`. It never blocks the physics thread, and the physics thread never
  waits for a frame.
- **Audio** reads the same snapshots at its own buffer rate (9.1).

### 2.4 Window and process model

Two host shapes, one crate:

| Host | When | Shape |
|---|---|---|
| In-process | WebView renderer (phase 1, and the WebKit candidate) | The physics, HID and audio threads run in the QuadCam process. The sim opens a second `WebviewWindow` on its own URI scheme and CSP. A Tauri `Channel` sends the newest snapshot at each frame request. The page interpolates |
| Sim process | A native engine (Bevy, Godot) | A second executable in the bundle (`QuadCam.app/Contents/MacOS/quadcam-sim`), signed and notarized with the app. QuadCam launches it with the profile, calibration and settings, and talks to it over a local socket (JSON lines, as `control.sock`). The sim process opens the radio itself: `radio_hid` opens the device shared (`macos-shared-device`), so QuadCam's controls page keeps working |

Why a separate process for a native engine:

- Bevy and Godot each want to own the main thread and its event loop. So does Tauri. Two event
  loops on the macOS main thread do not share well.
- A panic or GPU hang in the renderer does not take the library or an import down with it.
- The sim's binary size and dependencies stay out of the main executable's link.

The cost: one more signed executable, a launch step of about a second, and settings sent over a
socket. Settings change rarely, so the socket carries little.

The window: full screen on the display QuadCam's window is on, or windowed (a setting). Esc
opens the sim's menu. Quitting the sim returns focus to QuadCam's Sim page.

### 2.5 How sim data reaches the UI

| Data | Path | Rate |
|---|---|---|
| OSD, stick display, toasts in flight | Drawn by the renderer from the snapshot | Every frame |
| Calibration screen sticks | QuadCam's own UI, through WP6's throttled radio stream | 60 Hz |
| Sim status (running, armed, flight time, dropped steps) | Host → `Core` → event `sim-status` | 2 Hz |
| Session summary (flights, crashes, turtle count, max speed) | Host → `Core` at quit | Once |
| Settings changed in the sim's menu | Host → `Core::settings_set` → file | On change |
| Settings changed in QuadCam | `Core` → host over the control channel | On change |

The sim writes nothing on its own. `Core` owns `settings.json` and the gear folder, as today.

### 2.6 Surfaces: `api` rows, CLI, MCP

| Row | Params → result | Writes |
|---|---|---|
| `gear_sim_status` | – → `SimStatus` (host running, profile, radio, armed, dropped steps) | no |
| `gear_sim_profiles` | – → `Vec<SimProfileSummary>` | no |
| `gear_sim_profile` | `{ aircraft }` → `SimProfile` with each value's source | no |
| `gear_sim_fit` | `{ aircraft, logs }` → `FitReport` (values, residuals, what is still an estimate) | the profile, after confirm |
| `gear_sim_validate` | `{ aircraft }` → `ValidationReport` (each check, band, result) | no |
| `gear_sim_calibration` | `{ radio }` → `Calibration` | no |
| `gear_sim_calibration_save` | `Calibration` → `()` | the calibration |
| `gear_sim_launch` | `{ aircraft, world }` → `()` | no (GUI only, see below) |

- CLI: `quadcam-cli gear sim status | profiles | profile <aircraft> | fit <aircraft> [--log ID]
  | validate <aircraft> | calibration <radio>`.
- MCP: `quadcam_gear` gains `sim_status`, `sim_profiles`, `sim_validate`; `quadcam_gear_edit`
  gains `sim_fit` and `sim_calibration_save`.
- `gear_sim_launch` is a GUI command only. It opens a full-screen window, so an agent never
  starts it.

---

## 3. Engine evaluation

### 3.1 Candidates

| | WebKit (WKWebView) | Chromium (CEF or Electron) | Bevy | Godot |
|---|---|---|---|---|
| What | The web view Tauri already uses, three.js on WebGPU | A bundled Chromium | Rust game engine on `wgpu` (Metal) | Game engine and editor, Metal renderer |
| License | System framework | BSD-3 (Chromium), plus a large notices list | MIT or Apache-2.0 | MIT. `godot-rust` (gdext) is MPL-2.0 |
| Embeds in Tauri as | A second `WebviewWindow`, in process | Not natively: Tauri's CEF runtime is experimental at the time of writing; Electron is a separate app | A separate process (2.4). In-process only by giving up Bevy's windowing and drawing `wgpu` into a Tauri window's handle | A separate process. The physics crate loads as a GDExtension |
| Frame pacing and 120 Hz | The spike measured 60 fps rAF in WKWebView on a 120 Hz panel. Safari caps page rendering near 60 fps by default; the switch is a WebKit preference, not public API | Chromium renders at the display's rate on ProMotion | Native `CAMetalLayer`; 120 Hz depends on the present mode and frame-rate hints the windowing layer sets | Native Metal renderer since Godot 4.4; V-Sync on Metal |
| Input latency | Snapshots cross the IPC per frame; GC pauses in JavaScriptCore stall frames, not physics | As WebKit, plus a second process | Physics and render share a process; no IPC per frame | As Bevy, through GDExtension |
| Build time and disk | No new toolchain. Seconds | Large downloads; CEF builds are minutes and gigabytes | Clean build several minutes; target directories of several GB | Godot export templates are prebuilt; the gdext crate builds like any crate |
| App size added | About 1-3 MB of scripts and assets | 150-250 MB | Roughly 20-50 MB | Roughly 60-100 MB for the export template |
| Graphics tooling | three.js, no editor | Same as WebKit | Code-first; scene editor immature | Full editor, lighting, terrain and import tooling |
| Fit | Lowest cost; 120 Hz unproven | Pays the most to fix only pacing | Shares crates and language with QuadCam | Best tools for building worlds |

The table's size and pacing figures are expectations, not measurements. Section 3.3 measures
them.

### 3.2 Prototype per engine

Each prototype is the same small program, so the numbers compare:

- One closed room at true scale (5 × 4 × 2.5 m), about 50k triangles, one shadowed light, a
  baked environment map, and one whoop model.
- Fed by the `quadcam-sim` crate (S1). A recorded 120 s flight replays through the physics
  thread at 2 kHz, so every engine draws the same motion. A live-radio mode is there for the
  owner to fly.
- FPV camera, 4:3 and 16:9, the stick display and three OSD text elements.
- Instrumented: each frame's present time (Metal `presentedTime` where the engine exposes it,
  the rAF timestamp in a web view), the newest input sample time in that frame, the physics
  thread's step lateness, CPU time per thread and GPU time.
- The WebKit prototype is phase 1's minimal renderer (S5). The Chromium prototype reuses that
  page in Electron, to measure Chromium without first solving its embedding.

### 3.3 Measurements and the bar

Run on an Apple Silicon Mac with a 120 Hz ProMotion display, full screen, at the display's
native resolution. Headless runs collect the timing; the owner flies once in each for feel and
films one latency test.

| Measure | How | Bar |
|---|---|---|
| Frame pacing at 120 Hz | 120 s replay, frame-to-frame present interval | p99 ≤ 9.2 ms; at most 1 missed vsync per 200 frames; no gap over 25 ms |
| Frame pacing at 60 Hz | Same, display set to 60 Hz | p99 ≤ 17.5 ms; same miss and gap rules |
| Physics real time | Dropped-step count over a 10 min replay while rendering | Zero drops after the first second |
| Input-to-present, proxy | Present time minus the newest input sample's arrival time | p50 ≤ 20 ms, p95 ≤ 30 ms |
| Input-to-photon, filmed | The owner films stick and screen at 240 fps, 20 stick flicks per engine | Reported, compared across engines; no fixed bar |
| Cost | CPU % per thread and GPU % at 120 Hz | Reported; the sim must leave headroom for a screen recorder |
| Cold start | Launch to first frame | ≤ 3 s |
| Build | Clean build time, incremental rebuild after touching one sim file, target directory size, CI minutes added | Incremental ≤ 30 s; CI ≤ +10 min |
| Size | Zipped `.app` before and after | Reported |
| Glue | Lines of host code beyond the shared crate | Reported |

**Decision rule.**

1. An engine that misses the 120 Hz pacing bar or the latency bar is out.
2. Among those left, prefer the lowest build, size and maintenance cost, unless the worlds work
   (S8) needs tooling only one of them has.
3. The owner decides from the report.

### 3.4 Recommendation, conditional

**Bevy in a separate sim process**, if its prototype meets the bar.

- It shares the language and crates with QuadCam: `quadcam-sim`, `radio_hid` and the profile
  types link in directly.
- MIT or Apache-2.0, native Metal through `wgpu`, small next to Godot or Chromium.
- Graphics quality comes mostly from assets and lighting, not from the engine (section 8).

Fallbacks, in order:

- **WebKit stays** if its prototype meets the 120 Hz bar. That depends on the 60 fps page-render
  cap, which only the measurement settles. It costs nothing new.
- **Godot** if Bevy misses the bar, or if building worlds in code becomes the bottleneck.
- **Chromium** only if it alone meets the bar. It adds the most size for the least gain.

Bevy's compile times and target directories are its known cost. S6 records them, and the
build plan keeps Bevy out of the main crate's build (a separate binary and feature).

---

## 4. Physics model

Every term below says where its model comes from and how it may be used:

- **reuse code**: MIT, BSD or Apache source; port or copy with its notice (section 10).
- **idea only**: GPL or no-license source; read to learn, write our own.
- **fact**: published physics or a paper's equation; re-derive freely.

Units are SI. Body frame: x forward, y left, z up.

### 4.1 Rigid body

- Six degrees of freedom: position, velocity, attitude quaternion, body rates.
- Mass is the **all-up mass with the pack**. A whoop's pack is a third of its weight.
- Inertia is a diagonal tensor. It is estimated from the frame's geometry and mass (a box for
  the body and pack, point masses for the motors), then scaled until the roll response matches
  the log (6.3).
- Source: fact (Newton-Euler). propwash's rigid-body step is a reference (reuse code).

### 4.2 Motors

**Electrical model**, not a fixed lag:

- Motor voltage = command × loaded pack voltage.
- Current = (motor voltage − ω / Kv) / R_winding.
- Motor torque = (current − no-load current) / Kv.
- Rotor: J_rotor · dω/dt = motor torque − prop torque (kQ · ω²).
- The time constant falls out of the model: it is short for small steps and long for a full
  punch, as the logs show (16 ms small steps, 35-40 ms full punch, on a 75 mm whoop).
- Kv comes from the spec. R_winding, no-load current and rotor inertia are estimates, fitted so
  the rise time and punch current match the log.
- A check from the logs: motor speed ≈ idle + (0.655 · Kv · V − idle) · command, fitted on two
  whoops within about 1.4k rpm RMS. The fitted model must reproduce it.
- Bidirectional DShot idle (`dshot_idle_value`) sets the idle command.
- Source: the DC-motor equations are fact. WebFPV uses this structure (idea only). propwash's
  first-order lag with separate up and down time constants is the fallback for a profile with no
  fit (reuse code).

### 4.3 Props: thrust and torque

- **Static thrust**: T = kT · ω². kT comes from hover: at the logged hover speed, thrust equals
  weight. The accelerometer gives thrust over mass directly, so this needs no scale.
- **Torque tied to thrust**: from momentum theory, ideal power is T^1.5 / √(2ρA), and real power
  is that over a figure of merit (FM). So kQ follows from kT and FM. Small props at low Reynolds
  numbers have a low FM (about 0.3; a 5-inch about 0.5). This is why whoop yaw is weak. A second
  estimate of kQ comes from the hover current; the fit uses both.
- **Inflow and advance ratio**: thrust falls as axial inflow rises. With J = V_axial / (n · D),
  thrust coefficient CT(J) = CT0 · (1 − J / J0). J0 comes from the prop pitch (zero thrust near
  1.1-1.2 × pitch speed for real props).
- **Descent (negative inflow)** raises thrust, except in the vortex ring state (4.6). This is
  the spike's defect: full throttle did too little while falling. The fix is the right descent
  model and the right airframe, validated on fall-recovery flights (6.2).
- Source: momentum theory and the advance-ratio form are fact. propwash's inflow factor is a
  reference (reuse code). WebFPV's FM-based kQ and its whoop FM value are idea only; the
  equation itself is textbook.

### 4.4 Ducts

- **Thrust**: the duct's effect on static thrust folds into kT, because kT is fitted from hover.
- **Rotor momentum drag**: air entering a spinning rotor sideways leaves axially, which drags.
  F = −c_duct · ṁ · v_inplane, with ṁ = ρ · A · v_induced. A ducted whoop has a much higher c
  than an open prop. This is a large part of how a whoop slows down when it stops pitching.
- **H-force (rotor drag) for open props**: linear in ω and in-plane velocity.
- Later, not v1: duct lip lift in forward flight and its pitch-up moment.
- Source: rotor momentum drag and H-force are fact. RotorPy (MIT) implements H-force (reuse
  code). WebFPV's duct coefficients are idea only; the sim fits its own.

### 4.5 Body drag

- Quadratic per body axis: F = −½ ρ · Cd·A_axis · |v| · v, with plan and front areas.
- Angular damping scaled by inertia. The spike found a fixed coefficient tuned for a 7-inch held
  a whoop's roll at 60 % of the setpoint.
- Cd·A starts from geometry and is fitted to coast-downs and terminal speed in a flat fall
  (6.2). A whoop falling flat tops out near 10 m/s.
- Source: fact. propwash's attitude-blended drag is a reference (reuse code).

### 4.6 Ground effect, prop wash and vortex ring state

- **Ground effect**: T_IGE / T_OGE = 1 / (1 − (R / 4z)²) (Cheeseman and Bennett), clamped
  near the ground. On for every airframe; it matters indoors.
- **Prop wash**: severity rises when the quad descends into its own wake, and fades with
  horizontal speed. It costs thrust and adds band-limited noise torque per rotor (10-40 Hz).
  One amplitude, default low, exposed as a single slider. Too much reads as fake.
- **Vortex ring state**: in a near-vertical descent at about 0.5 to 1.5 times the hover induced
  velocity, thrust drops and turns rough. A whoop's hover induced velocity is about 6 m/s
  (√(T / 2ρA) with the 75 mm and 65 mm examples' disc loading), so the band is roughly 3-9 m/s:
  speeds a whoop reaches in a drop. Horizontal speed takes the rotor out of the band.
- Source: Cheeseman-Bennett and the VRS empirical induced-velocity curve (Leishman) are fact.
  propwash's prop-wash severity and noise model (reuse code). WebFPV's single prop-wash
  amplitude (idea only).

### 4.7 Collisions and crashes

- **Engine**: Rapier (Apache-2.0) with its deterministic build, as a dependency. Static world
  colliders (boxes and triangle meshes) plus one dynamic body.
- **Quad collider**: four duct rings and the body box for a whoop; the body box and prop-disc
  sensors for an open-prop quad (prop strikes cost thrust on that motor for a moment).
- **Bounces**: restitution and friction per material (wall, floor, carpet, grass, gate). Whoops
  bounce off walls and keep flying; this has to feel right indoors.
- **No damage**. A crash is "stuck": upside down, or not moving with motors at idle. The fix is
  turtle mode (5.5) or a reset on a radio switch.
- **Cost**: collider lists stream within a radius of the quad. The S1 bench asserts contact
  costs no more than twice a free-flight step. The spike saw lag near the ground but could not
  reproduce it in physics; the per-frame instrumentation in 3.2 catches it if it returns.
- Source: Rapier (dependency). propwash's collider streaming and prop-strike sensors (reuse
  code).

### 4.8 Battery

- **Off by default**: an ideal pack. Constant voltage, no depletion. Sag rarely adds to a sim
  session, and it changes the feel in a way many pilots do not want while practising.
- **Sag, an option**: open-circuit voltage from state of charge (a 1S LiPo or LiHV curve), minus
  total current × pack resistance. The fitted pack-plus-wiring resistance per quad (6.1) feeds
  it. Loaded voltage sets the motors' top speed, so sag changes the flight, not only the OSD.
  A 1S punch sags about 20-25 %, enough to stop a heavy whoop arresting a steep dive.
- **Capacity** (mAh used, low-voltage warnings) only with sag on.
- Source: the equivalent-circuit model is fact. propwash's `Battery.ts` (reuse code). WebFPV's
  cell-resistance argument (idea only).

### 4.9 Wind and air

- Indoors: still air.
- Outdoors: mean wind plus gusts (a Dryden-style Gauss-Markov model per axis), off by default.
- Air density: sea level by default; altitude as an option.
- Source: Dryden turbulence (MIL-F-8785C) is fact. propwash's Dryden-lite (reuse code).

### 4.10 Sensors

- The FC sees an ideal gyro by default. Gyro noise and motor vibration are an option for the
  analog-look and realism settings, scaled by motor speed.
- Source: fact. propwash's gyro and accel noise model (reuse code).

### 4.11 What makes a sim feel floaty

The community's top complaint (12+ mentions in the research). The design answers each named
cause:

| Cause | Answer |
|---|---|
| Wrong mass, thrust or drag | Fitted per quad (section 6) |
| Maps too big | True-scale worlds (8.1) |
| Camera uptilt or FOV off | From the quad's profile (8.2) |
| Low or uneven frame rate | 120 Hz and the pacing bar (3.3) |
| No air resistance in a fast fall | Body drag and rotor momentum drag (4.4, 4.5) |

### 4.12 Physics tests in the crate

- Step convergence: the same flight at 1, 2 and 4 kHz agrees within stated bands (position after
  10 s, peak rate on a flip).
- Determinism: two runs of a 60 s input recording give the same state hash.
- Energy: an unpowered body in still air loses energy, never gains it.
- Unit checks per term: hover at the fitted command, ground effect at h = R, VRS thrust loss in
  its band and not outside it.

---

## 5. Flight controller model

The sim runs its own Betaflight-style controller. Each part is written from Betaflight's public
documentation and from logged behaviour, never from its source (section 10).

### 5.1 Rates

- **Models**: Betaflight, Actual and Quick, per axis, with the quad's own numbers. Raceflight and
  KISS as presets for pilots who use them elsewhere.
- **Source of numbers**: the active rate profile in the quad's latest `diff all` (WP4's backup
  store, parsed by WP2's `dump` parser: `rates_type`, `roll_rc_rate`, `roll_srate`,
  `roll_expo` and the pitch and yaw equivalents, `rateprofile` selection).
- **Throttle curve**: `thr_mid` and `thr_expo`, and `throttle_limit_type` and
  `throttle_limit_percent`.
- **One implementation**: the rate math lives in `quadcam-sim::rates` (the sim crate cannot
  depend on the app crate). WP8's `gear/rates.rs` re-exports it for the Rates segment and the
  sim adapters. Whichever package lands first writes it; the acceptance tests (curves match
  reference values for each model) are shared.
- Source: the rate formulas are published math (fact). propwash's `rates.ts` (reuse code).

### 5.2 RC smoothing and feedforward

- RC smoothing: a PT3 low-pass on setpoint, cutoff from the link's packet rate, as Betaflight's
  automatic mode does.
- Feedforward from the setpoint's rate of change, with averaging and jitter reduction. This is
  why link jitter (7.6) matters for feel.
- Source: Betaflight's docs describe both (idea only). propwash's implementation (reuse code).

### 5.3 PID approach

- The sim's plant is a model, not the real quad. Real PIDs on a model plant do not reproduce the
  real quad's response. Pilots give the same advice for every sim: copy rates, not PIDs.
- So the sim runs a Betaflight-style PID (P, I with I-term relax, D on gyro with a low-pass,
  feedforward) with **its own gains, fitted per profile** so the closed-loop step response
  matches the log: setpoint-to-gyro lag and overshoot per axis (13.7 ms roll lag on the 75 mm
  example; the spike needed twice propwash's feedforward to get there).
- The quad's PIDs from the `diff all` are stored in the profile for reference and shown in the
  profile view. They do not drive the sim.
- An "Ideal" option skips the PID: body rates follow setpoint through a fitted first-order lag.
  It is the baseline the validation harness compares against.

### 5.4 Angle and horizon

- Angle: setpoint is a target attitude limited by `angle_limit`, driven by a level gain.
- Horizon: blends angle and acro by stick deflection, per Betaflight's documented behaviour.
- Mode switches come from the quad's `aux` lines (WP6's `switchmap::aux_modes`): ANGLE and
  HORIZON on their ranges, as on the real quad.

### 5.5 Airmode, turtle and arming

| Feature | Design |
|---|---|
| Airmode | On when the `diff all` enables it (feature or AIRMODE aux). The mixer keeps differential authority at zero throttle by raising all motors. Off: the mixer clips at idle |
| Turtle | The FLIP OVER AFTER CRASH aux mode from the quad's `aux` lines, on its switch. Armed with it on, the motors spin in reverse, only the pair the stick points at, with `crashflip_motor_percent` and `crashflip_expo` from the `diff all`. Reversed props make less thrust (an estimated factor, about half) |
| Arming | The ARM aux range on its switch, throttle below `min_check`, attitude within `small_angle` of level (except in turtle), and the arm switch seen off since the sim started (Betaflight's arm-at-boot guard). A refusal shows its reason on the OSD |
| No battery step | The pack is connected when the sim starts. ESC start-up tones play once |
| Disarm | The arm switch, or the link lost past failsafe (7.6) |
| Reset | A radio switch the user picks in the sim's settings (sim only), or the menu. Reset puts the quad at the world's start pad |
| Crash detection | Off by default, as on most whoops |

Source: Betaflight's documented behaviour for each mode and guard (idea only). propwash's turtle
implementation and the spike's arming fix (a per-airframe spawn tilt limit) (reuse code).

---

## 6. Quad profiles

### 6.1 The parameter set

One profile per aircraft (the aircraft profile QuadCam already has). Each value records its
**source**: `fitted` (from logs, with the fit's log ids and residual), `spec` (manufacturer),
`diff` (from the quad's `diff all`), `estimate` (geometry or a typical value) or `default`.

| Group | Values | Usual source |
|---|---|---|
| Frame | All-up mass, pack mass, wheelbase, inertia (3), body Cd·A (3), duct or open, collider sizes | spec + estimate; inertia and Cd·A fitted |
| Motors | Kv, pole count, R_winding, no-load current, rotor inertia, idle command | spec (Kv, poles), `diff` (idle), fitted (rest) |
| Props | Diameter, blades, pitch, kT, FM (or kQ), J0, reverse thrust factor | spec, fitted (kT, kQ), estimate (J0, reverse) |
| Ducts | c_duct | fitted, else estimate |
| Battery | Cells, chemistry, capacity, pack + wiring resistance | spec, fitted (resistance) |
| FC | Sim PID gains, feedforward, loop rate, RC smoothing | fitted |
| From the `diff all` | Rates and type, throttle curve and limit, airmode, angle limit, small angle, crashflip, aux mode ranges, PIDs (reference) | `diff` |
| Camera | Uptilt, FOV, aspect | `diff` where present, else the user's entry (the O4 and analog cameras differ) |

Stored as `<gear>/sim/profiles/<aircraft>.json`, written only through `Core`. Built-in presets
(a 65 mm and a 75 mm whoop, a 5-inch, a 7-inch) ship as data in the crate, with every value
marked `spec`, `estimate` or `default`. A user's fitted profile is user data and never ships.

### 6.2 Where the values come from

| Source | Rate | What it gives |
|---|---|---|
| Blackbox (the FC's flash, pulled on connect by task BB, section 11) | 800 Hz to kHz | Hover command and motor speed, motor speed map, kT / mass, motor rise times, pack resistance from punch windows, current at hover and punch, setpoint-to-gyro lag per axis |
| Radio logs (EdgeTX CSV, already read by WP12) | 10 Hz | Hover throttle, sag against current, flight time, mAh per flight. Too slow for dynamics; a cross-check |
| `diff all` (WP4 backups, WP2 parser) | – | Everything marked `diff` in 6.1 |
| The aircraft profile | – | The user's mass and pack weight; links to the FC and the radio |

What blackbox cannot give, and what fills it:

- **Mass**: weigh the quad with a pack. The fit does not need mass for kT / mass, but the
  collision response does.
- **Drag and fall recovery**: the logs so far hold gentle flights. Fitting drag and the descent
  model needs a short "fit flight": hover, punch, coast to a stop, a flat drop and punch out,
  roll and yaw flicks. Section 12 asks whether to publish one in the user guide.
- **Blackbox dates**: FC logs carry no real date (no RTC). Logs are dated by matching blackbox
  stick traces to the radio's logs, the same shape matching as `logmatch`.

Decoding is QuadCam's own reader, written from the documented blackbox format.
`blackbox_decode` is GPL: look, never copy, and never bundle it. Until QuadCam's reader exists,
fixtures come from CSV that `blackbox_decode`, run as a separate program, produced. Its output is
data, not code.

### 6.3 Fitting

`quadcam-sim::fit` does the fit, in Rust, deterministic, run by `gear_sim_fit`:

1. Pick windows: steady hover, punches (all motors > 90 %), small motor steps, roll and pitch
   steps, coasts and drops.
2. Fit in order, each step holding the earlier values: hover (kT, motor speed map) → motor
   electrical (rise times, punch current) → pack resistance → kQ (from hover current and FM,
   reconciled) → inertia (roll and pitch response) → drag (coasts and drops, when present) →
   sim PID gains (closed-loop lag).
3. Report each value, its residual, its windows, and the values still estimates. The user
   confirms before the profile changes.

The spike's Python fit (two whoops, about 500 s of logs each) is the reference for step 1-3. The
Rust fit must reproduce its numbers on the same windows.

Example fits from two stock 1S whoops, from the spike (the example profiles start here):

| | 75 mm (1102 21000 KV, 45 mm tri) | 65 mm (0702 25000 KV, 31 mm tri) |
|---|---|---|
| Hover command | 0.347 | 0.338 |
| Hover speed | 20.9k rpm | 27.3k rpm |
| Static T/W at 4.2 V / 3.6 V | 7.6 / 5.6 | 6.4 / 4.7 |
| Motor rise (small step) | 16 ms | 24 ms |
| Pack + wiring resistance | 0.026 Ω | 0.049 Ω |
| Hover current | 6.3 A | 2.4 A |
| Roll setpoint-to-gyro lag | 13.7 ms | 19.3 ms |

### 6.4 Validation harness

The harness replays logged sticks and motor commands through the crate and compares the result
with the log. It runs in CI on fixtures and on demand on a user's own logs (`gear_sim_validate`).

| Check | Mode | Band |
|---|---|---|
| Hover | Closed loop, logged sticks | Command within ±0.02; speed within ±5 % |
| Punch, open loop | Logged motor commands, body held | Peak vertical accel ±15 %; speed rise to 90 % within ±15 ms; peak current ±10 % |
| Sag (sag on) | Logged motor commands | Minimum voltage ±0.1 V |
| Roll and pitch steps | Closed loop | Lag ±3 ms; overshoot ±10 points |
| Yaw steps | Closed loop | Lag ±5 ms |
| Coast-down | Closed loop | Deceleration ±15 % (when the log has one) |
| Fall recovery | Closed loop | Height lost ±20 % (when the log has one) |

Known gaps from the spike's replay, which the bands must close: peak thrust on a punch 3.0 g in
the sim against 3.7 g logged; speed reaching 90 % in 70 ms against 85-93 ms logged.

CI fixtures:

- Short excerpts (a few seconds each) of decoded logs, as compact CSV in
  `src-tauri/sim/tests/fixtures/`, under 1 MB in total.
- Scrubbed: craft and pilot names, board UIDs and dates removed. A test fails on a UID-shaped
  value, as `fixtures_are_scrubbed` does for Betaflight dumps.
- The headless bench (not CI): step cost, contact cost and a 10 min real-time run.

---

## 7. Input and calibration

Everything here was tried in the spike with a real radio. The owner judged the calibration the
best they had used in any sim; this design keeps its behaviour and fixes what it got wrong.

### 7.1 Reading the radio

- `radio_hid` (WP6): an EdgeTX radio in USB Joystick mode, `1209:4f54`, 19-byte reports, 8 axes
  of 0..2048 and 24 buttons. Read natively with hidapi; the web view's Gamepad API never sees it.
- **The axes are the radio's mixer outputs.** The radio's expo, weights, limits and switches
  are already applied, exactly as a receiver would see them. Axis i is CH(i+1).
- The sim reads every report with its arrival time (2.3).

### 7.2 Mapping

- For an EdgeTX radio whose model QuadCam knows (the aircraft profile's `edgetx_model`, WP3),
  `switchmap::StickChannel` already says which channel each stick drives, and the quad's `aux`
  lines say which channel arms and which turns on turtle and angle. The sim pre-fills the map
  from them.
- For any other radio, or with no model known, the auto-calibration (7.3) finds the map.
- The user can always change the channel per function.

### 7.3 Guided auto-calibration

The screen is QuadCam's own UI, reusing WP6's `Sticks` widget:

1. Show both sticks live as gimbal boxes, Mode 2 by default, with a mode picker (1-4). Each axis
   is labelled with its name and what it does: throttle (up and down, climb), yaw (left and
   right, turns the nose), pitch (up and down, tilts forward and back), roll (left and right,
   banks). The live channel values show beside them.
2. **Move the sticks**: the user moves both sticks around their full travel. The screen finds
   which axis moved for each function, and records each side's extreme. It advances when every
   stick axis has covered at least 80 % **of each side** (the spike first counted the full range,
   so a pitch swept only 60 % back still passed; per-side coverage fixed it). A Done button
   accepts 50 % per side.
3. **Let go**: centres come from 0.6 s of steady roll, pitch and yaw. Throttle needs none.
4. **Flip the arm switch**: any non-stick axis or button that changes is the arm channel (unless
   the quad's `aux` lines already named it).
5. **Review**: the live sticks with the adjusted output, and per axis: channel, Reverse, the two
   ends, deadzone. Save, or Recalibrate.

For an EdgeTX radio with a known model (7.2), steps 2-4 shrink to "move each stick once to
check", since the channel values are already exact (open question 3).

### 7.4 Per-axis tuning

- **Deadzone** in the centre, for roll, pitch and yaw. It is cut from the middle and the rest is
  rescaled, so full travel still reaches ±100 % and there is no jump at its edge.
- **Two endpoints per axis**, low and high set separately, for a worn or uneven gimbal. Each is
  named by its direction (Left end, Right end; Back end, Forward end; Bottom, Top), and the
  names follow Reverse.
- **Reverse** per axis.
- Auto-calibration fills the endpoints. An edited endpoint overrides it (marked as edited) until
  Recalibrate. Recalibrate keeps deadzones.
- The stick widget shows the adjusted output live, with the percentage beside each axis.

### 7.5 Saving: keyed by radio identity

- **Not by USB serial.** Every EdgeTX radio of one MCU type reports the same generic serial
  (`UsbInfo.serial` already says it is not an id), so two radios of one model would share one
  calibration.
- **Key: the Gear radio's device id** (the card volume UUID hash, gear-design 4.2). In joystick
  mode the card is not mounted, so the sim resolves which radio it is:
  1. Exactly one saved radio has this USB product name: that radio.
  2. Several do: ask once, "Which radio is this?", listing them. Remember the answer as the
     default for that product name.
  3. None do (a radio never seen in storage mode): a provisional key from the product name and
     VID:PID, which the user can link to a saved radio later.
- Stored in `<gear>/sim/calibrations.json`: per radio, the mode, the map, each axis' ends,
  centre, deadzone and Reverse, and the product, VID:PID and firmware version it was made with.
- The calibration screen opens on its own for a radio with no calibration. Otherwise it opens
  from the sim's settings.

### 7.6 Link model

- Off by default: each report reaches the FC as it arrives.
- On: the user picks an ELRS packet rate (50, 150, 250, 500 Hz). The model resamples the stream
  to that rate, adds timing jitter and optional packet loss, seeded.
- Lost packets past the failsafe time disarm the quad (Betaflight's failsafe stage 2), so a
  failsafe drill is possible.
- Why: feedforward and RC smoothing both follow packet timing. A perfectly regular stream feels
  sharper than any real link.
- Source: WebFPV's link model (idea only). ELRS packet rates are published (fact).

### 7.7 What the spike dropped, and stays dropped

- No keyboard flying. Keys are only shortcuts (menu, reset, camera).
- No gamepad path.
- No battery-connect step.

---

## 8. Worlds and scale

### 8.1 True scale first

Oversized maps are the community's most named cause of floatiness: a whoop in a room built for a
5-inch feels slow and stable. Every world is built at true scale, and a test checks key
dimensions (door height, ceiling, gate size) against the world's own spec.

| World | Size | What it trains |
|---|---|---|
| Living room | 5 × 4 × 2.5 m, sofa, table, doorway to a hall | Hover, slow lines, doorways, ground effect |
| Garage or basement | 6 × 6 × 2.6 m, shelving, a few gates | Tight turns, power loops under a low ceiling |
| Gym | 30 × 18 × 7 m, small whoop gates and flags | Racing lines, speed |
| Backyard | 15 × 10 m, fence, trees, a shed | Outdoor whoop flying, light wind |
| Field | 200 × 200 m, poles, a few trees | Larger quads, open flying |

- Phase 1 ships one plain room (the living room as boxes, flat shaded) in the minimal renderer.
- A **box-room builder** (later): enter a room's width, depth, height and door and window
  positions, and fly your own room. The result is user data in the gear folder, never committed.

### 8.2 Camera

- FPV by default. Chase and line-of-sight as options.
- Uptilt, FOV and aspect from the profile (6.1). Indoor whoop flying wants a low uptilt; a wrong
  uptilt reads as floaty.
- The camera sits at the real camera's position in the frame, so ground and wall distances look
  right.

### 8.3 Assets and licences

| Rule | Detail |
|---|---|
| Licences allowed | CC0 first (Poly Haven, ambientCG, Kenney). CC BY with credit. Never NC, ND or SA. Never "free for personal use" |
| Manifest | Every asset file has a row in `sim/assets/LICENSES.toml`: source URL, author, licence, changes. `scripts/notices.mjs` reads it, adds the credits to `THIRD_PARTY_NOTICES`, and fails the build on an asset with no row or a licence outside the list |
| Format | glTF 2.0 for meshes, KTX2 or PNG textures, HDR environment maps. Engine-neutral, so the engine choice does not lock the assets in |
| Size | Worlds beyond the first ship as QuadCam modules (`docs/modules.md`), downloaded on request, so the `.app` stays small. A world module's index entry carries its licence |
| Quad models | Low-poly whoop and 5-inch models, own work or CC0. propwash's drone model is CC BY 4.0: allowed with its credit |
| Own work | Blockouts and simple props built in Blender for QuadCam are MIT, like the code |

Lighting and assets decide how good the sim looks far more than the engine does. Worlds get
baked lighting where the engine supports it, and one real-time light for shadows.

---

## 9. Audio and UI

### 9.1 Motor sound

- **Synthesis, per motor**, driven by motor speed from the snapshots:
  - Blade-pass tone: rpm / 60 × blades. A tri-blade whoop hovers near 1.0-1.4 kHz and punches
    to 2-2.7 kHz; a 7-inch hovers near 0.5 kHz. The synth must cover the whoop's range; the
    spike's synth was tuned for 7-inch speeds.
  - Motor whine from the electrical frequency (pole pairs × rpm / 60), quieter.
  - Harmonics and the beat between four motors at slightly different speeds.
  - Loudness from thrust. Prop-wash events add short noise bursts tied to their severity.
- **No constant wind noise.** The spike removed two constant noise layers (a wash rumble and an
  airspeed rush), and the owner asked for none. An optional "Airflow swish" (a short burst on a
  hard flip or punch) exists, off by default.
- ESC start-up tones, arming beeps, the lost-model beeper after a disarm.
- **Engine**: own synth in Rust on an audio thread (`cpal`, Apache-2.0), independent of the
  renderer, so it is the same for every engine. Target output latency about 10 ms.
- Recorded samples only if CC0 or recorded for QuadCam. propwash's optional recording has an
  unknown source and is never used.
- Source: propwash's procedural motor voice (reuse code, re-tuned).

### 9.2 Sim page in QuadCam

- Gear > **Sim** in the sidebar: pick the aircraft (its profile and fit state), the world, then
  **Fly**. Shows the radio and its calibration state, and the last session's summary.
- The profile view: each value with its source, the last validation, **Fit from logs…** and
  **Validate**.

### 9.3 Settings

The spike's settings panel grew by accretion. The sim's settings are few and grouped:

| Group | Settings |
|---|---|
| Quad | Aircraft, rates source (the quad's `diff all`, or a saved preset), battery sag (off), prop wash (one slider) |
| Radio | Calibration (opens the screen), stick mode, reset switch |
| Video | View (FPV), camera uptilt and FOV (from the profile, editable), aspect (16:9 or 4:3), video latency (ms), analog look (off), link model (off, packet rate) |
| Display | Full screen or window, OSD (on), stick display (on) |
| Sound | Volume, airflow swish (off) |

- The settings panel is a popover: a click outside it closes it, Esc closes it.
- Each setting saves on change through `Core`. No Save button.
- Labels say what a setting does. Help text explains units, not design reasons.

### 9.4 In flight

| Element | Design |
|---|---|
| OSD | The quad's own OSD layout from its `diff all` (WP7's `osd.rs`: elements and positions per profile, on the analog or HD grid). The sim fills the values: voltage, timer, mAh, link quality, throttle, flight mode, warnings |
| Stick display | WP6's `Sticks` design, compact, in the stick mode of the radio's calibration, with live percentages. It sits inside the picture's right edge above the OSD's bottom two rows, so it never covers voltage, timer or mAh. A button or the setting hides it |
| 4:3 | Crops the picture to 4:3 with bars, as an analog or O4 4:3 feed |
| Video latency | Delays the shown frame by the set time (default 0; 12-25 ms for a digital system, about 10 ms analog). Physics and audio are not delayed |
| Analog look | A post-process: noise, chroma bleed, rolling lines, breakup with distance and obstacles later. Source: `ntsc-rs` core crates (MIT, Apache-2.0 or ISC; not its GUI crate) (reuse code) |
| Toasts | Arming refusals, failsafe, reset. Short, at the bottom centre, away from the OSD's throttle readout |

---

## 10. Licensing

QuadCam's code stays MIT (gear-design, section 12). Every borrowed idea and every code source:

### 10.1 Sources

| Source | Licence | Use | What |
|---|---|---|---|
| notfeylo/propwash | MIT | Reuse code, with its notice | Aero set (inflow, ground effect, prop-wash severity and noise, attitude drag, Dryden-lite), first-order motor fallback, battery, turtle, rate math, RC smoothing and feedforward, collider streaming, motor sound synthesis |
| propwash drone model | CC BY 4.0 | Asset, with credit | A quad model, if used |
| The spike's own changes to propwash | MIT (QuadCam's own work on an MIT base) | Reuse code | Calibration flow, per-side coverage, endpoints and deadzone, stick display, whoop airframes, arming fix |
| Mathew-Harvey/WebFPVSimulator | GPL-3.0 | Idea only | DC-motor electrical model, kQ from kT through FM, whoop reasoning (pack resistance, low FM, duct drag), link jitter model, numeric verification bands |
| Betaflight (firmware, Configurator, SITL) | GPL-3.0 | Idea only, from docs | Rate models, RC smoothing, feedforward, I-term relax, angle and horizon, airmode, crashflip, arming guards, failsafe |
| blackbox-tools (`blackbox_decode`) | GPL-3.0 | Idea only; may run as a separate program for test fixtures, never bundled | Blackbox format (QuadCam writes its own reader) |
| SimITL / pr0p | GPL-3.0 | Idea only | Frame resonance and motor asymmetry ideas (later) |
| RotorPy | MIT | Reuse code, with its notice | Rotor H-force, induced inflow terms |
| Flightmare | MIT | Reuse code | Integrator sub-stepping structure |
| gym-pybullet-drones | MIT | Reuse code | Ground effect and drag terms (from published papers) |
| mqnc/propwash | MIT (code); assets mixed, one CC BY-NC-SA | Idea only for structure; no assets | Physics in a worker thread |
| Rapier | Apache-2.0 | Dependency | Collisions |
| Bevy, wgpu | MIT or Apache-2.0 | Dependency (if chosen) | Renderer |
| Godot | MIT | Runtime (if chosen) | Renderer, editor |
| godot-rust (gdext) | MPL-2.0 | Dependency, unmodified (if Godot is chosen) | GDExtension binding |
| three.js | MIT | Dependency (phase 1 page) | Minimal renderer |
| cpal | Apache-2.0 | Dependency | Audio output |
| ntsc-rs core crates | MIT, Apache-2.0 or ISC | Reuse code | Analog look. Not its GUI crate |
| Chromium, CEF, Electron | BSD-3 and others | Only if chosen; large notices list | Renderer |
| Momentum theory, Cheeseman-Bennett ground effect, Leishman VRS curve, Dryden turbulence (MIL-F-8785C), DC-motor equations | Published | Fact | Physics terms |
| ELRS packet rates, Betaflight CLI setting names | Published | Fact | Link model, profile import |
| The Zone, Liftoff, Velocidrone, Uncrashed, SkyDive | Closed | Behaviour only | Calibration flow, latency and 4:3 options, whoop presets |
| No-licence repos (leyasherman/Drone-simulator and others) | None | Nothing | Not read for code |
| World assets | CC0 or CC BY | Assets, with credit | Section 8.3 |

### 10.2 Constraints

| Constraint | How this design meets it |
|---|---|
| Never copy GPL source | The FC model, rates and blackbox reader are written from docs, logs and measured behaviour. A reviewer checks each package for copied code |
| A GPL program only as a separate process, downloaded | Real Betaflight SITL is a non-goal. If it ever comes, it runs as a module (7.10 in gear-design), over UDP, never bundled |
| propwash: no association | No fork, no upstream contributions, no link or branding tie. Ported MIT code keeps propwash's copyright and permission notice in a `LICENSE` file beside the ported source and in `THIRD_PARTY_NOTICES`. The UI, About and `README.md` do not have to name it |
| Notices for what the `.app` bundles | `scripts/notices.mjs` covers the new crates and npm packages, the propwash and RotorPy notices for ported code, and the asset manifest (8.3) |
| MPL-2.0 dependencies | File-level copyleft only; fine unmodified, as `serialport` today |

---

## 11. Build plan: work packages and releases

Each package owns its files; another package changes them only through a small, agreed edit.
Every package ends with green CI, its docs (`docs/sim.md`, the user guide, grows with each) and
its rows in `api`, CLI and MCP.

### 11.1 Packages

| Id | Title | Owns | Depends on | Phase |
|---|---|---|---|---|
| S1 | Physics crate | `src-tauri/sim/`: rigid body, motors, props, ducts, drag, ground effect, prop wash, VRS, battery, Rapier collisions, FC (rates, smoothing, PID, angle, horizon, airmode, turtle, arming), the stepping thread, snapshots, recordings; built-in presets | – | 1 |
| S2 | Validation harness | `sim/src/validate.rs`, `sim/tests/`, fixtures and their scrubber, the headless bench | S1 | 1 |
| S3 | Profile fitting | `sim/src/fit.rs`, profile storage under `<gear>/sim/`, `gear_sim_fit` / `profile(s)` rows, Fit and profile views | S1, S2; task BB for native decoding | 2 |
| S4 | Input and calibration | `radio_hid` unthrottled subscriber, `sim/src/input.rs` (map, ends, deadzone, Reverse, link model), calibration storage keyed by radio id, the calibration screen | WP6 | 1 |
| S5 | Minimal renderer and in-process host | Sim window (WebView, three.js), pose stream and interpolation, FPV camera, the plain room, basic OSD, stick display, Sim page, launch, settings popover | S1, S4 | 1 |
| S6 | Engine prototypes and report | The four prototypes (3.2), the measurement scripts, the report | S1, S5 | 2 |
| S7 | Production renderer | The chosen engine's host (sim process, signing, control socket), cameras, lighting, OSD from WP7, 4:3, video latency, analog look | S6 decision; WP7 | 3 |
| S8 | Worlds | The five worlds, the asset manifest and its check, world modules, the box-room builder | S7 | 3 |
| S9 | Audio | The Rust synth, beeps, the swish option | S1 | 2 |
| S10 | Polish | Settings clean-up across hosts, session summary, MCP and CLI completeness, `docs/sim.md` complete | S5, S7 | 3 |
| BB | Blackbox pull and reader | Task outside the sim: pull on connect, verify, erase (a setting), QuadCam's own decoder | WP2, WP4 | 2 |

**Phase 1** (fly through a minimal renderer): S1, S2, S4, S5. Profiles start from the built-in
presets with the example fits (6.3) entered by hand.
**Phase 2**: S3, S6, S9, BB. The owner picks the engine from S6's report.
**Phase 3**: S7, S8, S10.

### 11.2 Acceptance criteria

| Id | Accepted when |
|---|---|
| S1 | Tests in 4.12 pass: convergence across 1, 2 and 4 kHz, determinism hash, energy, per-term unit checks. Bench: p99 step under 100 µs at 2 kHz in free flight and in contact. A 10 min real-time run drops no steps after the first second. A forced 100 ms stall drops sim time and logs it, with no slow motion. Rate curves match reference values for Betaflight, Actual and Quick (shared with WP8). Turtle, airmode, angle and arming guards each have a test |
| S2 | Every check in 6.4 runs on the CI fixtures and the 75 mm and 65 mm example profiles pass their bands. The fixture scrub test fails on a planted UID. The harness runs on a folder of decoded logs from the CLI |
| S3 | The Rust fit reproduces the spike's Python fit on the same windows (hover command ±0.005, speed ±2 %, rise time ±2 ms, resistance ±0.005 Ω, lag ±1 ms). The report lists every value's source and residual. A fit changes the profile only after confirm |
| S4 | Against `FakeHid`: every report reaches the physics ring with its time; the calibration flow (move, let go, arm switch, review) produces the right map, ends and centres; per-side coverage refuses a one-sided sweep; deadzone rescales with no jump; edited ends survive and Recalibrate clears them; calibrations save by radio id and the resolve step asks when two radios share a product name. Link model: packet rate, jitter and loss are seeded and replay exactly |
| S5 | Headless (no window focus): the window loads, flies a recorded flight, the pose interpolates, FPV is the default view, the stick display does not overlap the OSD at 16:10 and 16:9. The owner flies it with the radio |
| S6 | All four prototypes run the same recording; the report holds every measure in 3.3 for each, and the decision rule's result |
| S7 | The chosen host meets the 3.3 bar on the owner's Mac. The sim process is signed and notarized in the release build. A renderer crash leaves QuadCam running and the Sim page reports it |
| S8 | Every world passes its scale checks. `notices.mjs --check` fails on an asset with no manifest row. A world module installs and removes like the ffmpeg module |
| S9 | The synth covers 5k to 60k rpm with no aliasing (golden spectra in tests); no audio node runs when the motors are stopped and the quad is still (no constant noise); latency at most 15 ms on the default device |
| S10 | Settings match 9.3; a click outside closes the popover; MCP schema snapshot updated; `docs/sim.md` covers every screen |
| BB | Against `FakeFc`: the pull reads only used bytes, verifies, stores the blob in WP4's store and erases only after verify and only with the setting on. The decoder matches `blackbox_decode`'s CSV on the fixture logs, field by field |

### 11.3 Release mapping

Gear's release line (gear-design, section 11) stays as it is. The sim rides along it:

| Release | Sim packages | Owner tests on real gear |
|---|---|---|
| 0.8.0 (with WP6, WP8) | S1, S2, S4, S5: **Sim preview** behind a setting | Calibrate the radio once; quit and relaunch: no calibration screen. Fly the 75 mm and 65 mm presets in the plain room: hover near the real hover throttle, punch, flips, turtle on its switch. Drop a frame on purpose (open another heavy app): no slow motion |
| 0.9.0 | S3, S6, S9, BB | Plug a whoop in: blackbox pulled and erased (setting on). Fit its profile; validate it. Fly the fitted profile against the preset. Fly each engine prototype and film the latency test. Choose the engine |
| 0.10.0 | S7, S8 | Fly each world at 120 Hz; check scale (doorways, ceiling). Install and remove a world module |
| 1.0.0 | S10 | One session: plug the quad in, fit, fly the sim on its rates, then fly the real quad |

Whether the sim gates 1.0 is open question 7.

---

## 12. Open questions for the owner

Each has a default the build uses until you decide.

| # | Question | Default |
|---|---|---|
| 1 | Engine: after S6's measurements, which one? | Bevy in a sim process, if it meets the bar (3.4) |
| 2 | Physics step: 2 kHz, or 1 kHz if the convergence test shows no difference? | 2 kHz until S1's test |
| 3 | Calibration for an EdgeTX radio QuadCam knows: a short check (move each stick once), or always the full move-and-let-go flow? | Short check; Recalibrate runs the full flow |
| 4 | With battery sag off, what constant voltage per cell? | 3.9 V (mid-pack, loaded) |
| 5 | PIDs: the sim's own gains fitted to the logged response (5.3), or the quad's PIDs on the model plant? | The sim's own; the quad's PIDs shown for reference |
| 6 | Fit flights: publish a short "fit flight" routine (hover, punch, coast, drop and punch out, flicks) in the user guide? | Yes |
| 7 | Does the sim gate 1.0, or ship as a preview alongside it? | A preview; not a 1.0 gate |
| 8 | Which worlds first after the plain room? | Living room, gym, backyard |
| 9 | World assets: CC0 only, or CC BY with credits too? | CC0 and CC BY; never NC, ND or SA |
| 10 | Reset: which radio control resets the quad? The sim needs a free switch or button | User picks in settings; none by default (menu only) |
| 11 | Video latency default: 0 ms, or the profile's video system (about 12-25 ms digital, about 10 ms analog)? | The profile's video system |
| 12 | Window: full screen by default, or a window? | Full screen on QuadCam's display |
