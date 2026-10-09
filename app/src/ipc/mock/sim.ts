// The mock core's sim calibration (`core/sim.rs`), written by hand: which radio the
// joystick is, the defaults (a fixed synthetic aircraft), and a session that steps on the
// person's actions and follows the radio frames specs send. It does not sweep for ends; the
// core's tests cover the flow itself.
import type { AxisCal, CalibrateParams, CalibrateView, Calibration, CalPhase, SavedCalibration, SimCalibration, SimDefaults } from "../types";
import type { Device, SimBox, SimFrame, SimPose, SimPreset, SimStartInfo, SimStartParams, SimStopInfo } from "../types";

const axis = (ch: number): AxisCal => ({ ch, auto_low: 0, auto_high: 2048, edited_low: null, edited_high: null, centre: 1024, deadzone: 0, reverse: false });

export const defaultCalibration = (map = [1, 2, 3, 4]): Calibration => ({
  mode: 2,
  roll: axis(map[0]),
  pitch: axis(map[1]),
  throttle: axis(map[2]),
  yaw: axis(map[3]),
  arm: null,
  reset: null,
  turtle: null,
  angle: null,
  horizon: null,
  airmode: null,
});

/** Any aircraft: its model gives AETR, SA arms, SB selects angle and horizon, SE turtles,
 *  a free trim resets. No aircraft: AETR and nothing else. */
export function defaults(aircraft: string | null): SimDefaults {
  if (!aircraft) {
    return { aircraft: null, map: { roll: 1, pitch: 2, throttle: 3, yaw: 4, source: "EdgeTX's default order (AETR)", known: false }, notes: [], calibration: defaultCalibration() };
  }
  const src = "your radio model";
  const d: SimDefaults = {
    aircraft,
    map: { roll: 1, pitch: 2, throttle: 3, yaw: 4, source: src, known: true },
    arm: { control: { kind: "channel", ch: 5, min_us: 1700, max_us: 2100 }, label: "SA down", source: src },
    angle: { control: { kind: "channel", ch: 6, min_us: 900, max_us: 1300 }, label: "SB up", source: src },
    horizon: { control: { kind: "channel", ch: 6, min_us: 1300, max_us: 1700 }, label: "SB mid", source: src },
    turtle: { control: { kind: "button", button: 1, pressed: true }, label: "SE down", source: src },
    airmode: null,
    reset: { control: { kind: "button", button: 2, pressed: true }, label: "TrimRudLeft", source: src },
    notes: [],
    calibration: defaultCalibration(),
  };
  d.calibration = { ...d.calibration, arm: d.arm!.control, reset: d.reset!.control, turtle: d.turtle!.control, angle: d.angle!.control, horizon: d.horizon!.control };
  return d;
}

export class MockSim {
  saved = new Map<string, SavedCalibration>();
  view: CalibrateView | null = null;
  /** Raw axes and buttons, from the specs' radio frames. */
  axes: number[] = [1024, 1024, 0, 1024, 0, 0, 0, 0];
  buttons = 0;

  calibration(connected: boolean, devices: Device[], radio: string | null): SimCalibration {
    if (radio) return { resolution: null, calibration: this.saved.get(radio) ?? null, firmware: null };
    if (!connected) throw "No radio in USB Joystick mode. Plug it in and choose USB Joystick on the radio.";
    const product = "Test Radio Joystick";
    const choices = devices.filter((d) => d.kind === "radio" && /test/i.test(d.identity?.board ?? "")).map((d) => ({ id: d.id, name: d.name || "Unnamed Radio" }));
    const key = choices.length === 1 ? choices[0].id : choices.length ? null : "usb-00000000000000f1";
    return {
      resolution: { product, radio: key, provisional: key?.startsWith("usb-") ?? false, choices, how: key?.startsWith("usb-") ? "No saved radio is this model yet. Link it to one once its card has been read." : "The only saved radio of this model." },
      calibration: key ? (this.saved.get(key) ?? null) : null,
      firmware: "2.12",
    };
  }

  save(p: { radio: string; calibration: Calibration; product?: string | null; firmware?: string | null }): SavedCalibration {
    const s: SavedCalibration = { radio: p.radio, provisional: p.radio.startsWith("usb-"), calibration: p.calibration, product: p.product ?? null, vid: 0x1209, pid: 0x4f54, firmware: p.firmware ?? null, saved_at: "2026-10-07T12:00:00Z" };
    this.saved.set(p.radio, s);
    return s;
  }

