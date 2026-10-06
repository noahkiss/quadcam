// The generated shapes made to fit ipc/types.ts. specta types every f64 as `number | null`;
// the core never sends null for one, so a null here means a value the UI cannot place: a
// range or point without its time is dropped, a length or score reads as 0. Fields serde
// may omit get their default.
import type * as G from "../bindings";
import type {
  Clip,
  ClipMeta,
  ClipPlan,
  ClipResult,
  CutChange,
  Defaults,
  FlightStats,
  GeoResult,
  ImportProgress,
  LibClip,
  LibraryView,
  Location,
  Moment,
  Place,
  Profile,
  Session,
  SettingsView,
  SettingsValues,
  SignalScan,
  Span,
  Tunables,
} from "./types";

const n = (v: number | null | undefined, d = 0) => (v == null || !Number.isFinite(v) ? d : v);
const has = (v: number | null | undefined): v is number => v != null && Number.isFinite(v);

export const spans = (xs: G.Span[] | null | undefined): Span[] => (xs || []).filter((s) => has(s.start) && has(s.end)).map((s) => ({ start: s.start!, end: s.end! }));

export const moments = (xs: G.Moment[] | null | undefined): Moment[] =>
  (xs || []).filter((m) => has(m.start) && has(m.end)).map((m) => ({ ...m, start: m.start!, end: m.end!, score: n(m.score) }));

export const location = (l: G.Location | null | undefined): Location | null => (l && has(l.lat) && has(l.lon) ? { ...l, lat: l.lat, lon: l.lon } : null);

export const flight = (f: G.FlightStats | null | undefined): FlightStats | null => (f ? { ...f, armed_s: n(f.armed_s), pack_spans: spans(f.pack_spans) } : null);

export const places = (xs: G.Place[] | null | undefined): Place[] => (xs || []).filter((p) => has(p.lat) && has(p.lon)).map((p) => ({ name: p.name, lat: p.lat!, lon: p.lon! }));

export const profile = (p: G.Profile): Profile => ({
  name: p.name ?? "",
  aircraft: p.aircraft ?? "",
  camera_make: p.camera_make ?? "",
  camera_model: p.camera_model ?? "",
  video_system: p.video_system ?? "",
  keywords: p.keywords ?? [],
  author: p.author ?? "",
  place: p.place ?? null,
  edgetx_models: p.edgetx_models ?? [],
});

export const tunables = (t: G.Tunables): Tunables => ({ ...t, segment_gap_s: n(t.segment_gap_s, 5), session_gap_min: n(t.session_gap_min, 20), tolerance_s: n(t.tolerance_s, 30), clock_skew_s: n(t.clock_skew_s, 300) });

export const geo = (xs: G.GeoResult[]): GeoResult[] => xs.filter((g) => has(g.lat) && has(g.lon)).map((g) => ({ ...g, lat: g.lat!, lon: g.lon! }));

export function libClip(c: G.LibItem_Serialize): LibClip {
  return {
    ...c,
    duration: n(c.duration),
    time: c.time ?? null,
    rating: c.rating ?? 0,
    flag: c.flag ?? "none",
    place: c.place ?? null,
    location: location(c.location),
    aircraft: c.aircraft ?? null,
    keywords: c.keywords ?? [],
    author: c.author ?? null,
    moments: moments(c.moments),
    keep: spans(c.keep),
    stats: flight(c.stats),
    cuts: (c.cuts || []).filter((k) => has(k.start) && has(k.end)).map((k) => ({ ...k, start: k.start!, end: k.end! })),
    pending_cuts: spans(c.pending_cuts),
    in_photos: c.in_photos ?? false,
    original: c.original ?? null,
    dvr: c.dvr ?? null,
    import: c.import ?? null,
    aliases: c.aliases ?? [],
    parts: c.parts ?? [],
    cut_of: c.cut_of && has(c.cut_of[1].start) && has(c.cut_of[1].end) ? [c.cut_of[0], { start: c.cut_of[1].start!, end: c.cut_of[1].end! }] : null,
  };
}

export function libraryView(v: G.LibraryView_Serialize): LibraryView {
  return {
    ...v,
    totals: { ...v.totals, seconds: n(v.totals.seconds), flying: n(v.totals.flying) },
    groups: { days: v.groups.days, aircraft: v.groups.aircraft, places: v.groups.places },
    clips: v.clips.map(libClip),
  };
}

const signal = (s: G.SignalScan | null | undefined): SignalScan | null => (s ? { ...s, step: n(s.step), dead_air: moments(s.dead_air), keep: spans(s.keep) } : null);

export function clip(c: G.Clip): Clip {
  return { ...c, kind: c.kind ?? "analog", duration: n(c.duration), probe: c.probe ? { ...c.probe, duration: n(c.probe.duration) } : null, signal: signal(c.signal), key: c.key ?? "", clock: c.clock ?? null, sidecars: c.sidecars ?? [], mtime: c.mtime ?? null, join: c.join ?? null, part_of: c.part_of ?? null };
}

const meta = (m: G.ClipMeta | undefined): ClipMeta => ({ profile: m?.profile ?? null, location: location(m?.location), keywords: m?.keywords ?? [], author: m?.author ?? null });

export function plan(p: G.ClipPlan): ClipPlan {
  const s = p.suggested;
  return {
    ...p,
    suggested: { date: !!s?.date, name: !!s?.name, note: !!s?.note, skip: !!s?.skip, cuts: !!s?.cuts, meta: !!s?.meta },
    reason: p.reason ?? null,
    moments: moments(p.moments),
    log_interval_s: p.log_interval_s ?? null,
    log_offset_s: n(p.log_offset_s),
    cuts: spans(p.cuts),
    meta: meta(p.meta),
    log_model: p.log_model ?? null,
    match_reason: p.match_reason ?? null,
    flight: flight(p.flight),
  };
}

export function result(r: G.ClipResult): ClipResult {
  return {
    ...r,
    meta: r.meta ?? null,
    qt: r.qt ?? [],
    cuts: (r.cuts || []).filter((k) => has(k.start) && has(k.end)).map((k) => ({ ...k, start: k.start!, end: k.end! })),
  };
}

export function session(s: G.Session | null): Session | null {
  if (!s) return null;
  return { ...s, kind: s.kind ?? "analog", clips: s.clips.map(clip), plans: s.plans.map(plan), results: s.results.map(result) };
}

export function cutChange(c: G.CutChange): CutChange {
  return c.status === "applied" ? { status: "applied", cuts: spans(c.cuts), kept: c.kept ?? [], trashed: c.trashed ?? [] } : c;
}

export function settingsView(v: G.SettingsView_Serialize): SettingsView {
  const e = v.effective;
  const effective: Defaults = { ...e, tunables: tunables(e.tunables), places: places(e.places), profiles: e.profiles.map(profile) };
  return { path: v.path, values: v.values as SettingsValues, effective };
}

export const importProgress = (p: G.ImportProgress): ImportProgress => ({ id: p.id, seconds: n(p.seconds), duration: n(p.duration) });
