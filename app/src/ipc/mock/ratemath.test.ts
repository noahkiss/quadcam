import { describe, expect, it } from "vitest";
import { redraw } from "./ratemath";
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
});
