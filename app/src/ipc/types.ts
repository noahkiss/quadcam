// The core's shapes as the UI uses them. Every type comes from the generated bindings.ts
// (tauri-specta, never edited). Where bindings.ts has gaps the UI should not carry, this
// file narrows them, and ipc/normalize.ts makes the values fit:
// - specta types every f64 as `number | null`; here they are `number`.
// - fields serde may omit (`#[serde(default)]`) are `?` there; here they are present.
// A field renamed or removed in Rust breaks the build here or in normalize.ts.
import type * as G from "../bindings";

export type {
  Automation,
  BlackboxEntry,
  BlackboxErased,
  BlackboxPullResult,
  EraseState,
  FcJob,
  FlightLink,
  Linked,
  LogInfo,
  Pull,
  BackupContent,
  BackupImportItem,
  BackupProgress,
  BackupResult,
  BackupSummary,
  CardCheck,
  DeviceStorage,
  DiffItem,
  GearJob,
  ImportBackupsReport,
  PruneReport,
  RepairResult,
  StepFailure,
  StorageView,
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
  OsdCopy,
  OsdEditParams,
  OsdElement,
  OsdMove,
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
// Flights, packs, crashes, the session report and the "Pack up" check, as generated.
export type {
  ChargeState,
  Chemistry,
  CheckRow,
  Crash,
  CrashSaveParams,
  Dropout,
  FlightReport,
  FlightsView,
  Pack,
  PackType,
  PackTypeView,
  PackView,
  PacksView,
  PlaceTrend,
  Preflight,
  RowState,
  SessionReport,
  TrendPoint,
} from "../bindings";

// The sim's radio calibration, as generated.
export type {
  AxisCal,
  CalibrateParams,
  CalibrateView,
  Calibration,
  CalPhase,
  CaptureTarget,
  RadioControl,
  RadioResolution,
  SavedCalibration,
  SimCalibration,
  SimCalibrationSaveParams,
  SimDefaults,
  SuggestedControl,
} from "../bindings";

export type { SimPreset, SimStartParams, SimStopInfo } from "../bindings";

// The sim host's answers, with the numbers narrowed (specta types every f64 as number | null;
// the core sends a finite number, and the loop skips a frame that holds anything else).
type V3 = [number, number, number];
export interface SimPose {
  step: number;
  t: number;
  host_ns: number;
  input_ns: number;
  pos: V3;
  quat: [number, number, number, number];
}
export interface SimHud {
  armed: boolean;
  turtle: boolean;
  airmode: boolean;
  mode: "acro" | "angle" | "horizon";
  arm_block: string | null;
  vbat: number;
  mah: number;
  low_battery: boolean;
  sticks: { roll: number; pitch: number; yaw: number; throttle: number };
  throttle: number;
  stuck: boolean;
  contacts: number;
  dropped_steps: number;
}
export interface SimFrame {
  host_ns: number;
  prev: SimPose;
  cur: SimPose;
  hud: SimHud;
  radio: boolean;
}
/** The `simSettings` key of the settings file: the Sim page's choices. */
export interface SimUiSettings {
  profile?: string | null;
  view?: "fpv" | "chase";
  aspect?: "16:9" | "4:3" | null;
  stick_display?: boolean;
  osd?: boolean;
  uptilt_deg?: number | null;
  fov_deg?: number | null;
}
export interface SimBox {
  centre: V3;
  half: V3;
  yaw_deg: number;
  material: "wall" | "floor" | "carpet" | "grass" | "gate";
}
export interface SimStartInfo {
  profile: string;
  label: string;
  dt: number;
  world_name: string;
  boxes: SimBox[];
  start: V3;
  start_yaw_deg: number;
  camera: { uptilt_deg: number; fov_deg: number; aspect: string; position: V3 };
  wheelbase_m: number;
  body_half: V3;
  prop_radius_m: number;
  stick_mode: number;
  calibrated: boolean;
  host_ns: number;
}

export type Path = string;
/** `YYYY-MM-DD` */
export type IsoDate = string;

/** `T` with the float fields `K` as plain numbers. */
type Num<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: number };

export type Span = Num<G.Span, "start" | "end">;
export type Moment = Num<G.Moment, "start" | "end" | "score">;
export type FlightStats = Omit<Num<G.FlightStats_Serialize, "armed_s">, "flight_spans"> & { flight_spans: Span[] };
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
export type CardRelease = G.CardRelease;

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
  stickMode?: number;
  simSettings?: SimUiSettings;
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

