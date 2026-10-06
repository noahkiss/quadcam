// The settings as the core reports them. The core owns the file and writes only the keys
// it is given, so a write here never undoes one made elsewhere. No defaults are copied
// here: a missing key reads as the core's default through `effective`.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api, errText } from "../ipc/api";
import type { Place, Profile, SettingsValues } from "../ipc/types";
import { toast } from "../components/toastStore";
import { validSort, type Sort } from "../lib/library";

export interface SettingsSlice {
  values: SettingsValues;
  /** What every surface uses: the values over the core's defaults. */
  effective: Record<string, unknown>;
  googleKeySet: boolean;
  /** The user's home folder, for `~` in paths. */
  home: string;
  loadSettings: () => Promise<void>;
  /** Writes one key. */
  saveSetting: <K extends keyof SettingsValues>(key: K, value: SettingsValues[K]) => Promise<void>;
  /** Writes the keys whose value differs from `before`, in one call. */
  saveChanged: (values: SettingsValues, before: SettingsValues) => Promise<void>;
}

export const createSettingsSlice: StateCreator<State, [], [], SettingsSlice> = (set) => ({
  values: {},
  effective: {},
  googleKeySet: false,
  home: "",
  loadSettings: async () => {
    try {
      const v = await api.settings();
      set({ values: v.values, effective: v.effective, googleKeySet: !!v.values.googlePlacesKey });
    } catch (e) {
      console.warn("settings unavailable", e);
    }
  },
  saveSetting: async (key, value) => {
    set((s) => ({ values: { ...s.values, [key]: value } }));
    try {
      await api.settingsSet({ [key]: value } as SettingsValues);
    } catch (e) {
      toast(`Settings not saved: ${errText(e)}`, true);
    }
  },
  saveChanged: async (values, before) => {
    const changed = Object.fromEntries(Object.entries(values).filter(([k, v]) => !sameValue(v, (before as Record<string, unknown>)[k])));
    if (!Object.keys(changed).length) return;
    set((s) => ({ values: { ...s.values, ...changed } }));
    try {
      await api.settingsSet(changed);
    } catch (e) {
      toast(`Settings not saved: ${errText(e)}`, true);
    }
  },
});

/** Deep equality that ignores key order: the core sorts keys, the UI does not. */
export function sameValue(a: unknown, b: unknown): boolean {
  const norm = (v: unknown): unknown =>
    Array.isArray(v) ? v.map(norm) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).sort(([x], [y]) => x.localeCompare(y)).map(([k, x]) => [k, norm(x)])) : v;
  return JSON.stringify(norm(a)) === JSON.stringify(norm(b ?? null)) || (a == null && b == null);
}

const NO_RECENTS = {};
const NO_TUNABLES = { segment_gap_s: 5, session_gap_min: 20, tolerance_s: 30, max_log_age_days: 60, clock_skew_s: 300 };
const NONE: never[] = [];

// Reads with the core's defaults behind them.
const eff = <T>(s: State, file: keyof SettingsValues, key: string, fallback: T): T => (s.values[file] ?? s.effective[key] ?? fallback) as T;

export const sel = {
  outputDir: (s: State) => eff<string | null>(s, "outputDir", "output_dir", null),
  format: (s: State) => eff<"mp4" | "mov">(s, "format", "format", "mp4"),
  encoder: (s: State) => eff<"videotoolbox" | "x264">(s, "encoder", "encoder", "videotoolbox"),
  keepOriginals: (s: State) => eff<boolean>(s, "keepOriginals", "keep_originals", false),
  addTime: (s: State) => eff<boolean>(s, "addTime", "add_time", false),
  defaultName: (s: State) => eff<string>(s, "defaultName", "default_name", "flight") || "flight",
  photosAlbum: (s: State) => eff<string>(s, "photosAlbum", "photos_album", "Drone"),
  formatLabel: (s: State) => eff<string>(s, "formatLabel", "format_label", "DVR") || "DVR",
  logDir: (s: State) => eff<string | null>(s, "logDir", "log_dir", null),
  layout: (s: State) => eff<"year_day" | "day" | "flat">(s, "libraryLayout", "layout", "year_day"),
  placeFolders: (s: State) => eff<boolean>(s, "placeFolders", "place_folders", false),
  places: (s: State) => eff<Place[]>(s, "places", "places", NONE),
  profiles: (s: State) => eff<Profile[]>(s, "profiles", "profiles", NONE),
  defaultProfile: (s: State) => eff<string>(s, "defaultProfile", "default_profile", "") || "",
  geocoder: (s: State) => eff<string>(s, "geocoder", "geocoder", "apple"),
  nameDateFormat: (s: State) => eff<string>(s, "nameDateFormat", "name_date_format", "YYYY-MM-DD"),
  tunables: (s: State) => eff<NonNullable<SettingsValues["tunables"]>>(s, "tunables", "tunables", NO_TUNABLES),
  recents: (s: State) => s.values.recents || NO_RECENTS,
  libView: (s: State) => (s.values.libView === "list" ? "list" : "grid") as "grid" | "list",
  thumbSize: (s: State) => Math.min(5, Math.max(1, s.values.thumbSize || 3)),
  sort: (s: State): Sort => sortOf(s.values.libSort),
};

// Selectors must return the same object for the same input, or React re-renders forever.
let sortIn: unknown = {};
let sortOut: Sort = validSort(undefined);
function sortOf(v: SettingsValues["libSort"]): Sort {
  if (v !== sortIn) {
    sortIn = v;
    sortOut = validSort(v);
  }
  return sortOut;
}
