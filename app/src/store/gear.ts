// Gear as the core reports it (`gear_status`, `gear_devices`, `device-changed`), plus the
// Gear page the person is looking at. The core owns every fact; this slice caches it and
// remembers when each card was first seen unmounted, for the "still inserted" state.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api, errText } from "../ipc/api";
import type { ApplyPlan, ApplyReport, Connected, Device, DeviceChanged, GearStatus, StagedChange } from "../ipc/types";
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
}

export interface GearSlice {
  /** Staged changes waiting to be applied, every device. */
  changes: StagedChange[];
  applySheet: ApplySheetState | null;
  /** Opens the sheet on a device's first staged change, or a given one. */
  openApply: (device: string, change?: string) => Promise<void>;
  closeApply: () => void;
  /** Apply in the open sheet. */
  runApply: () => Promise<void>;
  /** Stages the backup taken before the apply as a change and shows it. */
  restoreBeforeApply: () => Promise<void>;
  /** The next staged change of the same device, or closes the sheet. */
  nextApply: () => Promise<void>;
  discardChange: (id: string) => Promise<void>;
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

const ready = (cs: StagedChange[], device: string) => cs.filter((c) => c.device === device && c.status === "ready");

export const createGearSlice: StateCreator<State, [], [], GearSlice> = (set, get) => ({
  changes: [],
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
      const report = await api.gearApplyClick(a.change.id, a.plan.digest);
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
      const c = await api.gearRestoreStage(backup);
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
      const [gear, devices, changes] = await Promise.all([api.gearStatus(), api.gearDevices(), api.gearChanges()]);
      set({ gear, devices, changes });
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
