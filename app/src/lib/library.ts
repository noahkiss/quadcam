// Library rules shared by the grid, the list, keys and the menu: filtering, the one sort
// order, day groups, flying time. Ported from the legacy app.js.
import type { LibClip, LibraryView, Span } from "../ipc/types";

export type Group = "all" | "last_import" | "moments" | "picks" | "rejected" | "not_in_photos";

export interface Filter {
  group: Group;
  day?: string;
  /** Days before this one (the sidebar's "Older"). */
  before?: string;
  place?: string;
  aircraft?: string;
}

export type SortKey = "date" | "rating" | "duration" | "name";
export interface Sort {
  key: SortKey;
  dir: "asc" | "desc";
}

/** The direction a key sorts in when first picked. */
export const SORT_FIRST_DIR: Record<SortKey, "asc" | "desc"> = { date: "desc", rating: "desc", duration: "desc", name: "asc" };

export function validSort(s: Partial<Sort> | undefined): Sort {
  return s?.key && SORT_FIRST_DIR[s.key] ? { key: s.key, dir: s.dir === "asc" ? "asc" : "desc" } : { key: "date", dir: "desc" };
}

/** A new key sorts in its natural direction; the same key again reverses it. */
export function nextSort(cur: Sort, key: SortKey): Sort {
  return cur.key === key ? { key, dir: cur.dir === "asc" ? "desc" : "asc" } : { key, dir: SORT_FIRST_DIR[key] };
}

export const sameFilter = (a: Filter, b: Filter) => JSON.stringify(a) === JSON.stringify(b);

export function matches(c: LibClip, f: Filter, query: string, lastImport: string | null): boolean {
  switch (f.group) {
    case "last_import":
      if (!(c.import && c.import === lastImport)) return false;
      break;
    case "moments":
      if (!c.moments.length) return false;
      break;
    case "picks":
      if (c.flag !== "pick") return false;
      break;
    case "rejected":
      if (c.flag !== "reject") return false;
      break;
    case "not_in_photos":
      if (c.in_photos) return false;
      break;
  }
  if (f.day && c.date !== f.day) return false;
  if (f.before && !(c.date < f.before)) return false;
  if (f.place && (c.place || "").toLowerCase() !== f.place.toLowerCase()) return false;
  if (f.aircraft && (c.aircraft || "").toLowerCase() !== f.aircraft.toLowerCase()) return false;
  const q = query.trim().toLowerCase();
  if (q) {
    const hay = [c.name, c.note, c.place, c.aircraft, (c.keywords || []).join(" "), c.dvr, c.path].join(" ").toLowerCase();
    if (!q.split(/\s+/).every((w) => hay.includes(w))) return false;
  }
  return true;
}

/** The one order for the grid, the list and keyboard moves. By date: days newest first
 * (or oldest first), and the clips of a day in the order they were flown. */
export function sortClips(list: LibClip[], sort: Sort): LibClip[] {
  const sign = sort.dir === "asc" ? 1 : -1;
  const flown = (a: LibClip, b: LibClip) => (a.time || "").localeCompare(b.time || "") || a.path.localeCompare(b.path);
  const byDate = (a: LibClip, b: LibClip) => sign * a.date.localeCompare(b.date) || flown(a, b);
  const cmp = {
    date: byDate,
    rating: (a: LibClip, b: LibClip) => sign * ((a.rating || 0) - (b.rating || 0)) || byDate(a, b),
    duration: (a: LibClip, b: LibClip) => sign * (a.duration - b.duration) || byDate(a, b),
    name: (a: LibClip, b: LibClip) => sign * a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }) || byDate(a, b),
  }[sort.key];
  return [...list].sort(cmp);
}

export function visibleClips(lib: LibraryView | null, f: Filter, query: string, sort: Sort): LibClip[] {
  if (!lib) return [];
  return sortClips(lib.clips.filter((c) => matches(c, f, query, lib.last_import)), sort);
}

/** Seconds with a picture: the keep ranges, else the whole clip. */
export const flyingOf = (c: Pick<LibClip, "keep" | "duration">) => (c.keep?.length ? c.keep.reduce((a, k) => a + k.end - k.start, 0) : c.duration);

/** The stretches outside the keep ranges. */
export function deadOf(c: Pick<LibClip, "keep" | "duration">): Span[] {
  if (!c.keep?.length) return [];
  const out: Span[] = [];
  let t = 0;
  for (const k of c.keep) {
    if (k.start - t > 0.05) out.push({ start: t, end: k.start });
    t = k.end;
  }
  if (c.duration - t > 0.05) out.push({ start: t, end: c.duration });
  return out;
}

/** Consecutive clips of one date, in list order. */
export function byDay(list: LibClip[]): [string, LibClip[]][] {
  const m = new Map<string, LibClip[]>();
  for (const c of list) {
    if (!m.has(c.date)) m.set(c.date, []);
    m.get(c.date)!.push(c);
  }
  return [...m];
}

/** "Home field · Whoop · 2 clips · 2 min flying" */
export function daySub(cs: LibClip[], fmtLong: (s: number) => string): string {
  const places = [...new Set(cs.map((c) => c.place).filter(Boolean))];
  const ac = [...new Set(cs.map((c) => c.aircraft).filter(Boolean))];
  const flying = cs.reduce((a, c) => a + flyingOf(c), 0);
  return [...places, ...ac, `${cs.length} clip${cs.length === 1 ? "" : "s"}`, `${fmtLong(flying)} flying`].join(" · ");
}

/** The Photos album action's label ("Add to Drone Album"); null without an album. */
export function albumAction(album: string | undefined | null, n = 1): string | null {
  const a = (album || "").trim();
  if (!a) return null;
  return `Add ${n > 1 ? `${n} ` : ""}to ${a}${/album$/i.test(a) ? "" : " Album"}`;
}

export const thumbPx = (size: number | undefined) => [80, 104, 136, 170, 210][(size || 3) - 1];
