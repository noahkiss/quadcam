import { describe, expect, it } from "vitest";
import { seedLibrary } from "../test/seed";
import type { Moment } from "../ipc/types";
import { fmtOffset, visibleClips } from "./library";
import { bestMoments, heat, heatGradient } from "./moments";

const m = (start: number, score: number, kind: Moment["kind"] = "roll"): Moment => ({ kind, start, end: start + 1, score, source: "radio_log", detail: "" });

describe("moments", () => {
  it("makes a heat strip that peaks at the busiest moment", () => {
    const h = heat([m(10, 0.5), m(90, 1), m(91, 0.8)], 100, 10);
    expect(h).toHaveLength(10);
    expect(Math.max(...h)).toBe(1);
    expect(h.indexOf(1)).toBe(9);
    expect(h[1]).toBeGreaterThan(0);
    expect(h[5]).toBe(0);
    expect(heat([], 100, 10)).toEqual([]);
    expect(heat([m(5, 1, "dead_air")], 100, 10)).toEqual([]);
    expect(heat([m(5, 1)], 0, 10)).toEqual([]);
    // A moment past the end lands in the last bin.
    expect(heat([m(150, 1)], 100, 4).indexOf(1)).toBe(3);
  });

  it("draws the strip as a gradient with one stop per bin", () => {
    expect(heatGradient([])).toBeUndefined();
    const g = heatGradient([0, 1, 0.5])!;
    expect(g).toContain("var(--accent) 0%, transparent) 0.00% 33.33%");
    expect(g).toContain("var(--accent) 65%, transparent) 66.67% 100.00%");
    expect(g).toContain("var(--accent) 100%, transparent) 33.33% 66.67%");
  });

  it("lists the best moments across clips, best first", () => {
    const lib = seedLibrary();
    const best = bestMoments(lib.clips);
    expect(best.map((x) => [x.clip.name, x.moment.kind])).toEqual([
      ["gap-run", "roll"],
      ["gap-run", "flip"],
      ["gap-run", "punch"],
    ]);
    expect(bestMoments(lib.clips, 1)).toHaveLength(1);
    expect(visibleClips(lib, { group: "best_moments" }, "", { key: "name", dir: "asc" }).map((c) => c.name)).toEqual(["gap-run"]);
  });

  it("formats a time shift's offset", () => {
    expect(fmtOffset(0)).toBe("+0:00");
    expect(fmtOffset(66_300)).toBe("+18:25");
    expect(fmtOffset(-300)).toBe("-0:05");
    expect(fmtOffset(3 * 86_400 + 7200)).toBe("+3 d 2:00");
  });
});
