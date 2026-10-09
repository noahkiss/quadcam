// Typed calls into the core, on the commands tauri-specta generates (../bindings.ts). Each
// call resolves its data or rejects with the core's error text. Answers the UI reads are made
// to fit ipc/types.ts (see normalize.ts); the rest come back as generated, since the next
// library-changed or session-changed event refetches the view. Views and stores use only this module.
import { convertFileSrc } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { commands } from "../bindings";
import * as N from "./normalize";
import type { ChangeStatus, CopyRequest, Edit, SimFrame, SimPreset, SimStartInfo, SimStartParams, SimStopInfo, CrashSaveParams, ImportOptions, LibEdit, LibraryFilter, Moved, ModelEditParams, ModelParams, PackInstallParams, VoiceChooseParams, VoiceEditParams, VoiceParams, VoicePreviewParams, VoiceRenderParams, OsdEditParams, OsdParams, RatesParams, RatesPreview, RateProfile, FlashParams, SplashParams, SimSyncParams, SimsParams, Pack, PackType, PlanPatch, RemovedCuts, SettingsValues, Span, SwitchMapParams, SimCalibrationSaveParams, CalibrateParams } from "./types";

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
  thirdPartyNotices: () => ok(commands.thirdPartyNotices()),
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

  // Gear
  gearStatus: () => ok(commands.gearStatus()),
  gearDevices: () => ok(commands.gearDevices()),
  gearDeviceSave: (id: string, name: string | null, aircraft: string | null) => ok(commands.gearDeviceSave({ id, name, aircraft })),
  gearDeviceForget: (id: string) => ok(commands.gearDeviceForget({ id })),
  gearPollPause: (port: string, paused: boolean) => ok(commands.gearPollPause({ port, paused })),
  gearDismissReminder: (handle: string) => ok(commands.gearDismissReminder({ handle })),
  gearBackup: (target: { device?: string | null; port?: string | null; mount?: string | null }) => ok(commands.gearBackup(target)),
  gearBackups: (device: string | null) => ok(commands.gearBackups({ device })),
  gearBackupRead: (id: string, path: string | null) => ok(commands.gearBackupRead({ id, path })),
  gearBackupDiff: (a: string, b: string | null = null, path: string | null = null) => ok(commands.gearBackupDiff({ a, b, path })),
  gearBackupPin: (id: string, pinned: boolean) => ok(commands.gearBackupPin({ id, pinned })),
  gearStorage: () => ok(commands.gearStorage()),
  gearPrune: (dryRun: boolean) => ok(commands.gearPrune({ dry_run: dryRun })),
  gearExport: (to: string, target: { device?: string | null; snapshot?: string | null }) => ok(commands.gearExport({ to, ...target })),
  gearImportBackups: (folder: string, dryRun: boolean, device: string | null = null) => ok(commands.gearImportBackups({ folder, device, dry_run: dryRun })),
  gearCardCheck: (device: string) => ok(commands.gearCardCheck({ device })),
  gearCardChecks: (device: string) => ok(commands.gearCardChecks({ device })),
  gearCardRepair: (check: string) => ok(commands.gearCardRepair({ check, confirm: true })),
  gearStop: (handle: string) => ok(commands.gearStop({ handle })),
  gearChanges: (device: string | null = null, history = false) => ok(commands.gearChanges({ device, status: null, history })),
  gearChangeStage: (device: string, edits: Edit[], title: string | null = null) => ok(commands.gearChangeStage({ device, title, edits, note: null, editor: null, draft: false })),
  gearChangeDiscard: (id: string) => ok(commands.gearChangeDiscard({ id })),
  gearRestoreStage: (backup: string, paths: string[] = []) => ok(commands.gearRestoreStage({ backup, paths, editor: null })),
  gearChangeSetStatus: (id: string, status: ChangeStatus) => ok(commands.gearChangeUpdate({ id, title: null, edits: null, status, note: null, order: null })),
  gearChangeKeep: (id: string) => ok(commands.gearChangeKeep({ id })),
  gearChangeRevert: (id: string) => ok(commands.gearChangeRevert({ id })),
  gearCopyPlan: (p: CopyRequest) => ok(commands.gearCopyPlan({ ...p, editor: null })),
  gearCopyStage: (p: CopyRequest) => ok(commands.gearCopyStage({ ...p, editor: null })),
  gearCardMount: (device: string, minutes: number | null = null) => ok(commands.gearCardMount({ device, minutes })),
  gearCardUnmount: (device: string) => ok(commands.gearCardUnmount({ device, minutes: null })),
  gearApplyPlan: (id: string) => ok(commands.gearApplyPlan({ id, port: null })),
  /** The apply sheet's own Apply click: the click is the confirm. */
  gearApplyClick: (id: string, digest: string) => ok(commands.gearApplyClick({ id, digest, confirm: true, port: null })),
  answerApplyRequest: (id: number, approve: boolean) => commands.answerApplyRequest(id, approve),

  // Modules
  modules: () => ok(commands.modules()),
  moduleInstall: (name: string) => ok(commands.moduleInstall({ name, confirm: true })),
  moduleRemove: (name: string) => ok(commands.moduleRemove({ name })),
  modulesCheck: () => ok(commands.modulesCheck()),

  // Gear
  gearVoice: (params: VoiceParams) => ok(commands.gearVoice(params)),
  gearVoiceEdit: (params: VoiceEditParams) => ok(commands.gearVoiceEdit(params)),
  gearVoicePreview: (params: VoicePreviewParams) => ok(commands.gearVoicePreview(params)),
  gearVoiceRender: (params: VoiceRenderParams) => ok(commands.gearVoiceRender(params)),
  gearVoicePackInstall: (params: PackInstallParams) => ok(commands.gearVoicePackInstall(params)),
  gearVoiceChoose: (params: VoiceChooseParams) => ok(commands.gearVoiceChoose(params)),
  gearModel: (params: ModelParams) => ok(commands.gearModel(params)),
  gearModelEdit: (params: ModelEditParams) => ok(commands.gearModelEdit(params)),
  gearOsd: (params: OsdParams) => ok(commands.gearOsd(params)),
  gearOsdEdit: (params: OsdEditParams) => ok(commands.gearOsdEdit(params)),
  gearRates: async (params: RatesParams) => N.rates(await ok(commands.gearRates(params))),
  /** An edited profile with fresh curves; `to` fits it onto another rate model first. */
  gearRatesPreview: async (profile: RateProfile, to: string | null = null): Promise<RatesPreview> => {
    const r = await ok(commands.gearRatesPreview({ profile, to }));
    return { profile: N.rateProfile(r.profile), fit_error: r.fit_error.map((x) => x ?? 0), fit_share: r.fit_share.map((x) => x ?? 0) };
  },
  gearSimSyncPlan: (p: SimSyncParams) => ok(commands.gearSimSyncPlan(p)),
  /** The apply sheet's own Apply click for a sim sync: the click is the confirm. */
  gearSimSyncClick: (p: SimSyncParams, digest: string) => ok(commands.gearSimSyncClick({ ...p, digest, confirm: true })),
  gearFirmware: (check: boolean | null = null) => ok(commands.gearFirmware({ check })),
  gearSplash: (p: SplashParams) => ok(commands.gearSplash(p)),
  gearFlashPlan: (p: FlashParams) => ok(commands.gearFlashPlan(p)),
  /** The apply sheet's own Apply click for a firmware flash: the click is the confirm. */
  gearFlashClick: (p: FlashParams, digest: string) => ok(commands.gearFlashClick({ ...p, digest, confirm: true })),
  gearSims: async (params: SimsParams) => N.sims(await ok(commands.gearSims(params))),
  gearSwitchMap: async (params: SwitchMapParams) => N.switchMap(await ok(commands.gearSwitchMap(params))),
  gearRadio: async () => N.radioSnapshot(await ok(commands.gearRadio({ wait_ms: null }))),
  gearRadioWatch: (on: boolean) => ok(commands.gearRadioWatch({ on })),
  gearSimCalibration: (radio: string | null = null) => ok(commands.gearSimCalibration({ radio })),
  gearSimCalibrationSave: (p: SimCalibrationSaveParams) => ok(commands.gearSimCalibrationSave(p)),
  gearSimDefaults: (aircraft: string | null) => ok(commands.gearSimDefaults({ aircraft, radio: null, fc: [] })),
  gearSimCalibrate: (p: CalibrateParams) => ok(commands.gearSimCalibrate(p)),

  // The sim host (GUI commands only; the answers are narrowed in ipc/types.ts)
  simPresets: (): Promise<SimPreset[]> => commands.simPresets(),
  simStart: async (p: SimStartParams) => (await ok(commands.simStart(p))) as unknown as SimStartInfo,
  simFrame: async () => (await ok(commands.simFrame())) as unknown as SimFrame,
  simReset: () => ok(commands.simReset()),
  simStop: (): Promise<SimStopInfo | null> => ok(commands.simStop()),

  // Flights and packs
  gearFlights: (day: string | null = null) => ok(commands.gearFlights({ day })),
  gearFlightSet: (flight: string, pack: string | null, place: string | null = null) => ok(commands.gearFlightSet({ flight, pack, place })),
  gearFlightFolders: (add: string | null, remove: string | null = null) => ok(commands.gearFlightFolders({ add, remove })),
  gearPacks: () => ok(commands.gearPacks({})),
  gearPackSave: (pack: Pack, charged: boolean | null = null) => ok(commands.gearPackSave({ pack, charged })),
  gearPackDelete: (label: string) => ok(commands.gearPackDelete({ name: label })),
  gearPackTypeSave: (t: PackType) => ok(commands.gearPackTypeSave(t)),
  gearPackTypeDelete: (name: string) => ok(commands.gearPackTypeDelete({ name })),
  gearPackNotes: (text: string) => ok(commands.gearPackNotes({ text })),
  gearSessionReport: (day: string | null = null) => ok(commands.gearSessionReport({ day })),
  gearSessionReportSave: (day: string | null, path: string) => ok(commands.gearSessionReportSave({ day, path, overwrite: true })),
  gearPreflight: () => ok(commands.gearPreflight()),
  gearCrashes: (clip: string | null = null, aircraft: string | null = null) => ok(commands.gearCrashes({ clip, aircraft })),
  gearCrashSave: (c: CrashSaveParams) => ok(commands.gearCrashSave(c)),
  gearCrashDelete: (id: string) => ok(commands.gearCrashDelete({ id })),
};

