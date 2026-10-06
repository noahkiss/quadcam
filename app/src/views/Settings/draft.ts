// The Settings sheet works on a draft. Done writes only the keys whose value changed while
// it was open, so a change the CLI or an agent made meanwhile stays.
import type { State } from "../../store";
import { sel } from "../../store/settings";
import type { Layout, Place, Profile, SettingsValues, Tunables } from "../../ipc/types";

export interface PlaceRow {
  name: string;
  lat: string;
  lon: string;
}

export interface Draft {
  outputDir: string | null;
  libraryLayout: Layout;
  placeFolders: boolean;
  keepOriginals: boolean;
  nameDateFormat: "YYYY-MM-DD" | "YY.MM.DD";
  format: "mp4" | "mov";
  encoder: "videotoolbox" | "x264";
  defaultName: string;
  addTime: boolean;
  segGap: string;
  sessionGap: string;
  tolerance: string;
  clockSkew: string;
  photosAlbum: string;
  geocoder: string;
  googleKey: string;
  places: PlaceRow[];
  profiles: Profile[];
  /** Index of the default profile in `profiles`, or null for none. */
  defaultIndex: number | null;
  /** The person unticked Default aircraft. */
  defaultCleared: boolean;
}

export function draftFrom(s: State): Draft {
  const t = sel.tunables(s);
  const profiles = structuredClone(sel.profiles(s));
  const def = sel.defaultProfile(s);
  const di = profiles.findIndex((p) => p.name === def);
  return {
    outputDir: sel.outputDir(s),
    libraryLayout: sel.layout(s),
    placeFolders: sel.placeFolders(s),
    keepOriginals: sel.keepOriginals(s),
    nameDateFormat: sel.nameDateFormat(s) === "YY.MM.DD" ? "YY.MM.DD" : "YYYY-MM-DD",
    format: sel.format(s),
    encoder: sel.encoder(s),
    defaultName: sel.defaultName(s),
    addTime: sel.addTime(s),
    segGap: String(t.segment_gap_s),
    sessionGap: String(t.session_gap_min),
    tolerance: String(t.tolerance_s),
    clockSkew: String(t.clock_skew_s ?? 300),
    photosAlbum: sel.photosAlbum(s),
    geocoder: sel.geocoder(s),
    googleKey: "",
    places: sel.places(s).map((p) => ({ name: p.name, lat: String(p.lat), lon: String(p.lon) })),
    profiles,
    defaultIndex: di >= 0 ? di : null,
    defaultCleared: false,
  };
}

const num = (v: string, d: number) => (v.trim() !== "" && Number.isFinite(+v) ? +v : d);

export function validPlaces(rows: PlaceRow[]): { places: Place[]; dropped: boolean } {
  const parsed = rows.map((r) => ({ name: r.name.trim(), lat: parseFloat(r.lat), lon: parseFloat(r.lon) }));
  const places = parsed.filter((p) => p.name && Number.isFinite(p.lat) && Number.isFinite(p.lon) && Math.abs(p.lat) <= 90 && Math.abs(p.lon) <= 180);
  const meant = parsed.filter((p) => p.name || Number.isFinite(p.lat));
  return { places, dropped: places.length < meant.length };
}

/** The values the draft stands for, in file keys. */
export function valuesOf(d: Draft, before: Tunables): SettingsValues {
  const profiles = d.profiles.filter((p) => p.name.trim()).map((p) => ({ ...p, name: p.name.trim(), place: p.place || null, video_system: p.video_system || "Analog" }));
  const def = d.defaultIndex != null ? d.profiles[d.defaultIndex] : null;
  // A deleted or unnamed default falls to the first profile, unless the person cleared it.
  const defaultProfile = def && def.name.trim() ? def.name.trim() : d.defaultCleared ? "" : profiles[0]?.name || "";
  return {
    defaultName: d.defaultName.trim() || "flight",
    encoder: d.encoder,
    format: d.format,
    addTime: d.addTime,
    photosAlbum: d.photosAlbum.trim(),
    libraryLayout: d.libraryLayout,
    placeFolders: d.placeFolders,
    keepOriginals: d.keepOriginals,
    geocoder: d.geocoder as SettingsValues["geocoder"],
    nameDateFormat: d.nameDateFormat,
    ...(d.googleKey.trim() ? { googlePlacesKey: d.googleKey.trim() } : {}),
    ...(d.outputDir ? { outputDir: d.outputDir } : {}),
    places: validPlaces(d.places).places,
    profiles,
    defaultProfile,
    tunables: { ...before, segment_gap_s: num(d.segGap, 5), session_gap_min: num(d.sessionGap, 20), tolerance_s: num(d.tolerance, 30), clock_skew_s: num(d.clockSkew, 300) },
  };
}

/** The folder tree a layout makes, as `library::day_dir` does. */
export function layoutExample(d: Pick<Draft, "libraryLayout" | "placeFolders" | "keepOriginals" | "nameDateFormat" | "outputDir">, place: string, today = new Date()) {
  const day = today.toISOString().slice(0, 10);
  const yr = day.slice(0, 4);
  const dayDir = d.placeFolders ? `${day} ${place}` : day;
  const root = `${(d.outputDir || "").split("/").pop() || "quadcam"}/`;
  const fd = d.nameDateFormat === "YY.MM.DD" ? day.slice(2).replaceAll("-", ".") : day;
  const files = [`${fd}_backyard_loops.mp4`, `${fd}_backyard_loops_cut1.mp4`, ...(d.keepOriginals ? [`originals/${fd}_backyard_loops.avi`] : []), "…"];
  const tree = (indent: string, list: string[]) => list.map((f, i) => `${indent}${i === list.length - 1 ? "└─" : "├─"} ${f}`).join("\n");
  if (d.libraryLayout === "flat") return `${root}\n${tree("", files)}`;
  if (d.libraryLayout === "day") return `${root}\n└─ ${dayDir}/\n${tree("   ", files)}`;
  return `${root}\n└─ ${yr}/\n   └─ ${dayDir}/\n${tree("      ", files)}`;
}

export const VIDEO_SYSTEMS = ["Analog", "DJI O4", "Walksnail", "HDZero"];

export const emptyProfile = (): Profile => ({ name: "", aircraft: "", camera_make: "", camera_model: "", video_system: "Analog", keywords: [], author: "", place: null, edgetx_models: [] });
