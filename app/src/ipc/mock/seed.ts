// Starting states for the mock core, built from the JSON the real core wrote
// (scripts/make-fixtures.sh). The recorded library has no radio-log data and no ratings,
// so `richLibrary` adds moments, flight stats, keep ranges, ratings and flags by hand.
import libraryFixture from "../../../e2e/fixtures/library.json";
import reviewFixture from "../../../e2e/fixtures/session-review.json";
import finishedFixture from "../../../e2e/fixtures/session-finished.json";
import settingsFixture from "../../../e2e/fixtures/settings.json";
import type { LibClip, LibraryView, Session, SettingsView, Volume } from "../types";

export const HOME = "/Users/pilot";
export const CACHE = `${HOME}/Library/Caches/app.quadcam`;
export const LIBRARY_ROOT = `${HOME}/Movies/quadcam`;
export const PREVIEW = `${CACHE}/previews/preview.mp4`;

const clone = <T>(v: T): T => structuredClone(v);

export function stripOf(c: Pick<LibClip, "path">): string {
  const stem = c.path.split("/").pop()!.replace(/\.[^.]+$/, "");
  return `${CACHE}/strips/${stem}-strip.jpg`;
}

/** The recorded library with radio-log data, ratings and flags added. */
export function richLibrary(): LibraryView {
  const lib = clone(libraryFixture) as unknown as LibraryView;
  for (const c of lib.clips) c.strip = stripOf(c);
  const by = (name: string) => lib.clips.find((c) => c.name === name)!;
  const gap = by("gap-run");
  gap.rating = 4;
  gap.flag = "pick";
  gap.keep = [{ start: 0, end: 50 }, { start: 60, end: 120 }];
  gap.moments = [
    { kind: "roll", start: 20, end: 21.5, score: 0.9, source: "radio_log", detail: "full aileron 1.5 s" },
    { kind: "flip", start: 45, end: 46, score: 0.8, source: "radio_log", detail: "full elevator 1.0 s" },
    { kind: "punch", start: 70, end: 71.2, score: 0.4, source: "radio_log", detail: "throttle 10 % to 95 % in 0.3 s" },
  ];
  gap.stats = { armed_s: 110, packs: 2, min_rx_bat_v: 3.42, min_lq: 71, min_rssi_db: -96, max_throttle: 0.98, pack_spans: [{ start: 1, end: 50 }, { start: 60, end: 118 }] };
  const river = by("river-dive");
  river.flag = "reject";
  by("backyard-loops").rating = 2;
  return recount(lib);
}

/** Groups and totals the core would compute for `lib.clips`. */
export function recount(lib: LibraryView): LibraryView {
  const count = (key: (c: LibClip) => string | null | undefined) => {
    const m = new Map<string, number>();
    for (const c of lib.clips) {
      const k = key(c);
      if (k) m.set(k, (m.get(k) || 0) + 1);
    }
    return [...m];
  };
  const byCount = (a: [string, number], b: [string, number]) => b[1] - a[1] || a[0].localeCompare(b[0]);
  lib.groups = {
    days: count((c) => c.date).sort((a, b) => b[0].localeCompare(a[0])),
    aircraft: count((c) => c.aircraft).sort(byCount),
    places: count((c) => c.place).sort(byCount),
  };
  const flying = (c: LibClip) => (c.keep.length ? c.keep.reduce((a, k) => a + k.end - k.start, 0) : c.duration);
  lib.totals = {
    clips: lib.clips.length,
    bytes: lib.clips.reduce((a, c) => a + c.size + c.cuts.reduce((x, k) => x + k.size, 0), 0),
    seconds: lib.clips.reduce((a, c) => a + c.duration, 0),
    flying: lib.clips.reduce((a, c) => a + flying(c), 0),
  };
  for (const c of lib.clips) c.last_import = !!c.import && c.import === lib.last_import;
  lib.clips.sort((a, b) => b.date.localeCompare(a.date) || (a.time || "").localeCompare(b.time || "") || a.path.localeCompare(b.path));
  return lib;
}

export function emptyLibrary(): LibraryView {
  const lib = clone(libraryFixture) as unknown as LibraryView;
  lib.clips = [];
  lib.exists = false;
  lib.last_import = null;
  return recount(lib);
}

export function reviewSession(source = "/Volumes/DVR"): Session {
  const s = clone(reviewFixture) as unknown as Session;
  s.source = source;
  return s;
}

/** The review session with PICT0001 and PICT0002 as one recording the DVR split, joined, and
 * a radio log that matched it with two packs. */
export function joinedSession(): Session {
  const s = reviewSession();
  const [a, b] = s.clips;
  a.join = { parts: [b.id], list: `${dirOf(a.staged!)}/PICT0001.joined.ffconcat`, on: true, bytes: a.size + b.size, swap: { duration: a.duration, probe: a.probe, signal: a.signal, detail: a.detail } };
  a.duration = a.duration + b.duration;
  a.detail = "2 files, one recording, 2:05";
  b.part_of = a.id;
  const p = s.plans[0];
  p.badge = "matched";
  p.segments = 2;
  p.flight = { armed_s: 100, packs: 2, min_rx_bat_v: 3.6, min_lq: 90, min_rssi_db: -80, max_throttle: 0.9, pack_spans: [{ start: 2, end: 60 }, { start: 70, end: 112 }] };
  return s;
}

const dirOf = (p: string) => p.split("/").slice(0, -1).join("/");

export function finishedSession(): Session {
  return clone(finishedFixture) as unknown as Session;
}

export function settings(): SettingsView {
  const v = clone(settingsFixture) as unknown as SettingsView;
  v.values.outputDir = LIBRARY_ROOT;
  return v;
}

export function cardVolume(): Volume {
  return {
    mount: "/Volumes/DVR",
    info: {
      device_identifier: "disk9s1",
      parent_whole_disk: "disk9",
      internal: false,
      removable: true,
      ejectable: true,
      total_size: 31_914_983_424,
      volume_uuid: "00000000-0000-4000-8000-000000000001",
      volume_name: "DVR",
      mount_point: "/Volumes/DVR",
      filesystem: "MS-DOS FAT32",
      media_name: "SD Card Reader",
      bus_protocol: "USB",
    },
    is_card: true,
    source: "analog",
    is_radio: false,
    warnings: [],
  };
}

/** A DJI air unit over USB: exFAT, clips under `DCIM/DJI_001/`. */
export function djiVolume(): Volume {
  const v = cardVolume();
  v.mount = "/Volumes/O4";
  v.source = "dji";
  v.info = { ...v.info, device_identifier: "disk8s1", parent_whole_disk: "disk8", volume_uuid: "00000000-0000-4000-8000-000000000002", volume_name: "O4", mount_point: "/Volumes/O4", filesystem: "ExFAT", media_name: "DJI Air Unit" };
  return v;
}