  private live(): CalibrateView {
    const v = this.view!;
    const c = v.calibration;
    const centred = (a: AxisCal) => Math.round(((this.axes[a.ch - 1] - a.centre) / 1024) * 100) * (a.reverse ? -1 : 1);
    return {
      ...v,
      sticks: { roll: centred(c.roll), pitch: centred(c.pitch), yaw: centred(c.yaw), throttle: Math.round((this.axes[c.throttle.ch - 1] / 2048) * 100) },
      channels: this.axes.map((a) => 988 + Math.floor(a / 2)),
      buttons: this.buttons,
    };
  }

  /** A radio frame while a session runs. */
  frame(axes: number[], buttons: number): CalibrateView | null {
    this.axes = axes;
    this.buttons = buttons;
    if (!this.view?.active) return null;
    this.view = this.live();
    return this.view;
  }

  calibrate(p: Partial<CalibrateParams>): CalibrateView {
    const go = (phase: CalPhase) => {
      this.view = { ...this.view!, phase, target: phase === "capture" ? this.view!.target : null, message: null };
    };
    switch (p.action ?? "get") {
      case "start": {
        const quick = !!p.quick;
        this.view = {
          active: true,
          connected: true,
          phase: p.review ? "review" : "move",
          target: null,
          quick,
          arm_known: !!p.arm_known,
          calibration: p.calibration ?? defaultCalibration(),
          coverage: { roll: 0, pitch: 0, throttle: 0, yaw: 0 },
          sticks: null,
          channels: [],
          buttons: 0,
          message: null,
        };
        break;
      }
      case "stop":
        if (this.view) this.view = { ...this.view, active: false };
        return this.view ?? { ...this.calibrate({ action: "start" }), active: false };
      case "get":
        break;
      default:
        if (!this.view?.active) throw "No calibration is running: start one first.";
    }
    const v = this.view!;
    switch (p.action) {
      case "advance":
        if (v.phase === "move") go(v.quick ? (v.arm_known ? "reset" : "arm") : "let_go");
        else if (v.phase === "let_go") go(v.arm_known ? "reset" : "arm");
        else if (v.phase === "arm") go("reset");
        else go("review");
        break;
      case "skip":
        if (v.phase === "arm") this.view = { ...v, calibration: { ...v.calibration, arm: null } };
        if (v.phase === "reset") this.view = { ...v, calibration: { ...v.calibration, reset: null } };
        if (v.phase === "capture" && v.target) this.view = { ...v, calibration: { ...v.calibration, [v.target]: null } };
        go(v.phase === "arm" ? "reset" : "review");
        break;
      case "recalibrate": {
        const c = structuredClone(v.calibration);
        for (const f of ["roll", "pitch", "throttle", "yaw"] as const) c[f] = { ...c[f], edited_low: null, edited_high: null };
        this.view = { ...v, calibration: c, quick: false };
        go("move");
        break;
      }
      case "capture":
        this.view = { ...v, target: p.target ?? null };
        go("capture");
        break;
      case "set":
        this.view = { ...v, calibration: p.calibration! };
        break;
    }
    this.view = this.live();
    return this.view;
  }
}

// ---------- the sim host (`core/sim_host.rs`) ----------

const PRESETS = [
  { id: "meteor75", label: "75 mm whoop", wheelbase_mm: 75, mass_g: 29, uptilt: 15, fov: 155 },
  { id: "air65ii", label: "65 mm whoop", wheelbase_mm: 65, mass_g: 22, uptilt: 15, fov: 155 },
  { id: "five_inch", label: "5 inch freestyle", wheelbase_mm: 220, mass_g: 650, uptilt: 30, fov: 140 },
  { id: "seven_inch", label: "7 inch long range", wheelbase_mm: 300, mass_g: 900, uptilt: 20, fov: 140 },
];

const bx = (centre: [number, number, number], half: [number, number, number], material: SimBox["material"], yaw_deg = 0): SimBox => ({ centre, half, yaw_deg, material });

/** The core's reference room: a 5 × 4 × 2.5 m box, a crate, a table and two gates. */
export const ROOM: SimBox[] = [
  bx([0, 0, -0.1], [2.6, 2.1, 0.1], "floor"),
  bx([0, 0, 2.6], [2.6, 2.1, 0.1], "wall"),
  bx([2.6, 0, 1.25], [0.1, 2, 1.25], "wall"),
  bx([-2.6, 0, 1.25], [0.1, 2, 1.25], "wall"),
  bx([0, 2.1, 1.25], [2.5, 0.1, 1.25], "wall"),
  bx([0, -2.1, 1.25], [2.5, 0.1, 1.25], "wall"),
  bx([-1.6, -1.2, 0.25], [0.25, 0.25, 0.25], "wall", 20),
  bx([1.5, -1.2, 0.2], [0.5, 0.3, 0.2], "wall"),
  bx([1.2, 0.62, 0.28], [0.03, 0.03, 0.28], "gate"),
  bx([1.2, 1.18, 0.28], [0.03, 0.03, 0.28], "gate"),
  bx([1.2, 0.9, 0.59], [0.03, 0.31, 0.03], "gate"),
];

