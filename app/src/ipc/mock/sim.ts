// The mock core's sim calibration (`core/sim.rs`), written by hand: which radio the
// joystick is, the defaults (a fixed synthetic aircraft), and a session that steps on the
// person's actions and follows the radio frames specs send. It does not sweep for ends; the
// core's tests cover the flow itself.
import type { AxisCal, CalibrateParams, CalibrateView, Calibration, CalPhase, SavedCalibration, SimCalibration, SimDefaults } from "../types";
import type { Device } from "../types";

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
