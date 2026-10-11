// Gear as the core reports it (`gear_status`, `gear_devices`, `device-changed`), plus the
// Gear page the person is looking at. The core owns every fact; this slice caches it and
// remembers when each card was first seen unmounted, for the "still inserted" state.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api, errText } from "../ipc/api";
import type { ApplyPlan, ApplyReport, Connected, Device, DeviceChanged, FirmwareView, FlashParams, GearStatus, SimRestoreParams, SimSyncParams, StagedChange } from "../ipc/types";
import { linkHandle } from "../lib/gear";
import { toast } from "../components/toastStore";

/** A Gear page: the Connected list, the device list, one device (by `pageKey`), or a page a
 *  later package adds (by its id in `views/Gear/pages.tsx`). */
export type GearPage = { page: "connected" } | { page: "devices" } | { page: "device"; key: string } | { page: "slot"; id: string };

/** The apply sheet: one staged change, its plan, and what the apply did. */
export interface ApplySheetState {
  device: string;
  /** The change shown; null when the device has none left. */
  change: StagedChange | null;
  plan: ApplyPlan | null;
  report: ApplyReport | null;
  /** The request id when an agent asked: the click answers it. */
  agent: number | null;
  busy: boolean;
  error: string | null;
  /** Set when the sheet shows a sim sync: the click writes these sim profiles. */
  sim?: SimSyncParams | null;
  /** Set when the sheet shows a sim restore: the click puts this sim's backup back. */
  restore?: SimRestoreParams | null;
  /** Set when the sheet shows a firmware flash: the click flashes this radio. */
  flash?: FlashParams | null;
  /** Which opening of the sheet this is: an answer that arrives later lands only in the sheet it started from. */
  token: number;
  /** The person approved the agent's request: the sheet shows the write and its result. */
  approved?: boolean;
}

/** An agent's apply request (`agent-apply-request`). */
export interface AgentApplyAsk {
  id: number;
  change: StagedChange;
  plan: ApplyPlan;
}

let tokens = 0;
/** A newly opened sheet, with its own token. */
const opened = (a: Omit<ApplySheetState, "token">): ApplySheetState => ({ ...a, token: ++tokens });
/** The sheet with this token is still the one open. */
const same = (s: State, token: number) => s.applySheet?.token === token;
const agentSheet = (q: AgentApplyAsk) => opened({ device: q.change.device, change: q.change, plan: q.plan, report: null, agent: q.id, busy: false, error: null });
/** The sheet closes; the first queued request, if any, takes its place. */
const nextAsk = (s: State): Partial<State> => {
  const [q, ...rest] = s.applyQueue;
  return q && !s.writeSheet ? { applySheet: agentSheet(q), applyQueue: rest } : { applySheet: null };
};

/** The id of the change an agent's ExpressLRS flash request carries (the core's `ELRS_FLASH_CHANGE`). */
export const ELRS_FLASH = "elrs-flash";

/** The device id a sim sync's sheet, report and stand-in change carry (the core's `DEVICE`). */
export const SIMS_DEVICE = "sims";

/** The change the sheet shows for a sim sync (the core's `pseudo_change`). */
const simChange = (plan: ApplyPlan | null): StagedChange => ({
  id: "sim-sync",
  device: SIMS_DEVICE,
  title: "Sync sims",
  status: "ready",
  edits: [],
  base_backup: "",
  editor: "user",
  note: (plan?.warnings ?? []).join(" "),
  order: 0,
  history: [],
});

/** The change the sheet shows for a sim restore (the core's `pseudo_restore`). */
const restoreChange = (plan: ApplyPlan | null): StagedChange => ({ ...simChange(plan), id: "sim-restore", title: "Restore a sim backup" });

/** The change the sheet shows for a firmware flash (the core's `flash_change`). */
const flashChange = (device: string, plan: ApplyPlan | null): StagedChange => ({
  id: "flash",
  device,
  title: "Flash firmware",
  status: "ready",
  edits: [],
  base_backup: "",
  editor: "user",
  note: (plan?.warnings ?? []).join(" "),
  order: 0,
  history: [],
});

