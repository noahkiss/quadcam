// The Rates segment's pure helpers: names, chart geometry, and the things a curve can be
// compared with (another profile, another quad, a sim's profile).
import type { Edit, RateAxis, RateProfile, RateThrottle, SimRates } from "../ipc/types";

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
  if (s.in_sync === false) return "Out of date";
  return "Read";
}

/** The largest per-axis difference of a sim profile from the quad, in deg/s. */
export const worstDiff = (d: { max_diff: number[] } | null): number | null => (d ? Math.max(0, ...d.max_diff) : null);

// ----- editing -----

/** The rate models a profile can use, in the order the editor lists them. */
export const RATE_MODELS = ["betaflight", "actual", "quick", "raceflight", "kiss"] as const;

/** What the three numbers of an axis are called under each model (the CLI's `*_rc_rate`,
 * `*_srate`, `*_expo`). */
export const MODEL_FIELDS: Record<string, [string, string, string]> = {
  betaflight: ["RC rate", "Super rate", "Expo"],
  actual: ["Center rate", "Max rate", "Expo"],
  quick: ["Center rate", "Max rate", "Expo"],
  raceflight: ["Rate", "Acro plus", "Expo"],
  kiss: ["RC rate", "Rate", "Curve"],
};

/** The CLI range of rc rate, super rate and expo under each model: the same table the
 * core's fit clamps to (`gear/rates.rs`, `bounds`). */
export const MODEL_BOUNDS: Record<string, [[number, number], [number, number], [number, number]]> = {
  betaflight: [[1, 255], [0, 99], [0, 100]],
  actual: [[1, 200], [1, 200], [0, 100]],
  quick: [[1, 255], [1, 200], [0, 100]],
  raceflight: [[1, 255], [0, 255], [0, 100]],
  kiss: [[1, 255], [0, 99], [0, 100]],
};

const AXES = ["roll", "pitch", "yaw"] as const;

/** `set` edits (rate profile section) that make `before` read as `after`. Only the values
 * that differ; the order is name, model, axes, throttle. An empty name is left alone (the
 * CLI cannot clear it). */
export function profileEdits(before: RateProfile, after: RateProfile): Edit[] {
  const section = { kind: "rate_profile" as const, index: before.index };
  const out: Edit[] = [];
  const set = (name: string, a: string | number | null, b: string | number | null) => {
    if (b !== null && String(a ?? "") !== String(b)) out.push({ kind: "fc_set", section, name, value: String(b) } as Edit);
  };
  const name = after.name?.trim();
  if (name) set("rateprofile_name", before.name, name);
  set("rates_type", before.rates_type.toUpperCase(), after.rates_type.toUpperCase());
  AXES.forEach((ax, i) => {
    const [a, b] = [before.axes[i], after.axes[i]];
    if (!a || !b) return;
    set(`${ax}_rc_rate`, a.rc_rate, b.rc_rate);
    set(`${ax}_srate`, a.srate, b.srate);
    set(`${ax}_expo`, a.expo, b.expo);
    set(`${ax}_rate_limit`, a.rate_limit, b.rate_limit);
  });
  set("thr_mid", before.throttle.mid, after.throttle.mid);
  set("thr_expo", before.throttle.expo, after.throttle.expo);
  set("throttle_limit_type", before.throttle.limit.toUpperCase(), after.throttle.limit.toUpperCase());
  set("throttle_limit_percent", before.throttle.limit_percent, after.throttle.limit_percent);
  return out;
}

/** One sentence for the staged change's title. */
export function editTitle(before: RateProfile, edits: Edit[]): string {
  const label = profileLabel(before);
  const model = edits.find((e) => e.kind === "fc_set" && e.name === "rates_type");
  if (model && model.kind === "fc_set") return `Rate profile ${label}: ${RATES_TYPE[model.value.toLowerCase()] ?? model.value} rates`;
  return `Rate profile ${label}: ${edits.length === 1 ? "1 value" : `${edits.length} values`}`;
}

/** The text of a value for a number field: whole numbers only (the CLI stores whole numbers). */
export const wholeNumber = (text: string, min: number, max: number): number | null => {
  if (text.trim() === "") return null;
  const v = Math.round(Number(text));
  return Number.isFinite(v) ? Math.min(max, Math.max(min, v)) : null;
};
