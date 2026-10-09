import { describe, expect, it } from "vitest";
import type { RateAxis, RateProfile, SimRates } from "../ipc/types";
import { MODEL_BOUNDS, MODEL_FIELDS, RATE_MODELS, editTitle, maxDiff, niceMax, points, profileEdits, profileLabel, simState, simTargets, ticks, wholeNumber, worstDiff } from "./rates";

const axis = (curve: number[]): RateAxis => ({ axis: "roll", rc_rate: 1, srate: 1, expo: 1, rate_limit: 1998, max_deg_s: curve.at(-1) ?? 0, center_deg_s: 0, curve });
const sim = (over: Partial<SimRates> = {}): SimRates => ({ id: "liftoff", name: "Liftoff", enabled: true, found: true, running: false, note: null, files: [], in_sync: null, ...over });

describe("profileLabel", () => {
  it("joins the index and the name, or says Profile N", () => {
    expect(profileLabel({ index: 1, name: "RACE" })).toBe("1 RACE");
    expect(profileLabel({ index: 2, name: null })).toBe("Profile 2");
  });
});

describe("chart geometry", () => {
  it("rounds the top up to a step and never below one step", () => {
    expect(niceMax([0, 907])).toBe(1000);
    expect(niceMax([670], 200)).toBe(800);
    expect(niceMax([])).toBe(100);
  });
  it("ticks run from 0 to the top", () => {
    expect(ticks(1000)).toEqual([0, 250, 500, 750, 1000]);
  });
  it("maps a curve into the box, bottom left to top right", () => {
    expect(points([0, 50, 100], { x: 10, y: 0, w: 100, h: 50 }, 100)).toBe("10.0,50.0 60.0,25.0 110.0,0.0");
    expect(points([5], { x: 0, y: 0, w: 1, h: 1 }, 10)).toBe("");
  });
  it("clamps a value above the top", () => {
    expect(points([0, 500], { x: 0, y: 0, w: 10, h: 10 }, 100)).toBe("0.0,10.0 10.0,0.0");
  });
});

describe("maxDiff", () => {
  it("is the largest gap over the shared points", () => {
    expect(maxDiff([0, 10, 20], [0, 14, 17])).toBe(4);
    expect(maxDiff([1, 2], [1, 2, 99])).toBe(0);
  });
});

describe("sims", () => {
  it("lists only profiles QuadCam could read", () => {
    const s = sim({
      files: [
        {
          path: "~/x",
          error: null,
          profiles: [
            { name: "Race", supported: true, note: null, axes: [axis([0, 1])], throttle: null, diff: null },
            { name: "Odd", supported: false, note: "other type", axes: [], throttle: null, diff: null },
          ],
        },
      ],
    });
    expect(simTargets([s]).map((t) => t.label)).toEqual(["Liftoff · Race"]);
  });
  it("states each sim's state", () => {
    expect(simState(sim({ enabled: false }))).toBe("Off");
    expect(simState(sim({ found: false }))).toBe("No rate file");
    expect(simState(sim({ in_sync: true, files: [{ path: "p", error: null, profiles: [] }] }))).toBe("Matches the quad");
    expect(simState(sim({ in_sync: false, files: [{ path: "p", error: null, profiles: [] }] }))).toBe("Differs from the quad");
    expect(simState(sim({ files: [{ path: "p", error: "bad", profiles: [] }] }))).toBe("Cannot read");
  });
  it("takes the worst axis", () => {
    expect(worstDiff({ max_diff: [1, 30, 2] })).toBe(30);
    expect(worstDiff(null)).toBeNull();
  });
});

const profile = (over: Partial<RateProfile> = {}): RateProfile => ({
  index: 1,
  name: "RACE",
  active: false,
  rates_type: "betaflight",
  complete: true,
  axes: ["roll", "pitch", "yaw"].map((a) => ({ axis: a, rc_rate: 100, srate: 70, expo: 0, rate_limit: 1998, max_deg_s: 667, center_deg_s: 200, curve: [0, 667] })),
  throttle: { mid: 50, expo: 0, hover: null, limit: "off", limit_percent: 100, curve: [0, 1] },
  ...over,
});

describe("profileEdits", () => {
  it("stages only the values that changed, in the rate profile section", () => {
    const a = profile();
    const b = profile();
    b.axes[0].rc_rate = 127;
    b.axes[2].expo = 20;
    b.throttle.mid = 40;
    expect(profileEdits(a, b)).toEqual([
      { kind: "fc_set", section: { kind: "rate_profile", index: 1 }, name: "roll_rc_rate", value: "127" },
      { kind: "fc_set", section: { kind: "rate_profile", index: 1 }, name: "yaw_expo", value: "20" },
      { kind: "fc_set", section: { kind: "rate_profile", index: 1 }, name: "thr_mid", value: "40" },
    ]);
    expect(profileEdits(a, profile())).toEqual([]);
  });
  it("writes the model and the name in the CLI's words", () => {
    const e = profileEdits(profile(), profile({ rates_type: "actual", name: "CINE" }));
    expect(e.map((x) => (x.kind === "fc_set" ? [x.name, x.value] : []))).toEqual([
      ["rateprofile_name", "CINE"],
      ["rates_type", "ACTUAL"],
    ]);
  });
  it("leaves an empty name alone and writes a limit in capitals", () => {
    const b = profile({ name: "  " });
    b.throttle.limit = "scale";
    expect(profileEdits(profile(), b).map((x) => (x.kind === "fc_set" ? x.name : ""))).toEqual(["throttle_limit_type"]);
  });
  it("titles a change by what it does", () => {
    const a = profile();
    expect(editTitle(a, profileEdits(a, profile({ rates_type: "quick" })))).toBe("Rate profile 1 RACE: Quick rates");
    const b = profile();
    b.axes[0].expo = 5;
    expect(editTitle(a, profileEdits(a, b))).toBe("Rate profile 1 RACE: 1 value");
  });
});

describe("model tables", () => {
  it("name and bound the three numbers of every model", () => {
    for (const m of RATE_MODELS) {
      expect(MODEL_FIELDS[m]).toHaveLength(3);
      expect(MODEL_BOUNDS[m].every(([lo, hi]) => lo <= hi)).toBe(true);
    }
  });
  it("rounds to a whole number inside the range, and refuses a non-number", () => {
    expect(wholeNumber("127.6", 1, 255)).toBe(128);
    expect(wholeNumber("999", 1, 255)).toBe(255);
    expect(wholeNumber("-3", 0, 100)).toBe(0);
    expect(wholeNumber("", 0, 100)).toBeNull();
    expect(wholeNumber("abc", 0, 100)).toBeNull();
  });
});
