import { describe, expect, it } from "vitest";
import { hint } from "./secret";

describe("the mock's secret hint follows the core's rule", () => {
  it("shows 4 at each end from 16 characters, a quarter below, only the length under 4", () => {
    expect(hint("0123456789abcdef").hint).toBe("0123…cdef (16 chars)");
    expect(hint("0123456789abcde").hint).toBe("012…cde (15 chars)");
    expect(hint("abcd").hint).toBe("a…d (4 chars)");
    expect(hint("abc")).toEqual({ hint: "(3 chars)", prefix: "", suffix: "", length: 3 });
  });
  it("never shows more than half", () => {
    for (let n = 0; n < 60; n++) {
      const h = hint("x".repeat(n));
      expect(2 * (h.prefix.length + h.suffix.length)).toBeLessThanOrEqual(n);
    }
  });
});
