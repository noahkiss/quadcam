import { describe, expect, it } from "vitest";
import { seedLibrary } from "../test/seed";
import { fmtBytes, fmtDur, fmtLong, fmtT, parseT } from "./format";
import { albumAction, byDay, daySub, deadOf, flyingOf, matches, nextSort, sortClips, validSort, visibleClips } from "./library";

describe("format", () => {
  it("formats like the legacy UI", () => {
    expect(fmtBytes(0)).toBe("0 B");
    expect(fmtBytes(6_521_519)).toBe("7 MB");
    expect(fmtBytes(31_914_983_424)).toBe("31.9 GB");
    expect(fmtDur(128.4)).toBe("2:08");
    expect(fmtDur(3725)).toBe("1:02:05");
    expect(fmtLong(128)).toBe("2 min");
    expect(fmtLong(3900)).toBe("1 h 5 min");
    expect(fmtT(62.54)).toBe("1:02.5");
    expect(fmtT(62.54, false)).toBe("1:02");
    expect(parseT("1:02.5")).toBe(62.5);
    expect(parseT("12")).toBe(12);
    expect(parseT("")).toBeNull();
    expect(parseT("x")).toBeNull();
  });
});

describe("library rules", () => {
  const lib = seedLibrary();
  it("sorts by date newest first, clips in flown order", () => {
    expect(sortClips(lib.clips, { key: "date", dir: "desc" }).map((c) => c.name)).toEqual(["river-dive", "backyard-loops", "gap-run"]);
    expect(sortClips(lib.clips, { key: "date", dir: "asc" }).map((c) => c.name)).toEqual(["backyard-loops", "gap-run", "river-dive"]);
    expect(sortClips(lib.clips, { key: "rating", dir: "desc" }).map((c) => c.name)).toEqual(["gap-run", "backyard-loops", "river-dive"]);
    expect(sortClips(lib.clips, { key: "name", dir: "asc" }).map((c) => c.name)).toEqual(["backyard-loops", "gap-run", "river-dive"]);
  });

  it("picks a key's natural direction, then reverses", () => {
    expect(nextSort({ key: "date", dir: "desc" }, "name")).toEqual({ key: "name", dir: "asc" });
    expect(nextSort({ key: "name", dir: "asc" }, "name")).toEqual({ key: "name", dir: "desc" });
    expect(validSort(undefined)).toEqual({ key: "date", dir: "desc" });
  });

  it("filters by group, day, place, aircraft and words", () => {
    const names = (f: Parameters<typeof visibleClips>[1], q = "") => visibleClips(lib, f, q, { key: "name", dir: "asc" }).map((c) => c.name);
    expect(names({ group: "picks" })).toEqual(["gap-run"]);
    expect(names({ group: "rejected" })).toEqual(["river-dive"]);
    expect(names({ group: "moments" })).toEqual(["gap-run"]);
    expect(names({ group: "all", day: "2026-09-27" })).toEqual(["backyard-loops", "gap-run"]);
    expect(names({ group: "all", before: "2026-09-28" })).toEqual(["backyard-loops", "gap-run"]);
    expect(names({ group: "all", place: "riverside PARK" })).toEqual(["river-dive"]);
    expect(names({ group: "all", aircraft: "whoop" })).toEqual(["backyard-loops", "gap-run"]);
    expect(names({ group: "all" }, "two packs")).toEqual(["gap-run"]);
    expect(matches(lib.clips[0], { group: "last_import" }, "", lib.last_import)).toBe(true);
  });

  it("works out flying time and dead air from keep ranges", () => {
    const gap = lib.clips.find((c) => c.name === "gap-run")!;
    expect(flyingOf(gap)).toBe(110);
    expect(deadOf(gap)).toEqual([{ start: 50, end: 60 }]);
    expect(deadOf({ keep: [], duration: 5 })).toEqual([]);
  });

  it("groups days and describes them", () => {
    const days = byDay(sortClips(lib.clips, { key: "date", dir: "desc" }));
    expect(days.map(([d, cs]) => [d, cs.length])).toEqual([["2026-09-28", 1], ["2026-09-27", 2]]);
    expect(daySub(days[1][1], fmtLong)).toBe("Home field · Whoop · 2 clips · 2 min flying");
  });

  it("names the album action", () => {
    expect(albumAction("Drone")).toBe("Add to Drone Album");
    expect(albumAction("FPV album", 3)).toBe("Add 3 to FPV album");
    expect(albumAction("")).toBeNull();
  });
});
