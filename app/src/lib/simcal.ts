// The sim's radio calibration as the screen shows it: the names of each axis' ends (they
// follow Reverse, as in the core's `AxisCal::end_name`), a control in words, the step text,
// and the calibrated output as the Sticks widget's values.
import type { AxisCal, CalPhase, CalibrateView, CaptureTarget, RadioControl } from "../ipc/types";
import type { StickValues } from "./controls";

export type StickFn = "roll" | "pitch" | "throttle" | "yaw";
export const STICK_FNS: StickFn[] = ["roll", "pitch", "throttle", "yaw"];

const ENDS: Record<StickFn, [string, string]> = {
  roll: ["Left end", "Right end"],
  yaw: ["Left end", "Right end"],
  pitch: ["Back end", "Forward end"],
  throttle: ["Bottom", "Top"],
};

/** The name of an axis' low or high raw end. */
export function endName(f: StickFn, end: "low" | "high", reverse: boolean): string {
  const [lo, hi] = ENDS[f];
  return (end === "low") !== reverse ? lo : hi;
}

export const lowEnd = (a: AxisCal) => a.edited_low ?? a.auto_low;
export const highEnd = (a: AxisCal) => a.edited_high ?? a.auto_high;

/** `CH5 1700-2100 µs`, `Button 1`, `None`. */
export function controlText(c: RadioControl | null | undefined): string {
  if (!c) return "None";
  if (c.kind === "channel") return `CH${c.ch} ${c.min_us}-${c.max_us} µs`;
  return `Button ${c.button}${c.pressed ? "" : " released"}`;
}

export const TARGET_TEXT: Record<CaptureTarget, string> = {
  arm: "Arm",
  reset: "Reset",
  turtle: "Turtle",
  angle: "Angle",
  horizon: "Horizon",
  airmode: "Air mode",
};

/** What the person does at each step. */
export function stepText(phase: CalPhase, quick: boolean, target: CaptureTarget | null | undefined): string {
  switch (phase) {
    case "move":
      return quick ? "Move each stick all the way both ways." : "Move both sticks around their full travel.";
    case "let_go":
      return "Let go of the sticks.";
    case "arm":
      return "Flip the arm switch.";
    case "reset":
      return "Press the button or trim to use for reset.";
    case "capture":
      return `Move the control for ${target ? TARGET_TEXT[target] : "it"}.`;
    case "review":
      return "Check each axis, then save.";
  }
}

/** The calibrated output as the widget draws it: −1..1, throttle low at −1. */
export function sticksOf(v: CalibrateView | null): StickValues | null {
  const s = v?.sticks;
  if (!s) return null;
  return { roll: s.roll / 100, pitch: s.pitch / 100, yaw: s.yaw / 100, throttle: s.throttle / 50 - 1 };
}