/** A canned flight in place of the physics: a slow circle, so a spec can watch the pose move,
 *  with the HUD and the sticks following the radio frames the spec sends. */
export class MockHost {
  info: SimStartInfo | null = null;
  startedAt = 0;
  resets = 0;
  starts: SimStartParams[] = [];
  private armedAt: number | null = null;

  presets(): SimPreset[] {
    return PRESETS.map((p) => ({ id: p.id, label: p.label, wheelbase_mm: p.wheelbase_mm, mass_g: p.mass_g }));
  }

  start(p: SimStartParams): SimStartInfo {
    this.starts.push(p);
    const pre = PRESETS.find((x) => x.id === (p.profile ?? "meteor75"));
    if (!pre) throw `No sim profile "${p.profile}". Built-in profiles: ${PRESETS.map((x) => x.id).join(", ")}.`;
    this.startedAt = performance.now();
    this.armedAt = null;
    this.info = {
      profile: pre.id,
      label: pre.label,
      dt: 0.0005,
      world_name: "plain room",
      boxes: ROOM,
      start: [0, 0, 0],
      start_yaw_deg: 0,
      camera: { uptilt_deg: pre.uptilt, fov_deg: pre.fov, aspect: "16:9", position: [pre.wheelbase_mm / 5000, 0, 0.01] },
      wheelbase_m: pre.wheelbase_mm / 1000,
      body_half: [0.02, 0.02, 0.01],
      prop_radius_m: 0.02,
      stick_mode: p.calibration?.mode ?? 2,
      calibrated: !!p.calibration,
      host_ns: hostNs(),
    };
    return this.info;
  }

  stop(): SimStopInfo | null {
    if (!this.info) return null;
    this.info = null;
    return { steps: 1000, dropped_steps: 0, step_p50_us: 5, step_p99_us: 9 };
  }

  reset() {
    if (!this.info) throw "The sim is not running.";
    this.resets++;
    this.startedAt = performance.now();
  }

  frame(axes: number[], radio: boolean): SimFrame {
    if (!this.info) throw "The sim is not running.";
    const now = hostNs();
    const step = Math.floor(now / 5e5);
    const pose = (s: number): SimPose => {
      const t = (s * 5e5 - hostAt(this.startedAt)) / 1e9;
      const a = 0.5 * t;
      const yaw = a + Math.PI / 2;
      const roll = 0.15;
      // yaw about z, then roll about the body's x
      const [cy, sy, cr, sr] = [Math.cos(yaw / 2), Math.sin(yaw / 2), Math.cos(roll / 2), Math.sin(roll / 2)];
      return { step: s, t, host_ns: s * 5e5, input_ns: radio ? s * 5e5 - 4e6 : 0, pos: [1.2 * Math.cos(a), 1.2 * Math.sin(a), 0.8], quat: [cy * cr, cy * sr, sy * sr, sy * cr] };
    };
    const [roll, pitch, thr, yaw] = [(axes[0] - 1024) / 1024, (axes[1] - 1024) / 1024, axes[2] / 2048, (axes[3] - 1024) / 1024];
    const armed = axes[4] > 1024 && thr < 0.1;
    if (armed && this.armedAt === null) this.armedAt = pose(step).t;
    if (!(axes[4] > 1024)) this.armedAt = null;
    const cur = pose(step);
    const flown = this.armedAt === null ? 0 : cur.t - this.armedAt;
    return {
      host_ns: now,
      prev: pose(step - 1),
      cur,
      hud: {
        armed: this.armedAt !== null,
        turtle: false,
        airmode: false,
        mode: "acro",
        arm_block: this.armedAt === null && thr >= 0.1 ? "Throttle is up: lower it to arm" : null,
        vbat: 3.9,
        mah: flown * 1.4,
        low_battery: false,
        sticks: { roll, pitch, yaw, throttle: thr },
        throttle: thr,
        stuck: false,
        contacts: 0,
        dropped_steps: 0,
      },
      radio,
    };
  }
}

// The mock's "host clock": the page's plus two hours, so the page has to align them.
const HOST_OFFSET_MS = 7_200_000;
const hostNs = () => (performance.now() + HOST_OFFSET_MS) * 1e6;
const hostAt = (pageMs: number) => (pageMs + HOST_OFFSET_MS) * 1e6;
