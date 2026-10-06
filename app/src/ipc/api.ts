// Typed calls into the core, on the commands tauri-specta generates (../bindings.ts). Each
// call resolves its data or rejects with the core's error text. Answers the UI reads are made
// to fit ipc/types.ts (see normalize.ts); the rest come back as generated, since the next
// library-changed or session-changed event refetches the view. Views and stores use only this module.
import { convertFileSrc } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { commands } from "../bindings";
import * as N from "./normalize";
import type { ImportOptions, LibEdit, LibraryFilter, Moved, PlanPatch, RemovedCuts, SettingsValues, Span } from "./types";

type Result<T> = Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>;

async function ok<T>(r: Result<T>): Promise<T> {
  const x = await r;
  if (x.status === "error") throw x.error;
  return x.data;
}

export const api = {
  envCheck: () => commands.envCheck(),
  defaultOutputDir: () => commands.defaultOutputDir(),
  listVolumes: () => ok(commands.volumes()),
  libraryScope: () => ok(commands.libraryScope()),
  menuState: (enabled: Record<string, boolean>, checked: Record<string, boolean>, albumItem: string | null) => commands.menuState(enabled, checked, albumItem),
  share: (paths: string[], r: { x: number; y: number; w: number; h: number }) => ok(commands.share(paths, r.x, r.y, r.w, r.h)),

  // Session
  getSession: async () => N.session(await ok(commands.session())),
  loadSource: async (path: string) => N.session(await ok(commands.load({ source: path })))!,
  loadDropped: async (paths: string[]) => N.session(await ok(commands.loadDropped(paths)))!,
  planDates: async (logDir: string | null, day: string | null) => N.session(await ok(commands.dates({ logs: logDir ? { kind: "dir", path: logDir } : { kind: "none" }, day })))!,
  editPlan: async (patch: PlanPatch) => N.session(await ok(commands.suggest({ patches: [patch], editor: "user" })))!,
  editPlans: async (patches: PlanPatch[]) => N.session(await ok(commands.suggest({ patches, editor: "user" })))!,
  importClips: (options: ImportOptions) => ok(commands.import(options)),
  addToPhotos: (ids: number[], album: string | null) => ok(commands.photos({ ids, album })),
  formatPlan: (label: string | null) => ok(commands.formatPlan({ label })),
  formatCard: (label: string) => ok(commands.formatCard(label)),
  answerFormatRequest: (id: number, approve: boolean) => commands.answerFormatRequest(id, approve),
  clearSession: () => ok(commands.clear()),
  eject: (path: string | null = null) => ok(commands.eject({ target: path })),
  preview: (id: number) => ok(commands.preview(id)),
  sessionCuts: async (id: number, cuts: Span[], removed: RemovedCuts | null) => N.cutChange(await ok(commands.sessionCuts({ id, cuts, removed_cuts: removed }))),
  sessionSplit: async (id: number) => N.cutChange(await ok(commands.sessionSplit({ id }))),

  // Library
  library: async (filter: LibraryFilter = {}) => N.libraryView(await ok(commands.library(filter))),
  rate: (ids: string[], rating: number | null, flag: "none" | "pick" | "reject" | null) => ok(commands.libraryRate({ ids, rating, flag })),
  edit: (id: string, edit: LibEdit) => ok(commands.libraryEdit({ id, ...edit })),
  rename: (id: string, name: string) => ok(commands.libraryRename({ id, name })),
  libraryCuts: async (id: string, cuts: Span[], removed: RemovedCuts | null) => N.cutChange(await ok(commands.libraryCuts({ id, cuts, removed_cuts: removed }))),
  exportCuts: (id: string) => ok(commands.libraryExportCuts({ id })),
  librarySplit: async (id: string) => N.cutChange(await ok(commands.librarySplit({ id }))),
  trash: (ids: string[]) => ok(commands.libraryTrash({ ids })),
  untrash: (moved: Moved[]) => ok(commands.libraryUntrash({ moved })),
  libraryPhotos: (ids: string[], album: string) => ok(commands.libraryPhotos({ ids, album })),
  applyNameFormat: () => ok(commands.libraryApplyNameFormat({})),
  rebuild: () => ok(commands.libraryRebuild()),
  matchLogs: (apply: boolean, ids: string[] = []) => ok(commands.libraryMatchLogs({ ids, logs: null, day: null, apply })),
  rescan: (id: string) => ok(commands.libraryRescan({ id })),
  libraryPreview: (id: string) => ok(commands.libraryPreview({ id })),
  strips: () => ok(commands.libraryStrips({})),
  cardStatus: (mount: string) => ok(commands.cardStatus({ mount })),

  // Setup
  settings: async () => N.settingsView(await ok(commands.settings())),
  settingsSet: async (values: SettingsValues) => N.settingsView(await ok(commands.settingsSet({ values }))),
  placeSearch: async (query: string, provider: string, limit = 6) => N.geo(await ok(commands.placeSearch({ query, provider, limit }))),
  placeSave: (name: string, lat: number, lon: number) => ok(commands.placeSave({ name, lat, lon })),
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
