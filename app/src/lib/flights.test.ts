import { describe, expect, it } from "vitest";
import { hhmm, mmss, num } from "./flights";

describe("flight formatting", () => {
  it("minutes and seconds", () => {
    expect(mmss(60)).toBe("1:00");
    expect(mmss(109.6)).toBe("1:50");
    expect(mmss(null)).toBe("–");
    expect(mmss(-3)).toBe("0:00");
  });
  it("numbers with units", () => {
    expect(num(3.5, 2, "V")).toBe("3.50 V");
    expect(num(43, 0, "%")).toBe("43 %");
    expect(num(null, 2, "V")).toBe("–");
    expect(num(3.3, 2)).toBe("3.30");
  });
  it("time of day", () => {
    expect(hhmm("2026-10-04T10:03:00")).toBe("10:03");
  });
});
