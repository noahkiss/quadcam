// Overlays: the Settings sheet, the question dialog, the removed-cuts question, the clip
// menu and the drop target. A question is a promise the dialog resolves.
import type { StateCreator } from "zustand";
import type { State } from ".";

import type { ReactNode } from "react";
import type { FormatRequest } from "../ipc/types";

export type SettingsSection = "library" | "import" | "aircraft" | "places" | "photos" | "gear" | "modules" | "advanced";

export interface AskRequest {
  title: string;
  text?: string;
  /** Content under the text (links, a table). */
  body?: ReactNode;
  /** Show a text field with this starting value. */
  input?: string;
  ok?: string;
  danger?: boolean;
  resolve: (v: string | true | null) => void;
}

export interface UiSlice {
  settingsOpen: SettingsSection | null;
  askReq: AskRequest | null;
  removedReq: { files: string[]; resolve: (v: "keep" | "trash" | null) => void } | null;
  menuAt: { id: string; x: number; y: number } | null;
  dropping: boolean;
  /** The erase confirmation (the person's own, or an agent's). */
  formatConfirm: { text: string; agent: boolean; /** Card prep: the request the Erase click sends. */ prep?: FormatRequest; /** The format step: the request of the plan the dialog shows. */ erase?: FormatRequest } | null;
  /** The third-party notices window. */
  noticesOpen: boolean;
  /** The sidebar shows over the page (a narrow window only). */
  sidebarOpen: boolean;
  setSidebarOpen: (on: boolean) => void;
  setNoticesOpen: (on: boolean) => void;
  openSettings: (section?: SettingsSection) => void;
  closeSettings: () => void;
  setMenu: (m: UiSlice["menuAt"]) => void;
  setDropping: (on: boolean) => void;
  setFormatConfirm: (c: UiSlice["formatConfirm"]) => void;
}

export const createUiSlice: StateCreator<State, [], [], UiSlice> = (set) => ({
  settingsOpen: null,
  askReq: null,
  removedReq: null,
  menuAt: null,
  dropping: false,
  formatConfirm: null,
  noticesOpen: false,
  sidebarOpen: false,
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
  setNoticesOpen: (noticesOpen) => set({ noticesOpen }),
  openSettings: (section = "library") => set({ settingsOpen: section }),
  closeSettings: () => set({ settingsOpen: null }),
  setMenu: (menuAt) => set({ menuAt }),
  setDropping: (dropping) => set({ dropping }),
  setFormatConfirm: (formatConfirm) => set({ formatConfirm }),
});

/** Any modal is open (the import sheet does not count). */
export const dialogOpen = (s: State) => !!(s.settingsOpen || s.askReq || s.removedReq || s.formatConfirm || s.noticesOpen || s.applySheet);
