// The core's shapes, written by hand from the Rust types until tauri-specta generates
// bindings.ts (plan step R5). Field names and optionality follow serde's output.

export type Path = string;
/** `YYYY-MM-DD` */
export type IsoDate = string;

export interface Span {
  start: number;
  end: number;
}

export type MomentKind = "roll" | "flip" | "punch" | "dive" | "crash" | "dead_air";

export interface Moment {
  kind: MomentKind;
  start: number;
  end: number;
  score: number;
  source: "radio_log" | "video";
  detail: string;
}

export interface FlightStats {
  armed_s: number;
  packs: number;
  min_rx_bat_v: number | null;
  min_lq: number | null;
  min_rssi_db: number | null;
  max_throttle: number | null;
}

export interface Location {
  lat: number;
  lon: number;
  name?: string | null;
}

export interface Place {
  name: string;
  lat: number;
  lon: number;
}

export interface Profile {
  name: string;
  aircraft: string;
  camera_make: string;
  camera_model: string;
  video_system: string;
  keywords: string[];
  author: string;
  place: string | null;
  edgetx_models: string[];
}

export type Flag = "none" | "pick" | "reject";

export interface LibCut {
  path: Path;
  start: number;
  end: number;
  size: number;
}

/** `LibItem`: the index entry flattened, plus absolute paths. */
export interface LibClip {
  id: string;
  path: Path;
  title: string;
  note: string;
  date: IsoDate;
  time: string | null;
  duration: number;
  size: number;
  rating: number;
  flag: Flag;
  place: string | null;
  location: Location | null;
  aircraft: string | null;
  keywords: string[];
  author: string | null;
  moments: Moment[];
  keep: Span[];
  stats: FlightStats | null;
  cuts: LibCut[];
  pending_cuts: Span[];
  in_photos: boolean;
  original: Path | null;
  dvr: string | null;
  import: string | null;
  cut_of: [string, Span] | null;
  name: string;
  file: Path;
  strip: Path | null;
  poster: Path | null;
  no_picture: boolean;
  last_import: boolean;
}

export type Layout = "year_day" | "day" | "flat";

export interface Totals {
  clips: number;
  bytes: number;
  seconds: number;
  flying: number;
}

export interface LibraryView {
  root: Path;
  exists: boolean;
  unindexed: number;
  last_import: string | null;
  totals: Totals;
  groups: Partial<Record<"days" | "aircraft" | "places", [string, number][]>>;
  layout: Layout;
  place_folders: boolean;
  clips: LibClip[];
}

export interface LibraryFilter {
  query?: string | null;
  group?: string | null;
  day?: IsoDate | null;
  place?: string | null;
  aircraft?: string | null;
  min_rating?: number | null;
}

/** Missing fields stay as they are. */
export interface LibEdit {
  note?: string;
  keywords?: string[];
  author?: string;
  place?: string;
  location?: Location;
  date?: IsoDate;
  time?: string;
  profile?: string;
}

export interface Moved {
  id: string;
  from: Path;
  to: Path;
}

export interface TrashReport {
  trashed: Path[];
  failed: [Path, string][];
  moved: Moved[];
}

export interface ShareReport {
  added: Path[];
  failed: [Path, string][];
  album: string | null;
}

export interface RenameReport {
  renamed: [Path, Path][];
  unchanged: number;
  skipped: Path[];
  failed: [Path, string][];
}

export interface RebuildReport {
  clips: number;
  cuts: number;
  problems: string[];
}

export type RemovedCuts = "keep" | "trash";

export type CutChange =
  | { status: "applied"; cuts: Span[]; kept: Path[]; trashed: Path[] }
  | { status: "confirm"; files: Path[] };

export interface CardStatus {
  mount: Path;
  clips: number;
  new: number;
  free: number | null;
  size: number;
}

export interface DiskInfo {
  device_identifier: string;
  parent_whole_disk: string;
  internal: boolean;
  removable: boolean;
  ejectable: boolean;
  total_size: number;
  volume_uuid: string | null;
  volume_name: string | null;
  mount_point: string | null;
  filesystem: string | null;
  media_name: string | null;
  bus_protocol: string | null;
}

export interface Volume {
  mount: Path;
  info: DiskInfo;
  is_card: boolean;
  is_radio: boolean;
  warnings: string[];
}

export interface CardIdentity {
  device_identifier: string;
  whole_disk: string;
  volume_uuid: string | null;
  volume_name: string | null;
  total_size: number;
  media_name: string | null;
}

export interface Probe {
  duration: number;
  video_packets: number;
  video_streams: number;
  audio_streams: number;
  fps: number | null;
  width: number | null;
  height: number | null;
  tags: Record<string, string>;
  errors: string;
}

export interface SignalScan {
  step: number;
  samples: number;
  dead_air: Moment[];
  keep: Span[];
}

export interface Clip {
  id: number;
  name: string;
  rel: string;
  card_path: Path;
  size: number;
  staged: Path | null;
  stage_error: string | null;
  status: "ok" | "incomplete" | "empty";
  duration: number;
  recovered: Path | null;
  probe: Probe | null;
  thumb: Path | null;
  detail: string;
  signal: SignalScan | null;
  key: string;
}

