import { describe, expect, it } from "vitest";
import type { Moment } from "../../ipc/types";
import { dragPoint, frameMoment, rulerStep, withCut, withKeep, without, type TrimCut } from "./model";

const roll: Moment = { kind: "roll", start: 20, end: 21.5, score: 0.9, source: "radio_log", detail: "" };
const cuts: TrimCut[] = [{ start: 10, end: 25, state: "saved", file: "a_cut1.mp4" }];

describe("trim rules", () => {
  it("frames a moment a second either side, dead air exactly", () => {
    expect(frameMoment(roll, 120)).toEqual({ in: 19, out: 22.5 });
    expect(frameMoment({ ...roll, kind: "dead_air", start: 50, end: 60 }, 55)).toEqual({ in: 50, out: 55 });
    expect(frameMoment({ ...roll, start: 0.4 }, 120).in).toBe(0);
  });

  it("adds a cut only when the points are 0.5 s apart", () => {
    expect(withCut(cuts, { in: 30, out: 40 })).toEqual([{ start: 10, end: 25 }, { start: 30, end: 40 }]);
    expect(withCut(cuts, { in: 30, out: 30.2 })).toMatch(/0.5 s apart/);
    expect(withCut(cuts, { in: null, out: 3 })).toMatch(/in and an out/);
  });

  it("removes a cut and adds keep ranges that are not cuts", () => {
    expect(without(cuts, 0)).toEqual([]);
    expect(withKeep(cuts, [{ start: 10.01, end: 25.02 }, { start: 60, end: 120 }])).toEqual([{ start: 10, end: 25 }, { start: 60, end: 120 }]);
  });

  it("keeps a dragged point inside the clip and away from the other", () => {
    expect(dragPoint({ in: 10, out: 20 }, "in", 25, 120)).toEqual({ in: 19.9, out: 20 });
    expect(dragPoint({ in: 10, out: 20 }, "out", 500, 120)).toEqual({ in: 10, out: 120 });
    expect(dragPoint({ in: 10, out: 20 }, "in", -3, 120)).toEqual({ in: 0, out: 20 });
  });

  it("picks about eight ruler ticks", () => {
    expect(rulerStep(5)).toBe(1);
    expect(rulerStep(120)).toBe(15);
    expect(rulerStep(3600)).toBe(600);
  });
});
