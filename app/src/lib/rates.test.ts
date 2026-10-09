import { describe, expect, it } from "vitest";
import type { RateAxis, SimRates } from "../ipc/types";
import { maxDiff, niceMax, points, profileLabel, simState, simTargets, ticks, worstDiff } from "./rates";

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
