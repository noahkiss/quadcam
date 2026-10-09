// The Rates segment's pure helpers: names, chart geometry, and the things a curve can be
// compared with (another profile, another quad, a sim's profile).
import type { RateAxis, RateProfile, RateThrottle, SimRates } from "../ipc/types";

export const AXIS_LABEL: Record<string, string> = { roll: "Roll", pitch: "Pitch", yaw: "Yaw" };
export const RATES_TYPE: Record<string, string> = { betaflight: "Betaflight", actual: "Actual", quick: "Quick", raceflight: "Raceflight", kiss: "KISS" };
export const LIMIT_LABEL: Record<string, string> = { off: "Off", scale: "Scale", clip: "Clip" };

/** "0 FREE", or "Profile 0" for a profile with no name. */
export const profileLabel = (p: Pick<RateProfile, "index" | "name">) => (p.name ? `${p.index} ${p.name}` : `Profile ${p.index}`);

export const deg = (v: number) => `${Math.round(v)}°/s`;

/** The chart's top value: the largest value rounded up to a step. */
export function niceMax(values: number[], step = 100): number {
  const m = Math.max(0, ...values.filter(Number.isFinite));
  return Math.max(step, Math.ceil(m / step) * step);
}

/** Tick values from 0 to `max`, `n` steps. */
export const ticks = (max: number, n = 4): number[] => Array.from({ length: n + 1 }, (_, i) => (max * i) / n);

/** An SVG polyline's points for a curve sampled at stick i / (n - 1), drawn in a box. */
export function points(curve: number[], box: { x: number; y: number; w: number; h: number }, max: number): string {
  if (curve.length < 2) return "";
  return curve
    .map((v, i) => {
      const px = box.x + (i / (curve.length - 1)) * box.w;
      const py = box.y + box.h - (Math.min(Math.max(v, 0), max) / max) * box.h;
      return `${px.toFixed(1)},${py.toFixed(1)}`;
    })
    .join(" ");
}

/** The largest difference between two curves, over the points they share. */
export function maxDiff(a: number[], b: number[]): number {
  const n = Math.min(a.length, b.length);
  let m = 0;
  for (let i = 0; i < n; i++) m = Math.max(m, Math.abs(a[i] - b[i]));
  return m;
}

/** Something a profile's curves are drawn against. */
export interface CompareTarget {
  key: string;
  group: "profile" | "quad" | "sim";
  label: string;
  axes: RateAxis[];
  throttle: RateThrottle | null;
}

export const profileTarget = (p: RateProfile, group: "profile" | "quad", label: string): CompareTarget => ({ key: `${group}:${label}`, group, label, axes: p.axes, throttle: p.throttle });

/** Every readable sim profile as a target. */
export function simTargets(sims: SimRates[]): CompareTarget[] {
  return sims.flatMap((s) =>
    s.files.flatMap((f) =>
      f.profiles
        .filter((p) => p.supported)
        .map((p) => ({ key: `sim:${s.id}:${f.path}:${p.name}`, group: "sim" as const, label: `${s.name} · ${p.name}`, axes: p.axes, throttle: p.throttle })),
    ),
  );
}

/** One line on a sim's state against the quad. */
export function simState(s: SimRates): string {
  if (!s.enabled) return "Off";
  if (!s.found) return "No rate file";
  if (s.files.every((f) => f.error)) return "Cannot read";
  if (s.in_sync === true) return "Matches the quad";
  if (s.in_sync === false) return "Differs from the quad";
  return "Read";
}

/** The largest per-axis difference of a sim profile from the quad, in deg/s. */
export const worstDiff = (d: { max_diff: number[] } | null): number | null => (d ? Math.max(0, ...d.max_diff) : null);
