// Typed calls into the core. Until tauri-specta generates bindings.ts (plan step R5) this
// mirrors the Tauri commands and the `core_call` methods by hand; views and stores use
// only this module, so the swap stays here.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import type {
  CardStatus,
  CutChange,
  EnvCheck,
  FormatPlan,
  GeoResult,
  ImportOptions,
  ImportOutcome,
  LibClip,
  LibCut,
  LibEdit,
  LibraryFilter,
  LibraryView,
  Moved,
  PlanPatch,
  RebuildReport,
  RemovedCuts,
  RenameReport,
  Session,
  SettingsValues,
  SettingsView,
  ShareReport,
  Span,
  TrashReport,
  Volume,
} from "./types";

const call = <T>(method: string, params: object | null = {}) => invoke<T>("core_call", { method, params });

export const api = {
  envCheck: () => invoke<EnvCheck>("env_check"),
  defaultOutputDir: () => invoke<string | null>("default_output_dir"),
  listVolumes: () => invoke<Volume[]>("list_volumes"),
  libraryScope: () => invoke<null>("library_scope"),
  menuState: (enabled: Record<string, boolean>, checked: Record<string, boolean>, albumItem: string | null) => invoke<null>("menu_state", { enabled, checked, albumItem }),
  share: (paths: string[], r: { x: number; y: number; w: number; h: number }) => invoke<null>("share", { paths, ...r }),

  // Session
  getSession: () => invoke<Session | null>("get_session"),
  loadSource: (path: string) => invoke<Session>("load_source", { path }),
  loadDropped: (paths: string[]) => invoke<Session>("load_dropped", { paths }),
  planDates: (logDir: string | null, day: string | null) => invoke<Session>("plan_dates", { logDir, day }),
  editPlan: (patch: PlanPatch) => invoke<Session>("edit_plan", { patch }),
  editPlans: (patches: PlanPatch[]) => invoke<Session>("edit_plans", { patches }),
  importClips: (options: ImportOptions) => invoke<ImportOutcome>("import_clips", { options }),
  addToPhotos: (ids: number[], album: string | null) => invoke<ShareReport>("add_to_photos", { ids, album }),
  formatPlan: (label: string | null) => invoke<FormatPlan>("format_plan", { label }),
  formatCard: (label: string) => invoke<FormatPlan>("format_card", { label }),
  answerFormatRequest: (id: number, approve: boolean) => invoke<null>("answer_format_request", { id, approve }),
  clearSession: () => invoke<null>("clear_session"),
  eject: (path: string | null = null) => invoke<null>("eject", { path }),
  preview: (id: number) => invoke<string>("preview", { id }),
  sessionCuts: (id: number, cuts: Span[], removed: RemovedCuts | null) => call<CutChange>("session_cuts", { id, cuts, removed_cuts: removed }),

  // Library
  library: (filter: LibraryFilter = {}) => call<LibraryView>("library", filter),
  rate: (ids: string[], rating: number | null, flag: LibClip["flag"] | null) => call<LibClip[]>("library_rate", { ids, rating, flag }),
  edit: (id: string, edit: LibEdit) => call<LibClip>("library_edit", { id, ...edit }),
  rename: (id: string, name: string) => call<LibClip>("library_rename", { id, name }),
  libraryCuts: (id: string, cuts: Span[], removed: RemovedCuts | null) => call<CutChange>("library_cuts", { id, cuts, removed_cuts: removed }),
  exportCuts: (id: string) => call<LibCut[]>("library_export_cuts", { id }),
  trash: (ids: string[]) => call<TrashReport>("library_trash", { ids }),
  untrash: (moved: Moved[]) => call<string[]>("library_untrash", { moved }),
  libraryPhotos: (ids: string[], album: string) => call<ShareReport>("library_photos", { ids, album }),
  applyNameFormat: () => call<RenameReport>("library_apply_name_format", {}),
  rebuild: () => call<RebuildReport>("library_rebuild"),
  rescan: (id: string) => call<LibClip>("library_rescan", { id }),
  libraryPreview: (id: string) => call<string>("library_preview", { id }),
  strips: () => call<number>("library_strips", {}),
  cardStatus: (mount: string) => call<CardStatus>("card_status", { mount }),

  // Setup
  settings: () => call<SettingsView>("settings"),
  settingsSet: (values: SettingsValues) => call<SettingsView>("settings_set", { values }),
  placeSearch: (query: string, provider: string, limit = 6) => call<GeoResult[]>("place_search", { query, provider, limit }),
  placeSave: (name: string, lat: number, lon: number) => call<unknown>("place_save", { name, lat, lon }),
};

/** A URL the web view can load for a file the core made (thumbnails, previews). */
export const fileSrc = (path: string | null | undefined) => (path ? convertFileSrc(path) : "");

export const reveal = (path: string) => revealItemInDir(path);
export const openFolder = (path: string) => openPath(path);

/** The folder picker. */
export async function pickFolder(title: string, defaultPath?: string | null): Promise<string | null> {
  const r = await openDialog({ directory: true, multiple: false, title, defaultPath: defaultPath || undefined });
  return typeof r === "string" ? r : null;
}

/** A core error as text. */
export const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
