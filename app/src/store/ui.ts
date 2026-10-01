// Overlays: the Settings sheet, the question dialog, the removed-cuts question, the clip
// menu and the drop target. A question is a promise the dialog resolves.
import type { StateCreator } from "zustand";
import type { State } from ".";

export type SettingsSection = "library" | "import" | "aircraft" | "places" | "photos" | "advanced";

export interface AskRequest {
  title: string;
  text?: string;
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
  formatConfirm: { text: string; agent: boolean } | null;
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
  openSettings: (section = "library") => set({ settingsOpen: section }),
  closeSettings: () => set({ settingsOpen: null }),
  setMenu: (menuAt) => set({ menuAt }),
  setDropping: (dropping) => set({ dropping }),
  setFormatConfirm: (formatConfirm) => set({ formatConfirm }),
});

/** Any modal is open (the import sheet does not count). */
export const dialogOpen = (s: State) => !!(s.settingsOpen || s.askReq || s.removedReq || s.formatConfirm);
