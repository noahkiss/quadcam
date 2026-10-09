// The mock core's rate curves: a TypeScript copy of `quadcam_sim::rates` (the one
// implementation lives in Rust), so the Rates editor redraws in the browser mock. A Vitest
// spec holds it to the recorded real-core curves (`ratemath.test.ts`). Not shipped: the app
// asks the core (`gear_rates_preview`).
import type { RateAxis, RateProfile, RateThrottle, RatesPreview } from "../types";

const MAX_RATE = 1998;
const STEPS = 50;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

interface Axis {
  rc_rate: number;
  srate: number;
  expo: number;
}

/** Deg/s at stick `x` (-1 to 1), before the axis limit. */
export function rate(model: string, p: Axis, x: number): number {
  x = clamp(x, -1, 1);
  const ax = Math.abs(x);
  let r: number;
  switch (model) {
    case "betaflight": {
      let rc = p.rc_rate / 100;
      if (rc > 2) rc += 14.54 * (rc - 2);
      const e = p.expo / 100;
      const cmd = e !== 0 ? x * ax ** 3 * e + x * (1 - e) : x;
      r = 200 * rc * cmd;
      if (p.srate !== 0) r /= clamp(1 - (ax * p.srate) / 100, 0.01, 1);
      break;
    }
    case "actual": {
      const center = p.rc_rate * 10;
      const max = p.srate * 10;
      const e = p.expo / 100;
      const expof = ax * (x ** 5 * e + x * (1 - e));
      r = x * center + Math.max(max - center, 0) * expof;
      break;
    }
    case "quick": {
      const center = p.rc_rate * 2;
      if (center <= 0) return 0;
      const max = Math.max(p.srate * 10, center);
      const e = p.expo / 100;
      const ratio = max / center;
      const superCfg = (ratio - 1) / ratio;
      const curve = ax ** 3 * e + ax * (1 - e);
      r = x * center * (1 / clamp(1 - curve * superCfg, 0.01, 1));
      break;
    }
    case "raceflight": {
      const rt = p.rc_rate * 10;
      const cmd = (1 + 0.01 * p.expo * (x * x - 1)) * x;
      r = cmd * (rt + Math.abs(cmd) * rt * p.srate * 0.01);
      break;
    }
    default: {
      // kiss
      const rc = p.rc_rate / 100;
      const curve = p.expo / 100;
      const use = 1 / clamp(1 - (ax * p.srate) / 100, 0.01, 1);
      r = 2000 * use * ((x ** 3 * curve + x * (1 - curve)) * (rc / 10));
    }
  }
  return clamp(r, -MAX_RATE, MAX_RATE);
}

const at = (model: string, p: Axis, limit: number, x: number) => clamp(rate(model, p, x), -Math.abs(limit), Math.abs(limit));
const sample = (model: string, p: Axis, limit: number) => Array.from({ length: STEPS + 1 }, (_, i) => at(model, p, limit, i / STEPS));

/** Throttle output (0-1) for a stick position (0-1). */
export function throttleAt(t: RateThrottle, stick: number): number {
  stick = clamp(stick, 0, 1);
  const mid = clamp(t.mid / 100, 0.01, 0.99);
  const hover = clamp((t.hover ?? t.mid) / 100, 0, 1);
  const e = clamp(t.expo / 100, 0, 1);
  const g = (x: number) => x * (1 - e + e * x * x);
  const out = stick < mid ? hover + hover * g((stick - mid) / mid) : hover + (1 - hover) * g((stick - mid) / (1 - mid));
  const lim = clamp(t.limit_percent / 100, 0, 1);
  return t.limit === "scale" ? out * lim : t.limit === "clip" ? Math.min(out, lim) : out;
}

const axisView = (model: string, a: RateAxis): RateAxis => ({
  ...a,
  max_deg_s: at(model, a, a.rate_limit, 1),
  center_deg_s: rate(model, a, 1e-4) / 1e-4,
  curve: sample(model, a, a.rate_limit),
});

/** The profile with its curves drawn again from its numbers. */
export function redraw(p: RateProfile): RateProfile {
  return {
    ...p,
    axes: p.axes.map((a) => axisView(p.rates_type, a)),
    throttle: { ...p.throttle, curve: Array.from({ length: STEPS + 1 }, (_, i) => throttleAt(p.throttle, i / STEPS)) },
  };
}

const BOUNDS: Record<string, [[number, number], [number, number], [number, number]]> = {
  betaflight: [[1, 255], [0, 99], [0, 100]],
  actual: [[1, 200], [1, 200], [0, 100]],
  quick: [[1, 255], [1, 200], [0, 100]],
  raceflight: [[1, 255], [0, 255], [0, 100]],
  kiss: [[1, 255], [0, 99], [0, 100]],
};

/** A coarse search for the settings of `to` closest to an axis (the real core runs a
 * least-squares fit; this only has to look right in the mock). */
function fitAxis(from: string, a: RateAxis, to: string): { v: Axis; max: number } {
  const xs = Array.from({ length: 101 }, (_, i) => i / 100);
  const target = xs.map((x) => at(from, a, a.rate_limit, x));
  const [rcB, srB, exB] = BOUNDS[to];
  let best = { v: { rc_rate: rcB[0], srate: srB[0], expo: exB[0] }, cost: Infinity };
  for (let rc = rcB[0]; rc <= rcB[1]; rc += 4) {
    for (let sr = srB[0]; sr <= srB[1]; sr += 4) {
      for (let ex = exB[0]; ex <= exB[1]; ex += 10) {
        const v = { rc_rate: rc, srate: sr, expo: ex };
        const cost = xs.reduce((s, x, i) => s + (at(to, v, a.rate_limit, x) - target[i]) ** 2, 0);
        if (cost < best.cost) best = { v, cost };
      }
    }
  }
  const max = Math.max(...xs.map((x, i) => Math.abs(at(to, best.v, a.rate_limit, x) - target[i])));
  return { v: best.v, max };
}

/** `Core::gear_rates_preview`. */
export function preview(profile: RateProfile, to: string | null): RatesPreview {
  if (profile.axes.length !== 3) throw "A rate profile has roll, pitch and yaw, and every value is a number.";
  if (!BOUNDS[profile.rates_type]) throw `${JSON.stringify(profile.rates_type)} is not a rate model (betaflight, actual, quick, raceflight, kiss).`;
  if (!to) return { profile: redraw(profile), fit_error: [0, 0, 0], fit_share: [0, 0, 0] };
  if (!BOUNDS[to]) throw `${JSON.stringify(to)} is not a rate model.`;
  const fits = profile.axes.map((a) => (profile.rates_type === to ? { v: a, max: 0 } : fitAxis(profile.rates_type, a, to)));
  const next: RateProfile = { ...profile, rates_type: to, axes: profile.axes.map((a, i) => ({ ...a, rc_rate: fits[i].v.rc_rate, srate: fits[i].v.srate, expo: fits[i].v.expo })) };
  return {
    profile: redraw(next),
    fit_error: fits.map((f) => f.max),
    fit_share: fits.map((f, i) => f.max / Math.max(1, at(profile.rates_type, profile.axes[i], profile.axes[i].rate_limit, 1))),
  };
}
