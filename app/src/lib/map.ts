// The Map page's pins: one per place (its clips counted on it), one per saved place without
// clips, and one per clip that has a location but no place. Pure, so Vitest covers it.
import type { MapPin } from "../bindings";
import type { LibClip, Place } from "../ipc/types";

export type { MapPin };

export interface MapData {
  pins: MapPin[];
  /** Clips on the map, through a place or on their own. */
  mapped: number;
  /** Clips with no location and no saved place to put them at. */
  unmapped: number;
}

const placeKey = (name: string) => name.trim().toLowerCase();
const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
const valid = (lat: number | null | undefined, lon: number | null | undefined): lat is number => lat != null && lon != null && Number.isFinite(lat) && Number.isFinite(lon) && Math.abs(lat) <= 90 && Math.abs(lon) <= 180;

/** The pin id of a place, and of a clip. `pickOf` reads them back. */
export const placePin = (name: string) => `place:${name}`;
export const clipPin = (id: string) => `clip:${id}`;

export function pickOf(id: string): { place: string } | { clip: string } | null {
  if (id.startsWith("place:")) return { place: id.slice(6) };
  if (id.startsWith("clip:")) return { clip: id.slice(5) };
  return null;
}

/** Pins for the clips and the saved places. A place sits at its saved point, else at the
 *  middle of its clips' locations. Names match without regard to case, as the library's place
 *  filter does. */
export function mapData(clips: LibClip[], places: Place[]): MapData {
  const saved = new Map<string, Place>();
  for (const p of places) if (p.name.trim()) saved.set(placeKey(p.name), p);
  const groups = new Map<string, { name: string; clips: LibClip[] }>();
  const loose: MapPin[] = [];
  let unmapped = 0;
  for (const c of clips) {
    const name = c.place?.trim();
    if (name) {
      const k = placeKey(name);
      const g = groups.get(k) ?? { name: saved.get(k)?.name ?? name, clips: [] };
      g.clips.push(c);
      groups.set(k, g);
    } else if (c.location && valid(c.location.lat, c.location.lon)) {
      loose.push({ id: clipPin(c.id), kind: "clip", title: c.name || c.date, subtitle: c.name ? c.date : null, lat: c.location.lat, lon: c.location.lon!, glyph: null });
    } else unmapped++;
  }
  const pins: MapPin[] = [];
  let mapped = loose.length;
  for (const [k, g] of groups) {
    const p = saved.get(k);
    let at: [number, number] | null = p && valid(p.lat, p.lon) ? [p.lat, p.lon!] : null;
    if (!at) {
      const pts = g.clips.map((c) => c.location).filter((l) => l && valid(l.lat, l.lon)) as { lat: number; lon: number }[];
      if (pts.length) at = [pts.reduce((a, l) => a + l.lat, 0) / pts.length, pts.reduce((a, l) => a + l.lon, 0) / pts.length];
    }
    if (!at) {
      unmapped += g.clips.length;
      continue;
    }
    mapped += g.clips.length;
    pins.push({ id: placePin(g.name), kind: "place", title: g.name, subtitle: plural(g.clips.length, "clip"), lat: at[0], lon: at[1], glyph: String(g.clips.length) });
  }
  for (const [k, p] of saved) {
    if (groups.has(k) || !valid(p.lat, p.lon)) continue;
    pins.push({ id: placePin(p.name), kind: "empty", title: p.name, subtitle: "No clips", lat: p.lat, lon: p.lon!, glyph: null });
  }
  pins.sort((a, b) => a.title.localeCompare(b.title));
  return { pins: [...pins, ...loose], mapped, unmapped };
}

/** The page's subtitle: places, clips on the map, clips without a location. */
export function mapSummary(d: MapData): string {
  const places = d.pins.filter((p) => p.kind !== "clip").length;
  const parts = [plural(places, "place"), `${plural(d.mapped, "clip")} on the map`];
  if (d.unmapped) parts.push(`${d.unmapped} without a location`);
  return parts.join(" · ");
}
