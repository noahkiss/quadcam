import { describe, expect, it } from "vitest";
import { preview, redraw } from "./ratemath";
import { plan, apply, freshSimSync } from "./simsync";
import * as seed from "./seed";
import type { RateProfile } from "../types";

const profiles = () => seed.rates().profiles as RateProfile[];

describe("the mock's rate math", () => {
  it("redraws the recorded real-core curves for Betaflight, Actual and Quick", () => {
    for (const p of profiles()) {
      const r = redraw(p);
      r.axes.forEach((a, i) => {
        expect(a.curve.length).toBe(p.axes[i].curve.length);
        a.curve.forEach((v, k) => expect(v).toBeCloseTo(p.axes[i].curve[k], 6));
        expect(a.max_deg_s).toBeCloseTo(p.axes[i].max_deg_s, 6);
      });
      r.throttle.curve.forEach((v, k) => expect(v).toBeCloseTo(p.throttle.curve[k], 6));
    }
    expect(profiles().map((p) => p.rates_type)).toEqual(["betaflight", "actual", "quick"]);
  });

  it("raises the maximum when the RC rate goes up", () => {
    const p = structuredClone(profiles()[0]);
    p.axes[0].rc_rate += 20;
    expect(preview(p, null).profile.axes[0].max_deg_s).toBeGreaterThan(profiles()[0].axes[0].max_deg_s + 10);
  });

  it("converts to another model and says how far off it is", () => {
    const actual = profiles()[1];
    const r = preview(actual, "betaflight");
    expect(r.profile.rates_type).toBe("betaflight");
    expect(r.fit_error[0]).toBeGreaterThan(0);
    expect(r.fit_share[0]).toBeLessThan(0.12);
    expect(Number.isInteger(r.profile.axes[0].rc_rate)).toBe(true);
    expect(() => preview(actual, "nope")).toThrow();
  });
});

describe("the mock's sim sync", () => {
  const sync = (over: Partial<Parameters<typeof plan>[1]> = {}) => ({ sims: [{ sim: "uncrashed", file: null, profile: "FREE" }], paths: [], device: null, backup: null, profile: 0, ...over });

  it("plans, refuses while a sim runs, and marks a written profile as matching", () => {
    const st = freshSimSync();
    st.running = [];
    const pl = plan(st, sync({ profile: 1 }));
    expect(pl.checks.every((c) => c.ok)).toBe(true);
    expect(pl.warnings?.join(" ")).toContain("uses the actual model");
    st.running = ["Uncrashed"];
    const refused = plan(st, sync({ profile: 1 }));
    expect(refused.checks.find((c) => !c.ok)?.refusal?.code).toBe("sim_running");
    expect(refused.digest).toBe("");
    st.running = [];
    const r = apply(st, sync({ profile: 1 }), pl.digest, true);
    expect(r.status).toBe("verified");
    expect(plan(st, sync({ profile: 1 })).checks.find((c) => !c.ok)?.refusal?.code).toBe("incompatible");
    expect(() => apply(st, sync({ profile: 1 }), "x", true)).toThrow();
  });
});
