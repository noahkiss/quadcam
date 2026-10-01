import { describe, expect, it } from "vitest";
import type * as G from "../bindings";
import { cutChange, libClip, location, moments, plan, spans } from "./normalize";

describe("normalize", () => {
  it("drops ranges and points without a time", () => {
    expect(spans([{ start: 1, end: 2 }, { start: null, end: 3 }, { start: 4, end: null }])).toEqual([{ start: 1, end: 2 }]);
    expect(moments([{ kind: "roll", start: 1, end: 2, score: null, source: "radio_log", detail: "" }, { kind: "flip", start: null, end: 1, score: 0.5, source: "radio_log", detail: "" }])).toEqual([
      { kind: "roll", start: 1, end: 2, score: 0, source: "radio_log", detail: "" },
    ]);
    expect(location({ lat: null, lon: 2 })).toBeNull();
    expect(location({ lat: 1, lon: 2, name: "Field" })).toEqual({ lat: 1, lon: 2, name: "Field" });
  });

  it("fills a library clip's omitted fields", () => {
    const c = libClip({ id: "a", path: "x.mp4", title: "", note: "", date: "2026-09-27", duration: null, size: 1, name: "x", file: "/x.mp4", strip: null, poster: null, no_picture: false, last_import: false } as G.LibItem);
    expect(c).toMatchObject({ duration: 0, rating: 0, flag: "none", keywords: [], moments: [], keep: [], cuts: [], pending_cuts: [], in_photos: false, location: null, cut_of: null });
  });

  it("fills a plan's suggestion marks and meta", () => {
    const p = plan({ id: 0, skip: false, date: "2026-09-27", time: null, source: "import", badge: "unmatched", segments: 0, name: "", note: "", suggested: { date: false, name: true, note: false, skip: false } } as G.ClipPlan);
    expect(p.suggested).toEqual({ date: false, name: true, note: false, skip: false, cuts: false, meta: false });
    expect(p.meta).toEqual({ profile: null, location: null, keywords: [], author: null });
    expect(p.log_offset_s).toBe(0);
  });

  it("defaults the kept and trashed lists of an applied cut change", () => {
    expect(cutChange({ status: "applied", cuts: [{ start: 1, end: 2 }] })).toEqual({ status: "applied", cuts: [{ start: 1, end: 2 }], kept: [], trashed: [] });
    expect(cutChange({ status: "confirm", files: ["a"] })).toEqual({ status: "confirm", files: ["a"] });
  });
});
