import { describe, expect, it } from "vitest";
import { announce } from "./StatusBar";

describe("announce", () => {
  it("says nothing at first: what was plugged in at launch is not news", () => {
    expect(announce(null, [["Radio", "connected"]])).toBe("");
  });

  it("names each group whose state changed, and a group that left", () => {
    const before = new Map([
      ["Radio", "working"],
      ["Quad", "connected"],
    ] as const);
    expect(
      announce(before, [
        ["Radio", "safe"],
        ["DJI", "inserted"],
      ]),
    ).toBe("Radio: Safe to unplug. DJI: Still inserted. Quad: Removed");
  });

  it("is empty when nothing changed", () => {
    const before = new Map([["Radio", "attention"]] as const);
    expect(announce(before, [["Radio", "attention"]])).toBe("");
  });
});
