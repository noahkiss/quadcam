// The mock core's sim profiles (`core/sim_profiles.rs`): the built-in presets, the fitted
// profiles a save wrote, and a canned fit whose roll gains the guard drops, so the Profiles
// section shows a kept and a dropped group.
import type { CheckResult, FitValue, SimFit, SimFitParams, SimPreset, SimProfileInfo, ValidationReport } from "../types";

const PRESETS = [
  { id: "meteor75", label: "75 mm whoop", wheelbase_mm: 75, mass_g: 29 },
  { id: "air65ii", label: "65 mm whoop", wheelbase_mm: 65, mass_g: 22 },
  { id: "five_inch", label: "5 inch freestyle", wheelbase_mm: 220, mass_g: 650 },
  { id: "seven_inch", label: "7 inch long range", wheelbase_mm: 300, mass_g: 900 },
];

export interface MockSimFits {
  /** Fitted profiles by id, as `gear_sim_fit_save` wrote them. */
  saved: SimProfileInfo[];
  /** Fit calls taken, for specs. */
  fits: SimFitParams[];
}

export const freshSimFits = (): MockSimFits => ({ saved: [], fits: [] });

const idFrom = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

const val = (name: string, unit: string, before: number, value: number, samples: number): FitValue => ({
  name,
  unit,
  before,
  value,
  source: "fitted",
  residual: null,
  residual_unit: "",
  samples,
  note: null,
});

const check = (c: string, outcome: CheckResult["outcome"]): CheckResult => ({
  check: c,
  measure: "command",
  log: "pull 1",
  window: [10, 20],
  logged: 0.3,
  sim: 0.31,
  band: 0.05,
  relative: false,
  unit: "",
  outcome,
});

const report = (profile: string, fails: string[]): ValidationReport => ({
  profile,
  logs: ["pull 1"],
  checks: ["hover", "punch", "sag", "roll", "pitch", "yaw"].map((c) => check(c, fails.includes(c) ? "fail" : "pass")),
});

/** A digest that changes with the fit's inputs, as the core's does with the profile. */
const digest = (p: SimFitParams) => {
  let h = 0;
  for (const ch of `${p.base}|${p.aircraft ?? ""}|${p.mass_g ?? ""}`) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h.toString(16).padStart(8, "0");
};

/** `gear_sim_profiles`: the presets, then the fitted profiles. */
export function list(st: MockSimFits): SimProfileInfo[] {
  return [...PRESETS.map((p) => ({ id: p.id, label: p.label, kind: "preset", base: null, aircraft: null, fitted_at: null, mass_g: p.mass_g, wheelbase_mm: p.wheelbase_mm })), ...st.saved];
}

/** The Sim page's Aircraft menu entries for the fitted profiles. */
export function presets(st: MockSimFits): SimPreset[] {
  return st.saved.map((p) => ({ id: p.id, label: p.label, wheelbase_mm: p.wheelbase_mm, mass_g: p.mass_g, fitted: true }));
}

/** `gear_sim_fit`: writes nothing. Refuses an unknown base and an aircraft with no FC. */
export function fit(st: MockSimFits, p: SimFitParams, aircraft: string[]): SimFit {
  st.fits.push(p);
  const base = PRESETS.find((x) => x.id === p.base);
  if (!base) throw `The fit starts from a built-in profile: ${PRESETS.map((x) => x.id).join(", ")}. Not "${p.base}".`;
  const a = p.aircraft?.trim();
  if (!a || !aircraft.includes(a)) throw `No FC is linked to the aircraft "${a ?? ""}" (gear devices); link it, or pass logs.`;
  const id = p.id || idFrom(a);
  const power = [val("prop.kt", "N/(rad/s)²", 1.9e-8, 2.05e-8, 412), val("battery.resistance", "Ω", 0.026, 0.031, 96)];
  const roll = [val("fc.gains.roll.p", "", 1, 1.18, 14)];
  const pitch = [val("fc.gains.pitch.p", "", 1, 0.94, 12)];
  const dropped = `the fitted roll gains validate worse than ${base.id}'s, so ${base.id}'s are kept`;
  return {
    report: {
      profile_id: id,
      base: base.id,
      logs: ["pull 1", "pull 2"],
      values: [...power, ...pitch],
      measured: [val("pack_resistance_radio", "Ω", 0.026, 0.03, 1180)],
      groups: [
        { name: "power", failed: 0, base_failed: 1, kept: true, values: power, note: null },
        { name: "roll", failed: 2, base_failed: 1, kept: false, values: roll, note: dropped },
        { name: "pitch", failed: 0, base_failed: 0, kept: true, values: pitch, note: null },
      ],
      thrust_curve: [],
      estimates: ["frame.cda, duct.c_duct: blackbox logs carry no ground speed, so there are no coast-downs to fit drag to"],
      notes: [dropped],
    },
    validation: report(id, ["roll"]),
    base_validation: report(base.id, ["sag", "roll"]),
    sources: ["pull 1", "pull 2", "backup fc-1/2026-10-07T101500-manual (diff all)", `radio log ${a}-2026-10-05-120000.csv (3 flights)`],
    replaces: st.saved.some((s) => s.id === id),
    digest: digest(p),
  };
}

/** `gear_sim_fit_save`: only with `confirm` and the fit's digest. */
export function save(st: MockSimFits, p: { fit: SimFitParams; digest: string; confirm?: boolean }, aircraft: string[]): SimProfileInfo {
  if (!p.confirm) throw "Saving a fitted sim profile needs confirm=true, after the person saw the fit.";
  const f = fit(st, p.fit, aircraft);
  if (f.digest !== p.digest) throw "The fit changed since the digest was given (new logs or settings): fit again and check it.";
  const base = PRESETS.find((x) => x.id === p.fit.base)!;
  const info: SimProfileInfo = {
    id: f.report.profile_id,
    label: p.fit.label || p.fit.aircraft || `${base.label} (fitted)`,
    kind: "fitted",
    base: base.id,
    aircraft: p.fit.aircraft ?? null,
    fitted_at: "2026-10-09T12:00:00Z",
    mass_g: p.fit.mass_g ?? base.mass_g,
    wheelbase_mm: base.wheelbase_mm,
  };
  st.saved = [...st.saved.filter((s) => s.id !== info.id), info];
  return info;
}

/** `gear_sim_profile_delete`: false when there was none. */
export function remove(st: MockSimFits, p: { id: string; confirm?: boolean }): boolean {
  if (!p.confirm) throw "Deleting a fitted sim profile needs confirm=true, after the person agreed.";
  const had = st.saved.some((s) => s.id === p.id);
  st.saved = st.saved.filter((s) => s.id !== p.id);
  return had;
}