export interface GearSlice {
  /** Staged changes waiting to be applied, every device. */
  changes: StagedChange[];
  /** Every change, staged or not: the Bench's applied-and-undecided items and history. */
  allChanges: StagedChange[];
  applySheet: ApplySheetState | null;
  /** Agent requests that arrived while a sheet was open: each shows when the sheet before it closes. */
  applyQueue: AgentApplyAsk[];
  /** A sheet outside the store that writes a device (the ExpressLRS flash) is open: agent requests wait. */
  writeSheet: boolean;
  setWriteSheet: (open: boolean) => void;
  /** Opens the sheet on a device's first staged change, or a given one. */
  openApply: (device: string, change?: string) => Promise<void>;
  /** Opens the sheet on a plan to write the quad's rates into sim profiles. */
  openSimSync: (params: SimSyncParams) => Promise<void>;
  openSimRestore: (params: SimRestoreParams) => Promise<void>;
  /** Opens the sheet on a plan to flash an EdgeTX radio, with a splash when given. */
  openFlash: (params: FlashParams) => Promise<void>;
  /** The Firmware page's rows. `check`: read the network (true), the saved answer (false), or follow the firmwareCheck setting (null). */
  firmware: FirmwareView | null;
  loadFirmware: (check?: boolean | null) => Promise<void>;
  closeApply: () => void;
  /** Apply in the open sheet. */
  runApply: () => Promise<void>;
  /** Stages the backup taken before the apply as a change and shows it. */
  restoreBeforeApply: () => Promise<void>;
  /** The next staged change of the same device, or closes the sheet. */
  nextApply: () => Promise<void>;
  discardChange: (id: string) => Promise<void>;
  /** Draft, Ready, Try or Read first. */
  setChangeStatus: (id: string, status: StagedChange["status"]) => Promise<void>;
  /** Keeps an applied Try change. */
  keepChange: (id: string) => Promise<void>;
  /** Stages a restore of the backup the change's apply took, and opens it in the sheet. */
  revertChange: (id: string) => Promise<void>;
  /** Mounts an unmounted card for the person to browse, or unmounts it (Done). */
  mountCard: (device: string) => Promise<void>;
  unmountCard: (device: string) => Promise<void>;
  /** Shows an agent's request, or queues it while another sheet is open. */
  agentApply: (q: AgentApplyAsk) => void;
  agentApplyClosed: (id: number) => void;
  /** The outcome of an approved agent request: shown in the sheet that approved it. */
  agentApplyResult: (id: number, report: ApplyReport | null, error: string | null) => void;
  gear: GearStatus | null;
  devices: Device[];
  /** Cards unmounted but still in (from `device-changed`). */
  unmounted: Connected[];
  /** When each unmounted link was first seen so (ms since epoch), by `linkHandle`. */
  unmountedSince: Record<string, number>;
  gearPage: GearPage | null;
  /** The device page's segment (`views/Gear/segments.tsx`). */
  gearSegment: string;
  /** The sidebar's Gear section is open. */
  gearExpanded: boolean;
  loadGear: () => Promise<void>;
  deviceChanged: (p: DeviceChanged) => void;
  openGear: (p: GearPage) => void;
  setGearSegment: (id: string) => void;
  closeGear: () => void;
  setGearExpanded: (on: boolean) => void;
  dismissReminder: (c: Connected) => Promise<void>;
  /** Pauses or resumes the app's own reads of an FC port. */
  setPollPaused: (port: string, paused: boolean) => Promise<void>;
}

const ready = (cs: StagedChange[], device: string) => cs.filter((c) => c.device === device && (c.status === "ready" || c.status === "try"));