/** A URL the web view can load for a file the core made (thumbnails, previews). */
export const fileSrc = (path: string | null | undefined) => (path ? convertFileSrc(path) : "");

export const reveal = (path: string) => revealItemInDir(path);
export const openFolder = (path: string) => openPath(path);
/** Opens a web page in the default browser. */
export const openLink = (url: string) => openUrl(url);

/** The folder picker. */
export async function pickFolder(title: string, defaultPath?: string | null): Promise<string | null> {
  const r = await openDialog({ directory: true, multiple: false, title, defaultPath: defaultPath || undefined });
  return typeof r === "string" ? r : null;
}

/** The file picker, for one or more files. */
export async function pickFiles(title: string, filters: { name: string; extensions: string[] }[] = []): Promise<string[]> {
  const r = await openDialog({ directory: false, multiple: true, title, filters });
  return r === null ? [] : Array.isArray(r) ? r : [r];
}

/** The save panel: the path the person picked, or null. The panel asks before it replaces a file. */
export async function pickSavePath(title: string, defaultPath: string, filters: { name: string; extensions: string[] }[] = []): Promise<string | null> {
  const r = await saveDialog({ title, defaultPath, filters });
  return typeof r === "string" ? r : null;
}

/** A refusal's machine frame, `Refused (usb_heat): `. The core puts its code there for scripts. */
const REFUSED_FRAME = /Refused \([a-z_]+\): /g;

/** A core error as text for a person. A refusal reads as its reason, never with its code. */
export const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e)).replace(REFUSED_FRAME, "");
