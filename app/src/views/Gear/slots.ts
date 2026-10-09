// The places later Gear packages fill. Each is a plain value or list; a package adds its
// entry with a one-line edit here or in `segments.tsx` / `pages.tsx`, never by changing the
// shell's components.
import type { ReactNode } from "react";
import type { Connected, Device, DeviceKind } from "../../ipc/types";
import type { DeviceState } from "../../lib/gear";
import { backupTime, checkFailed, checkFor, failureFor } from "../../lib/backups";
import { linkHandle } from "../../lib/gear";
import { useStore } from "../../store";

/** One device as a page shows it: saved, plugged in, or both. */
export interface DeviceRef {
  /** `pageKey`: the device id, else `link:<handle>`. */
  key: string;
  kind: DeviceKind;
  /** The saved record; null for a device QuadCam does not know yet. */
  device: Device | null;
  /** How it is plugged in now; null when it is not. */
  connected: Connected | null;
  /** Unmounted but still in. */
  unmounted: boolean;
  /** Null when it is not plugged in. */
  state: DeviceState | null;
}

/** A segment of a device page (Overview, Backups, Switches, ...). */
export interface DeviceSegment {
  id: string;
  label: string;
  /** The device kinds that show it. */
  kinds: DeviceKind[] | "all";
  render: (d: DeviceRef) => ReactNode;
}

/** A page under Gear in the sidebar (Bench, Aircraft, Radios, Packs, Flights, Sims,
 *  Firmware, Storage). */
export interface GearPageSlot {
  id: string;
  label: string;
  icon: import("../../components/Icon").IconName;
  /** A boolean settings key: the page shows only while it is on. */
  preview?: string;
  /** A count the sidebar shows beside the label. */
  count?: (s: import("../../store").State) => number;
  render: () => ReactNode;
}

export const gearSlots = {
  /** Staged changes ready for a device. The plug-in bar shows when it is above 0. */
  stagedFor: (deviceId: string): number => useStore.getState().changes.filter((c) => c.device === deviceId && (c.status === "ready" || c.status === "try")).length,
  /** Opens the apply sheet for a device's staged changes. */
  review: (deviceId: string): void => void useStore.getState().openApply(deviceId),
  /** The latest backup's time, from the saved record's `last_backup` id. */
  lastBackup: (d: Device | null): string | null => backupTime(d?.last_backup),
  /** What a connected device needs from the person beyond a name (a failed step, WP4 and
   *  later): a short phrase, or null. */
  attention: (c: Connected): string | null => {
    const s = useStore.getState().gear;
    const f = failureFor(s, linkHandle(c.link));
    if (f) return `${f.step} failed`;
    return checkFailed(checkFor(s, c.id)) ? "Card check failed" : null;
  },
};
