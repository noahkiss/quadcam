// Gear as the core reports it (`gear_status`, `gear_devices`, `device-changed`), plus the
// Gear page the person is looking at. The core owns every fact; this slice caches it and
// remembers when each card was first seen unmounted, for the "still inserted" state.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api, errText } from "../ipc/api";
import type { Connected, Device, DeviceChanged, GearStatus } from "../ipc/types";
import { linkHandle } from "../lib/gear";
import { toast } from "../components/toastStore";

/** A Gear page: the Connected list, the device list, one device (by `pageKey`), or a page a
 *  later package adds (by its id in `views/Gear/pages.tsx`). */
export type GearPage = { page: "connected" } | { page: "devices" } | { page: "device"; key: string } | { page: "slot"; id: string };

export interface GearSlice {
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

export const createGearSlice: StateCreator<State, [], [], GearSlice> = (set, get) => ({
  gear: null,
  devices: [],
  unmounted: [],
  unmountedSince: {},
  gearPage: null,
  gearSegment: "overview",
  gearExpanded: true,
  loadGear: async () => {
    try {
      const [gear, devices] = await Promise.all([api.gearStatus(), api.gearDevices()]);
      set({ gear, devices });
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
