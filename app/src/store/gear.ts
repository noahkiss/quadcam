// Gear as the core reports it (`gear_status`, `gear_devices`, `device-changed`), plus the
// Gear page the person is looking at. The core owns every fact; this slice caches it and
// remembers when each card was first seen unmounted, for the "still inserted" state.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api, errText } from "../ipc/api";
import type { ApplyPlan, ApplyReport, Connected, Device, DeviceChanged, FirmwareView, FlashParams, GearStatus, SimSyncParams, StagedChange } from "../ipc/types";
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
  /** Set when the sheet shows a firmware flash: the click flashes this radio. */
  flash?: FlashParams | null;
}

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
  /** Opens the sheet on a device's first staged change, or a given one. */
  openApply: (device: string, change?: string) => Promise<void>;
  /** Opens the sheet on a plan to write the quad's rates into sim profiles. */
  openSimSync: (params: SimSyncParams) => Promise<void>;
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
  agentApply: (id: number, change: StagedChange, plan: ApplyPlan) => void;
  agentApplyClosed: (id: number) => void;
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
  openApply: async (device, change) => {
    await get().loadGear();
    const c = (change ? get().changes.find((x) => x.id === change) : ready(get().changes, device)[0]) || null;
    set({ applySheet: { device, change: c, plan: null, report: null, agent: null, busy: !!c, error: null } });
    if (!c) return;
    try {
      const plan = await api.gearApplyPlan(c.id);
      set((s) => (s.applySheet?.change?.id === c.id ? { applySheet: { ...s.applySheet, plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (s.applySheet ? { applySheet: { ...s.applySheet, busy: false, error: errText(e) } } : {}));
    }
  },
  openSimSync: async (params) => {
    set({ applySheet: { device: SIMS_DEVICE, change: simChange(null), plan: null, report: null, agent: null, busy: true, error: null, sim: params } });
    try {
      const plan = await api.gearSimSyncPlan(params);
      set((s) => (s.applySheet?.sim === params ? { applySheet: { ...s.applySheet, change: simChange(plan), plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (s.applySheet?.sim === params ? { applySheet: { ...s.applySheet, busy: false, error: errText(e) } } : {}));
    }
  },
  openFlash: async (params) => {
    set({ applySheet: { device: params.device, change: flashChange(params.device, null), plan: null, report: null, agent: null, busy: true, error: null, flash: params } });
    try {
      const plan = await api.gearFlashPlan(params);
      set((s) => (s.applySheet?.flash === params ? { applySheet: { ...s.applySheet, change: flashChange(params.device, plan), plan, busy: false } } : {}));
    } catch (e) {
      set((s) => (s.applySheet?.flash === params ? { applySheet: { ...s.applySheet, busy: false, error: errText(e) } } : {}));
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
    if (a?.agent != null) void api.answerApplyRequest(a.agent, false);
    set({ applySheet: null });
  },
  runApply: async () => {
    const a = get().applySheet;
    if (!a?.change || !a.plan || a.busy) return;
    set({ applySheet: { ...a, busy: true, error: null } });
    // An agent's request is answered by the click; the agent's own call does the apply.
    if (a.agent != null) {
      await api.answerApplyRequest(a.agent, true);
      return;
    }
    try {
      const report = a.sim ? await api.gearSimSyncClick(a.sim, a.plan.digest) : a.flash ? await api.gearFlashClick(a.flash, a.plan.digest) : await api.gearApplyClick(a.change.id, a.plan.digest);
      set((s) => (s.applySheet ? { applySheet: { ...s.applySheet, report, busy: false } } : {}));
    } catch (e) {
      set((s) => (s.applySheet ? { applySheet: { ...s.applySheet, busy: false, error: errText(e) } } : {}));
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
      set((s) => (s.applySheet ? { applySheet: { ...s.applySheet, error: errText(e) } } : {}));
    }
  },
  nextApply: async () => {
    const a = get().applySheet;
    if (!a) return;
    await get().loadGear();
    if (ready(get().changes, a.device).length) await get().openApply(a.device);
    else set({ applySheet: null });
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
  agentApply: (id, change, plan) => set({ applySheet: { device: change.device, change, plan, report: null, agent: id, busy: false, error: null } }),
  agentApplyClosed: (id) => set((s) => (s.applySheet?.agent === id ? { applySheet: null } : {})),
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
