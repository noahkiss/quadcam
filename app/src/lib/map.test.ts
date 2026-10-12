import { describe, expect, it } from "vitest";
import type { LibClip } from "../ipc/types";
import { clipPin, mapData, mapSummary, pickOf, placePin } from "./map";

const clip = (id: string, over: Partial<LibClip> = {}) => ({ id, name: "", date: "2026-09-28", place: null, location: null, ...over }) as LibClip;
const at = (lat: number, lon: number, name: string | null = null) => ({ lat, lon, name });

describe("map pins", () => {
  const places = [
    { name: "Field", lat: 37.77, lon: -122.45 },
    { name: "Beach", lat: 37.5, lon: -122.5 },
  ];

  it("puts a place's clips on one pin at the saved point, with the count", () => {
    const d = mapData([clip("a", { place: "Field", location: at(37.7, -122.4) }), clip("b", { place: "field" })], places);
    const field = d.pins.find((p) => p.id === placePin("Field"))!;
    expect(field).toMatchObject({ kind: "place", lat: 37.77, lon: -122.45, glyph: "2", subtitle: "2 clips" });
    expect(d.mapped).toBe(2);
  });

  it("shows a saved place without clips as an empty pin", () => {
    const d = mapData([], places);
    expect(d.pins.map((p) => [p.title, p.kind])).toEqual([
      ["Beach", "empty"],
      ["Field", "empty"],
    ]);
  });

  it("puts an unsaved place at the middle of its clips' locations", () => {
    const d = mapData([clip("a", { place: "Park", location: at(10, 20) }), clip("b", { place: "Park", location: at(12, 22) })], []);
    expect(d.pins).toEqual([expect.objectContaining({ id: placePin("Park"), lat: 11, lon: 21, glyph: "2" })]);
  });

  it("gives a clip with a location but no place its own pin", () => {
    const d = mapData([clip("c1", { name: "loops", location: at(1, 2) })], []);
    expect(d.pins).toEqual([{ id: clipPin("c1"), kind: "clip", title: "loops", subtitle: "2026-09-28", lat: 1, lon: 2, glyph: null, first: "2026-09-28", last: "2026-09-28", time: null, sky: null, is_day: null, thumb: null }]);
  });

  it("gives a clip's pin what its callout shows: day, time, sky and poster frame", () => {
    const weather = { sky: "rain", is_day: false } as LibClip["weather"];
    const d = mapData([clip("c1", { name: "loops", time: "18:30", weather, poster: "/lib/p.jpg", location: at(1, 2) })], []);
    expect(d.pins[0]).toMatchObject({ first: "2026-09-28", last: "2026-09-28", time: "18:30", sky: "rain", is_day: false, thumb: "/lib/p.jpg" });
  });

  it("gives a place's pin its clip count and its first and last day", () => {
    const d = mapData([clip("a", { place: "Field", date: "2026-09-28" }), clip("b", { place: "Field", date: "2026-08-03" }), clip("c", { place: "Field", date: "2026-09-01" })], places);
    expect(d.pins.find((p) => p.id === placePin("Field"))).toMatchObject({ count: 3, first: "2026-08-03", last: "2026-09-28" });
    expect(d.pins.find((p) => p.id === placePin("Beach"))).toMatchObject({ kind: "empty", count: 0 });
  });

  it("counts clips it cannot place", () => {
    const d = mapData([clip("a"), clip("b", { place: "Nowhere" }), clip("c", { location: at(NaN, 2) }), clip("d", { location: at(95, 2) })], []);
    expect(d.pins).toEqual([]);
    expect(d).toMatchObject({ mapped: 0, unmapped: 4 });
  });

  it("skips a saved place without a valid point", () => {
    expect(mapData([], [{ name: "Broken", lat: null as unknown as number, lon: 2 }]).pins).toEqual([]);
  });

  it("reads a pin id back", () => {
    expect(pickOf(placePin("Field: north"))).toEqual({ place: "Field: north" });
    expect(pickOf(clipPin("abc"))).toEqual({ clip: "abc" });
    expect(pickOf("other")).toBeNull();
  });

  it("sums the page up", () => {
    const d = mapData([clip("a", { place: "Field" }), clip("b")], places);
    expect(mapSummary(d)).toBe("2 places · 1 clip on the map · 1 without a location");
  });
});
