import { describe, expect, it } from "vitest";
import type { CheckResult } from "../../../ipc/types";
import { fmtValue, passed } from "./ProfilesPage";

describe("fit values", () => {
  it("prints four places, or an exponent for very small or large values", () => {
    expect(fmtValue(0.0261)).toBe("0.0261");
    expect(fmtValue(1.18)).toBe("1.18");
    expect(fmtValue(2.05e-8)).toBe("2.050e-8");
    expect(fmtValue(250000)).toBe("2.500e+5");
    expect(fmtValue(0)).toBe("0");
    expect(fmtValue(null)).toBe("–");
  });

  it("counts passed checks against the checks the logs had a window for", () => {
    const c = (outcome: CheckResult["outcome"]) => ({ outcome }) as CheckResult;
    expect(passed({ profile: "p", logs: [], checks: [c("pass"), c("fail"), c("not_in_log"), c("pass")] })).toEqual([2, 3]);
  });
});
