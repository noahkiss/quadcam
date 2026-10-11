// The app's own update: the core's state (`update_status`, then `update-changed` events) and
// the version the person put off with Later this run.
import type { StateCreator } from "zustand";
import type { State } from ".";
import type { UpdateStatus } from "../ipc/types";

export interface UpdateSlice {
  update: UpdateStatus | null;
  /** The version Later hid. A newer one, or Check for Updates, shows the banner again. */
  updateLater: string | null;
  setUpdate: (u: UpdateStatus | null) => void;
  laterUpdate: () => void;
  clearLater: () => void;
}

export const createUpdateSlice: StateCreator<State, [], [], UpdateSlice> = (set, get) => ({
  update: null,
  updateLater: null,
  setUpdate: (update) => set({ update }),
  laterUpdate: () => set({ updateLater: get().update?.version ?? null }),
  clearLater: () => set({ updateLater: null }),
});

/** What the banner shows, or null for no banner. */
export function updateBanner(s: Pick<State, "update" | "updateLater">) {
  const u = s.update;
  if (!u?.version || u.version === s.updateLater) return null;
  if (u.phase === "available" || u.phase === "downloading" || u.phase === "ready") return u;
  // A failed download keeps the version, so the banner offers it again.
  if (u.phase === "failed") return u;
  return null;
}

/** Settings > Advanced's line on updates. */
export function updateLine(u: UpdateStatus | null): string {
  if (!u) return "";
  switch (u.phase) {
    case "idle":
      return `QuadCam ${u.current}. Not checked yet.`;
    case "checking":
      return "Checking for updates…";
    case "current":
      return `QuadCam ${u.current} is the newest version.`;
    case "available":
      return `QuadCam ${u.version} is available.`;
    case "downloading":
      return `Downloading QuadCam ${u.version}…`;
    case "ready":
      return `QuadCam ${u.version} installs when QuadCam restarts.`;
    case "failed":
      return u.error || "The last check failed.";
  }
}