// The switch map and the radio as a USB joystick.
export type Stick = G.Stick;
export type StickChannel = G.StickChannel;
export type ChannelValue = G.ChannelValue;
export type Adjustment = G.Adjustment;
export type Live = G.Live;
export type SwitchMapParams = G.SwitchMapParams;
export type Combo = G.Combo;
export type Unmapped = G.Unmapped;
export type Position = Omit<G.Position, "source" | "combos"> & { source: string | null; combos: Combo[] };
export type ControlRow = Omit<G.ControlRow, "switch_type" | "positions"> & { switch_type: string | null; positions: Position[] };
export type AuxMode = Omit<G.AuxMode, "linked"> & { linked: string | null };
export type SwitchMap = Omit<G.SwitchMap, "model" | "live" | "rows" | "modes" | "unmapped"> & {
  model: string | null;
  unmapped: Unmapped[];
  live: Live | null;
  rows: ControlRow[];
  modes: AuxMode[];
};
export type RadioFrame = G.RadioFrame;
export type RadioEvent = { connected: boolean; product: string | null; frame: RadioFrame | null };
export type RadioSnapshot = RadioEvent & { message: string | null };

// Events the core emits.
export type StageProgress = G.Progress;
export type ImportProgress = Num<G.ImportProgress, "seconds" | "duration">;
export type LibraryTask = G.LibraryTask;
export type AgentFormatRequest = G.AgentFormatRequest;
export type FormatRequest = G.FormatRequest;
export type AgentApplyRequest = G.AgentApplyRequest;
export type { ApplyPlan, ApplyReport, CardMounted, ChangeStatus, Check, CopyPart, CopyPlan, Edit, StagedChange, StepReport } from "../bindings";

/** `gear_copy_plan` and `gear_copy_stage`: where settings come from and go to. */
export interface CopyRequest {
  /** An FC's device id (its latest backup) or a backup id. */
  from: string;
  /** The FC that gets the settings. */
  to: string;
  parts: import("../bindings").CopyPart[];
  /** Settings by name, on top of the parts. */
  settings: string[];
}

// Rates and the sims' rates, with the numbers narrowed (the core sends a finite number).
export type { RatesParams, SimRestoreParams, SimsParams, SimSyncParams, SimTarget } from "../bindings";
export type { FirmwareStatus, FirmwareView, FlashParams, SplashParams, SplashPreview } from "../bindings";
/** A profile re-drawn, and after a conversion how far each axis is off. */
export interface RatesPreview {
  profile: RateProfile;
  fit_error: number[];
  fit_share: number[];
}
export interface RateAxis {
  axis: string;
  rc_rate: number;
  srate: number;
  expo: number;
  rate_limit: number;
  max_deg_s: number;
  center_deg_s: number;
  /** Deg/s at stick i / (curve.length - 1), from 0 to full. */
  curve: number[];
}
export interface RateThrottle {
  mid: number;
  expo: number;
  hover: number | null;
  limit: string;
  limit_percent: number;
  /** Output 0-1 at stick i / (curve.length - 1). */
  curve: number[];
}
export interface RateProfile {
  index: number;
  name: string | null;
  active: boolean;
  rates_type: string;
  complete: boolean;
  axes: RateAxis[];
  throttle: RateThrottle;
}
export interface RatesView {
  source: string[];
  firmware: string | null;
  active: number | null;
  profiles: RateProfile[];
  notes: string[];
}
export interface SimRateDiff {
  max_diff: number[];
  quad_max_diff: number[];
  fit_error: number[];
  throttle_differs: boolean | null;
  same: boolean;
}
export interface SimRateProfile {
  name: string;
  supported: boolean;
  note: string | null;
  axes: RateAxis[];
  throttle: RateThrottle | null;
  diff: SimRateDiff | null;
}
export interface SimRateFile {
  path: string;
  error: string | null;
  profiles: SimRateProfile[];
}
export interface SimRates {
  id: string;
  name: string;
  enabled: boolean;
  found: boolean;
  running: boolean;
  note: string | null;
  files: SimRateFile[];
  in_sync: boolean | null;
}

// The radio's model editors, as generated.
export type {
  CalloutDef,
  CalloutView,
  CalloutWhen,
  EditorView,
  Field as ModelField,
  LoggingDef,
  LoggingView,
  ModelDetail,
  ModelEditParams,
  ModelEntry,
  ModelOp,
  ModelParams,
  ModelView,
  RfAlarms,
  ScreenDetail,
  SensorLog,
  Timer as ModelTimer,
} from "../bindings";

// Radio voice, as generated.
export type {
  LineOverride,
  PackInstallParams,
  Plan as VoicePlan,
  ProviderView,
  RenderReport,
  VoiceChooseParams,
  VoiceEditParams,
  VoiceLine,
  VoiceParams,
  VoicePack,
  VoicePreviewParams,
  VoiceRenderParams,
  VoiceView,
} from "../bindings";