export interface Suggested {
  date: boolean;
  name: boolean;
  note: boolean;
  skip: boolean;
  cuts: boolean;
  meta: boolean;
}

export interface ClipMeta {
  profile: string | null;
  location: Location | null;
  keywords: string[];
  author: string | null;
}

export interface ClipPlan {
  id: number;
  skip: boolean;
  date: IsoDate;
  /** `HH:MM:SS`; null means local noon. */
  time: string | null;
  source: "log" | "import" | "edited";
  badge: "matched" | "likely" | "unmatched";
  segments: number;
  name: string;
  note: string;
  suggested: Suggested;
  reason: string | null;
  moments: Moment[];
  log_interval_s: number | null;
  log_offset_s: number;
  cuts: Span[];
  meta: ClipMeta;
  log_model: string | null;
  flight: FlightStats | null;
}

/** Missing fields stay as they are. */
export interface PlanPatch {
  id: number;
  date?: IsoDate;
  time?: string;
  name?: string;
  note?: string;
  skip?: boolean;
  reason?: string;
  cuts?: Span[];
  log_offset_s?: number;
  profile?: string;
  location?: Location;
  place?: string;
  keywords?: string[];
  author?: string;
  removed_cuts?: RemovedCuts;
}

export type Outcome = "verified" | "failed" | "skipped";

export interface CutResult {
  start: number;
  end: number;
  outcome: Outcome;
  output: Path | null;
  size: number;
  error: string | null;
}

export interface ClipResult {
  id: number;
  outcome: Outcome;
  output: Path | null;
  original: Path | null;
  size: number;
  error: string | null;
  encoder: string | null;
  meta: unknown;
  cuts: CutResult[];
  qt: [string, string][];
}

export interface Session {
  version: number;
  source: Path;
  card: CardIdentity | null;
  card_volume: Volume | null;
  staging: Path;
  clips: Clip[];
  plans: ClipPlan[];
  results: ClipResult[];
  analysed: boolean;
  log_dir: Path | null;
  log_day: IsoDate | null;
  log_days: IsoDate[];
  warnings: string[];
  date_warnings: string[];
  in_photos: number[];
  output_dir: Path | null;
}

export interface Summary {
  results: ClipResult[];
  imported: number;
  skipped: number;
  failed: number;
  total_bytes: number;
  output_dir: Path | null;
  format_ready: { Ok: null } | { Err: string };
}

export interface ImportOptions {
  output_dir: Path;
  format: "mp4" | "mov";
  encoder: "videotoolbox" | "x264";
  keep_originals: boolean;
  add_time: boolean;
}

export interface ImportOutcome {
  summary: Summary;
  photos: { Ok: ShareReport } | { Err: string } | null;
}

export interface FormatPlan {
  disk: string;
  device: string;
  volume_uuid: string;
  volume_name: string;
  size: number;
  media_name: string;
  clip_count: number;
  label: string;
}

export interface Tunables {
  segment_gap_s: number;
  session_gap_min: number;
  tolerance_s: number;
  max_log_age_days: number;
}

/** The settings file's values, by file key (camelCase). */
export interface SettingsValues {
  outputDir?: Path | null;
  format?: "mp4" | "mov";
  encoder?: "videotoolbox" | "x264";
  keepOriginals?: boolean;
  addTime?: boolean;
  defaultName?: string;
  photosAlbum?: string;
  formatLabel?: string;
  logDir?: Path | null;
  libraryLayout?: Layout;
  placeFolders?: boolean;
  tunables?: Tunables;
  places?: Place[];
  profiles?: Profile[];
  defaultProfile?: string;
  geocoder?: "apple" | "nominatim" | "census" | "google";
  googlePlacesKey?: string;
  nameDateFormat?: "YYYY-MM-DD" | "YY.MM.DD";
  recents?: { keywords?: string[]; authors?: string[]; notes?: string[] };
  libView?: "grid" | "list";
  thumbSize?: number;
  libSort?: { key: "date" | "rating" | "duration" | "name"; dir: "asc" | "desc" };
  [key: string]: unknown;
}

export interface SettingsView {
  path: Path;
  values: SettingsValues;
  effective: Record<string, unknown>;
}

export interface GeoResult {
  name: string;
  address: string;
  lat: number;
  lon: number;
  provider: string;
}

export interface EnvCheck {
  tools: { ffmpeg: Path; ffprobe: Path } | null;
  error: string | null;
  install_hint: string;
  socket: string | null;
}

// Events the core emits.
export interface StageProgress {
  phase: "stage" | "analyse";
  index: number;
  total: number;
  done: number;
  size: number;
}

export interface ImportProgress {
  id: number;
  seconds: number;
  duration: number;
}

export interface LibraryTask {
  task: string;
  done: number;
  total: number;
}

export interface AgentFormatRequest {
  id: number;
  plan: FormatPlan;
}