export const createGearSlice: StateCreator<State, [], [], GearSlice> = (set, get) => ({
  changes: [],
  allChanges: [],
  applySheet: null,
  applyQueue: [],
  writeSheet: false,
  setWriteSheet: (open) => {
    set({ writeSheet: open });
    if (!open && !get().applySheet) set(nextAsk(get()));
  },
  openApply: async (device, change) => {
    await get().loadGear();
    const c = (change ? get().changes.find((x) => x.id === change) : ready(get().changes, device)[0]) || null;
    const sheet = opened({ device, change: c, plan: null, report: null, agent: null, busy: !!c, error: null });
    set({ applySheet: sheet });
    if (!c) return;
    try {
      const plan = await api.gearApplyPlan(c.id);
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, busy: false, error: errText(e) } } : {}));
    }
  },
  openSimSync: async (params) => {
    const sheet = opened({ device: SIMS_DEVICE, change: simChange(null), plan: null, report: null, agent: null, busy: true, error: null, sim: params });
    set({ applySheet: sheet });
    try {
      const plan = await api.gearSimSyncPlan(params);
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, change: simChange(plan), plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, busy: false, error: errText(e) } } : {}));
    }
  },
  openSimRestore: async (params) => {
    const sheet = opened({ device: SIMS_DEVICE, change: restoreChange(null), plan: null, report: null, agent: null, busy: true, error: null, restore: params });
    set({ applySheet: sheet });
    try {
      const plan = await api.gearSimRestorePlan(params);
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, change: restoreChange(plan), plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, busy: false, error: errText(e) } } : {}));
    }
  },
  openFlash: async (params) => {
    const sheet = opened({ device: params.device, change: flashChange(params.device, null), plan: null, report: null, agent: null, busy: true, error: null, flash: params });
    set({ applySheet: sheet });
    try {
      const plan = await api.gearFlashPlan(params);
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, change: flashChange(params.device, plan), plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (same(s, sheet.token) ? { applySheet: { ...s.applySheet!, busy: false, error: errText(e) } } : {}));
    }
  },
  firmware: null,
  loadFirmware: async (check = null) => {
    try {
      set({ firmware: await api.gearFirmware(check) });
    } catch (e) {
      toast(errText(e), true);
    }
  },
  closeApply: () => {
    const a = get().applySheet;
    if (a?.busy && a.report === null && a.agent === null && a.plan === null) return;
    // An approved request's write is running: the sheet stays until its result.
    if (a?.approved && a.busy) return;
    if (a?.agent != null && !a.approved) void api.answerApplyRequest(a.agent, false);
    set(nextAsk(get()));
  },
  runApply: async () => {
    const a = get().applySheet;
    if (!a?.change || !a.plan || a.busy || a.approved) return;
    // An agent's request is answered by the click; the agent's own call does the apply, and
    // `agent-apply-result` brings its report back to this sheet.
    if (a.agent != null) {
      set({ applySheet: { ...a, busy: true, approved: true, error: null } });
      try {
        await api.answerApplyRequest(a.agent, true);
      } catch (e) {
        set((s) => (same(s, a.token) ? { applySheet: { ...s.applySheet!, busy: false, approved: false, error: errText(e) } } : {}));
      }
      return;
    }
    set({ applySheet: { ...a, busy: true, error: null } });
    try {
      const report = a.sim ? await api.gearSimSyncClick(a.sim, a.plan.digest) : a.restore ? await api.gearSimRestoreClick(a.restore, a.plan.digest) : a.flash ? await api.gearFlashClick(a.flash, a.plan.digest) : await api.gearApplyClick(a.change.id, a.plan.digest);
      set((s) => (same(s, a.token) ? { applySheet: { ...s.applySheet!, report, busy: false } } : {}));
    } catch (e) {
      set((s) => (same(s, a.token) ? { applySheet: { ...s.applySheet!, busy: false, error: errText(e) } } : {}));
    }
    await get().loadGear();
  },
  restoreBeforeApply: async () => {
    const a = get().applySheet;
    const backup = a?.report?.backup;
    if (!a || !backup) return;
    try {
      const c = await api.gearRestoreStage(backup, a.report?.files ?? []);
      await get().openApply(a.device, c.id);
    } catch (e) {
      set((s) => (same(s, a.token) ? { applySheet: { ...s.applySheet!, error: errText(e) } } : {}));
    }
  },
  nextApply: async () => {
    const a = get().applySheet;
    if (!a) return;
    await get().loadGear();
    if (!same(get(), a.token)) return;
    if (ready(get().changes, a.device).length) await get().openApply(a.device);
    else get().closeApply();
  },
  discardChange: async (id) => {
    try {
      await api.gearChangeDiscard(id);
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
  setChangeStatus: async (id, status) => {
    try {
      await api.gearChangeSetStatus(id, status);
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
  keepChange: async (id) => {
    try {
      await api.gearChangeKeep(id);
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
  revertChange: async (id) => {
    try {
      const c = await api.gearChangeRevert(id);
      await get().openApply(c.device, c.id);
    } catch (e) {
      toast(errText(e), true);
    }
  },
  mountCard: async (device) => {
    try {
      await api.gearCardMount(device);
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
  unmountCard: async (device) => {
    try {
      await api.gearCardUnmount(device);
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
  agentApply: (q) => {
    // Never over the person's own sheet, or over another request: it waits its turn.
    if (get().applySheet || get().writeSheet) set((s) => ({ applyQueue: [...s.applyQueue, q] }));
    else set({ applySheet: agentSheet(q) });
  },
  agentApplyClosed: (id) => {
    set((s) => ({ applyQueue: s.applyQueue.filter((q) => q.id !== id) }));
    // Cancelled or timed out before the click. An approved request waits for its result.
    const a = get().applySheet;
    if (a?.agent === id && !a.approved) set(nextAsk(get()));
  },
  agentApplyResult: (id, report, error) => {
    set((s) => (s.applySheet?.agent === id && s.applySheet.approved ? { applySheet: { ...s.applySheet, report, error, busy: false } } : {}));
    void get().loadGear();
  },
  gear: null,
  devices: [],
  unmounted: [],
  unmountedSince: {},
  gearPage: null,
  gearSegment: "overview",
  gearExpanded: true,
  loadGear: async () => {
    try {
      const [gear, devices, changes, allChanges] = await Promise.all([api.gearStatus(), api.gearDevices(), api.gearChanges(), api.gearChanges(null, true)]);
      set({ gear, devices, changes, allChanges });
      // The saved answer only: it never reads the network.
      api.gearFirmware(false).then(
        (firmware) => set({ firmware }),
        () => undefined,
      );
    } catch (e) {
      console.warn("gear unavailable", e);
    }
  },
  deviceChanged: (p) => {
    const before = get().unmountedSince;
    const now = Date.now();
    const unmountedSince: Record<string, number> = {};
    for (const c of p.unmounted) {
      const h = linkHandle(c.link);
      unmountedSince[h] = before[h] ?? now;
    }
    set((s) => ({ unmounted: p.unmounted, unmountedSince, gear: s.gear ? { ...s.gear, connected: p.connected } : s.gear }));
  },
  openGear: (gearPage) => set({ gearPage, gearSegment: "overview", detailId: null, detailSeek: null }),
  setGearSegment: (gearSegment) => set({ gearSegment }),
  closeGear: () => set({ gearPage: null }),
  setGearExpanded: (gearExpanded) => set({ gearExpanded }),
  setPollPaused: async (port, paused) => {
    try {
      await api.gearPollPause(port, paused);
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
  dismissReminder: async (c) => {
    try {
      await api.gearDismissReminder(linkHandle(c.link));
      await get().loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
  },
});

/** What is plugged in now. */
export const connectedOf = (s: State): Connected[] => s.gear?.connected || [];
