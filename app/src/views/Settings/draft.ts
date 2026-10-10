// The Settings sheet works on a draft. Done writes only the keys whose value changed while
// it was open, so a change the CLI or an agent made meanwhile stays.
import type { State } from "../../store";
import { sel } from "../../store/settings";
import type { Automation, CueSettings, DeviceKind, GearSettings, Layout, Place, Profile, SettingsValues, Tunables } from "../../ipc/types";

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
  deleteClips: boolean;
  joinSplit: boolean;
  segGap: string;
  sessionGap: string;
  tolerance: string;
  clockSkew: string;
  photosAlbum: string;
  geocoder: string;
  googleKey: string;
  ffmpegSource: "module" | "homebrew";
  places: PlaceRow[];
  profiles: Profile[];
  /** Index of the default profile in `profiles`, or null for none. */
  defaultIndex: number | null;
  /** The person unticked Default aircraft. */
  defaultCleared: boolean;
  gear: GearDraft;
}

/** Settings > Gear. Numbers stay text while they are typed. */
export interface GearDraft {
  autoBackup: boolean;
  /** Shows Gear > Sim. */
  simPreview: boolean;
  /** Offers the Betaflight flash on the Firmware page. */
  bfFlashPreview: boolean;
  /** Shows the ELRS tools on the Firmware page. */
  elrsPreview: boolean;
  keepRecent: string;
  keepWeeks: string;
  keepMonthly: boolean;
  usbMinutes: string;
  eraseBlackbox: boolean;
  firmwareDaily: boolean;
  blackboxMsc: boolean;
  onConnect: Record<DeviceKind, ShownAutomation[]>;
  cues: Required<Omit<CueSettings, "quiet_hours" | "voice">> & { voice: string };
  quiet: boolean;
  quietStart: string;
  quietEnd: string;
}

export const GEAR_KINDS: DeviceKind[] = ["radio", "fc", "elrs_tx", "elrs_rx", "goggles", "dvr_card"];
/** The on-connect steps Settings offers. `import` stays valid in the file, the CLI and MCP, and
 *  does nothing, so the pane does not show it. */
export type ShownAutomation = Exclude<Automation, "import">;
export const AUTOMATIONS: ShownAutomation[] = ["backup", "apply_ready", "blackbox"];

/** `CueSettings::default()` and `GearSettings::defaults` in the core, for a core that has not
 *  answered yet. */
const CUE_DEFAULTS: GearDraft["cues"] = {
  mute: false,
  speech: true,
  sound: false,
  notification: true,
  safe_to_unplug: true,
  still_inserted: true,
  step_failed: true,
  unplug_now: true,
  debounce_s: 30,
  reminder_grace_s: 60,
  still_inserted_every_s: 300,
  reminder_max: 3,
  voice: "",
  voice_source: "macos",
};

export function gearDraft(g: GearSettings | null | undefined, simPreview = false, bfFlashPreview = false, elrsPreview = false): GearDraft {
  const q = g?.cues.quiet_hours;
  const onConnect = Object.fromEntries(GEAR_KINDS.map((k) => [k, AUTOMATIONS.filter((a) => (g ? g.on_connect[k] || [] : a === "backup" ? [a] : []).includes(a))])) as GearDraft["onConnect"];
  const cues = { ...CUE_DEFAULTS, ...Object.fromEntries(Object.entries(g?.cues || {}).filter(([, v]) => v != null)) } as GearDraft["cues"];
  return {
    autoBackup: g?.auto_backup ?? true,
    simPreview,
    bfFlashPreview,
    elrsPreview,
    keepRecent: String(g?.keep_recent ?? 10),
    keepWeeks: String(g?.keep_weeks ?? 8),
    keepMonthly: g?.keep_monthly ?? true,
    usbMinutes: String(g?.usb_minutes ?? 20),
    eraseBlackbox: g?.erase_blackbox ?? false,
    firmwareDaily: g?.firmware_check === "daily",
    blackboxMsc: g?.blackbox_msc ?? false,
    onConnect,
    cues: { ...cues, voice: cues.voice || "" },
    quiet: !!q,
    quietStart: q?.start || "22:00",
    quietEnd: q?.end || "07:00",
  };
}

const whole = (v: string, d: number, min: number, max: number) => {
  const n = Math.round(Number(v));
  return v.trim() !== "" && Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : d;
};

/** The Gear keys the draft stands for (the checks in `settings::KEYS`). */
export function gearValues(g: GearDraft): SettingsValues {
  const { voice, ...cues } = g.cues;
  return {
    gearAutoBackup: g.autoBackup,
    simPreview: g.simPreview,
    bfFlashPreview: g.bfFlashPreview,
    elrsPreview: g.elrsPreview,
    gearKeepRecent: whole(g.keepRecent, 10, 1, 1000),
    gearKeepWeeks: whole(g.keepWeeks, 8, 0, 520),
    gearKeepMonthly: g.keepMonthly,
    gearUsbMinutes: whole(g.usbMinutes, 20, 0, 240),
    gearEraseBlackbox: g.eraseBlackbox,
    gearBlackboxMsc: g.blackboxMsc,
    firmwareCheck: g.firmwareDaily ? "daily" : "manual",
    gearOnConnect: g.onConnect,
    gearCues: { ...cues, voice: voice.trim() || null, quiet_hours: g.quiet ? { start: g.quietStart, end: g.quietEnd } : null },
  };
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
    deleteClips: sel.deleteClips(s),
    joinSplit: sel.joinSplit(s),
    segGap: String(t.segment_gap_s),
    sessionGap: String(t.session_gap_min),
    tolerance: String(t.tolerance_s),
    clockSkew: String(t.clock_skew_s ?? 300),
    photosAlbum: sel.photosAlbum(s),
    geocoder: sel.geocoder(s),
    googleKey: "",
    ffmpegSource: sel.ffmpegSource(s),
    places: sel.places(s).map((p) => ({ name: p.name, lat: String(p.lat), lon: String(p.lon) })),
    profiles,
    defaultIndex: di >= 0 ? di : null,
    defaultCleared: false,
    gear: gearDraft(s.gear?.settings, s.values.simPreview === true, s.values.bfFlashPreview === true, s.values.elrsPreview === true),
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
    deleteClipsAfterImport: d.deleteClips,
    joinSplitRecordings: d.joinSplit,
    photosAlbum: d.photosAlbum.trim(),
    libraryLayout: d.libraryLayout,
    placeFolders: d.placeFolders,
    keepOriginals: d.keepOriginals,
    geocoder: d.geocoder as SettingsValues["geocoder"],
    nameDateFormat: d.nameDateFormat,
    ffmpegSource: d.ffmpegSource,
    ...(d.googleKey.trim() ? { googlePlacesKey: d.googleKey.trim() } : {}),
    ...(d.outputDir ? { outputDir: d.outputDir } : {}),
    places: validPlaces(d.places).places,
    profiles,
    defaultProfile,
    ...gearValues(d.gear),
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

export const emptyProfile = (): Profile => ({ name: "", aircraft: "", camera_make: "", camera_model: "", video_system: "Analog", keywords: [], author: "", place: null, edgetx_models: [], gear: {} });
