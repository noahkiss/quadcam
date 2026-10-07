// The core's shapes as the UI uses them. Every type comes from the generated bindings.ts
// (tauri-specta, never edited). Where bindings.ts has gaps the UI should not carry, this
// file narrows them, and ipc/normalize.ts makes the values fit:
// - specta types every f64 as `number | null`; here they are `number`.
// - fields serde may omit (`#[serde(default)]`) are `?` there; here they are present.
// A field renamed or removed in Rust breaks the build here or in normalize.ts.
import type * as G from "../bindings";

export type {
  Automation,
  Connected,
  CueSettings,
  Device,
  DeviceChanged,
  DeviceEvent,
  DeviceKind,
  GearSettings,
  GearStatus,
  Identity,
  Link,
  QuietHours,
  VoiceSource,
  Badge,
  CardIdentity,
  CardStatus,
  DiskInfo,
  Editor,
  EnvCheck,
  Flag,
  FormatPlan,
  ImportOptions,
  Layout,
  LibEdit,
  MomentKind,
  OsdBox,
  OsdElement,
  OsdParams,
  OsdProblem,
  OsdProfile,
  OsdView,
  Moved,
  Outcome,
  PlanPatch,
  RebuildReport,
  RemovedCuts,
  RenameReport,
  ShareReport,
  SourceKind,
  Tools,
  TrashReport,
  Volume,
} from "../bindings";
export type { Filter as LibraryFilter } from "../bindings";

export type Path = string;
/** `YYYY-MM-DD` */
export type IsoDate = string;

/** `T` with the float fields `K` as plain numbers. */
type Num<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: number };

export type Span = Num<G.Span, "start" | "end">;
export type Moment = Num<G.Moment, "start" | "end" | "score">;
export type FlightStats = Omit<Num<G.FlightStats, "armed_s">, "pack_spans"> & { pack_spans: Span[] };
export type Location = Num<G.Location, "lat" | "lon">;
export type Place = Num<G.Place, "lat" | "lon">;
export type Profile = Required<G.Profile>;
export type Tunables = Num<G.Tunables, "segment_gap_s" | "session_gap_min" | "tolerance_s" | "clock_skew_s">;
export type GeoResult = Num<G.GeoResult, "lat" | "lon">;
export type Suggested = Required<G.Suggested>;

export type LibCut = Num<G.LibCut, "start" | "end">;

/** `LibItem`: the index entry flattened, plus absolute paths. */
export type LibClip = Omit<Required<G.LibItem_Serialize>, "duration" | "time" | "location" | "moments" | "keep" | "stats" | "cuts" | "pending_cuts" | "cut_of"> & {
  duration: number;
  time: string | null;
  location: Location | null;
  moments: Moment[];
  keep: Span[];
  stats: FlightStats | null;
  cuts: LibCut[];
  pending_cuts: Span[];
  cut_of: [string, Span] | null;
};

export type Totals = Num<G.Totals, "seconds" | "flying">;

export type LibraryView = Omit<G.LibraryView_Serialize, "totals" | "groups" | "clips"> & {
  totals: Totals;
  groups: Partial<Record<"days" | "aircraft" | "places", [string, number][]>>;
  clips: LibClip[];
};

export type Probe = Num<G.Probe, "duration">;
export type SignalScan = Omit<Num<G.SignalScan, "step">, "dead_air" | "keep"> & { dead_air: Moment[]; keep: Span[] };

export type Clip = Omit<Required<G.Clip>, "duration" | "probe" | "signal"> & {
  duration: number;
  probe: Probe | null;
  signal: SignalScan | null;
};

export type ClipMeta = Omit<Required<G.ClipMeta>, "location"> & { location: Location | null };

export type ClipPlan = Omit<Required<G.ClipPlan>, "suggested" | "moments" | "log_offset_s" | "cuts" | "meta" | "flight"> & {
  suggested: Suggested;
  moments: Moment[];
  log_offset_s: number;
  cuts: Span[];
  meta: ClipMeta;
  flight: FlightStats | null;
};

export type CutResult = Num<G.CutResult, "start" | "end">;
export type ClipResult = Omit<Required<G.ClipResult>, "cuts"> & { cuts: CutResult[] };

export type Session = Omit<G.Session, "kind" | "clips" | "plans" | "results"> & {
  kind: G.SourceKind;
  clips: Clip[];
  plans: ClipPlan[];
  results: ClipResult[];
};

export type CutChange = { status: "applied"; cuts: Span[]; kept: Path[]; trashed: Path[] } | { status: "confirm"; files: Path[] };

export type ImportOutcome = G.ImportOutcome;
export type ModuleStatus = G.ModuleStatus;
export type ModulePin = G.Pin;
export type ClipDeletion = G.ClipDeletion;

/** The settings file's values, by file key (camelCase). */
export interface SettingsValues {
  outputDir?: Path | null;
  format?: "mp4" | "mov";
  encoder?: "videotoolbox" | "x264";
  keepOriginals?: boolean;
  addTime?: boolean;
  deleteClipsAfterImport?: boolean;
  joinSplitRecordings?: boolean;
  defaultName?: string;
  photosAlbum?: string;
  formatLabel?: string;
  logDir?: Path | null;
  libraryLayout?: G.Layout;
  placeFolders?: boolean;
  tunables?: Tunables;
  places?: Place[];
  profiles?: Profile[];
  defaultProfile?: string;
  geocoder?: "apple" | "nominatim" | "census" | "google";
  googlePlacesKey?: string;
  nameDateFormat?: G.DateFormat;
  ffmpegSource?: G.FfmpegSource;
  recents?: { keywords?: string[]; authors?: string[]; notes?: string[] };
  libView?: "grid" | "list";
  thumbSize?: number;
  libSort?: { key: "date" | "rating" | "duration" | "name"; dir: "asc" | "desc" };
  [key: string]: unknown;
}

export type Defaults = Omit<G.Defaults_Serialize, "tunables" | "places" | "profiles"> & {
  tunables: Tunables;
  places: Place[];
  profiles: Profile[];
};

export interface SettingsView {
  path: Path;
  values: SettingsValues;
  effective: Defaults;
}

// Events the core emits.
export type StageProgress = G.Progress;
export type ImportProgress = Num<G.ImportProgress, "seconds" | "duration">;
export type LibraryTask = G.LibraryTask;
export type AgentFormatRequest = G.AgentFormatRequest;
