// The library as the core reports it, plus what the person is looking at: the filter, the
// search, the selection, and the open clip.
import type { StateCreator } from "zustand";
import type { State } from ".";
import { api } from "../ipc/api";
import type { LibClip, LibraryView } from "../ipc/types";
import { visibleClips, type Filter } from "../lib/library";
import { sel } from "./settings";

export type Screen = "library" | "first-run" | "detail" | "gear";

export interface LibrarySlice {
  lib: LibraryView | null;
  filter: Filter;
  query: string;
  selected: ReadonlySet<string>;
  anchor: string | null;
  /** The end of the selection that keys move. */
  cursor: string | null;
  detailId: string | null;
  dTab: "details" | "flight";
  /** Seconds to start the open clip at (a moment picked from the day summary). */
  detailSeek: number | null;
  renaming: string | null;
  /** Reads the library. With `onlyIfChanged` nothing changes unless the clips did. */
  loadLibrary: (onlyIfChanged?: boolean) => Promise<void>;
  setFilter: (f: Filter) => void;
  setQuery: (q: string) => void;
  /** Click semantics: plain replaces, Command toggles, Shift extends from the anchor. */
  select: (id: string, mods?: { meta?: boolean; shift?: boolean }) => void;
  selectAll: () => void;
  clearSelection: () => void;
  openDetail: (id: string, at?: number | null, tab?: "details" | "flight") => void;
  closeDetail: () => void;
  setDTab: (t: "details" | "flight") => void;
  setRenaming: (id: string | null) => void;
}

let libSig = "";

export const createLibrarySlice: StateCreator<State, [], [], LibrarySlice> = (set, get) => ({
  lib: null,
  filter: { group: "all" },
  query: "",
  selected: new Set(),
  anchor: null,
  cursor: null,
  detailId: null,
  dTab: "details",
  detailSeek: null,
  renaming: null,
  loadLibrary: async (onlyIfChanged = false) => {
    let lib: LibraryView;
    try {
      lib = await api.library({});
    } catch (e) {
      console.warn(e);
      lib = { clips: [], unindexed: 0, totals: { clips: 0, bytes: 0, seconds: 0, flying: 0 }, groups: {}, root: sel.outputDir(get()) || "", exists: false, last_import: null, layout: "year_day", place_folders: false };
    }
    const sig = JSON.stringify([lib.root, lib.unindexed, lib.last_import, lib.clips]);
    if (onlyIfChanged && sig === libSig) return;
    libSig = sig;
    const ids = new Set(lib.clips.map((c) => c.id));
    const selected = new Set([...get().selected].filter((id) => ids.has(id)));
    const detailId = get().detailId && ids.has(get().detailId!) ? get().detailId : null;
    set({ lib, selected, detailId });
  },
  setFilter: (filter) => set({ filter, detailId: null, gearPage: null }),
  setQuery: (query) => set({ query }),
  select: (id, mods = {}) => {
    const s = get();
    const ids = visible(s).map((c) => c.id);
    let selected: Set<string>;
    if (mods.meta) {
      selected = new Set(s.selected);
      if (selected.has(id)) selected.delete(id);
      else selected.add(id);
    } else if (mods.shift && s.anchor && ids.includes(s.anchor)) {
      const [a, b] = [ids.indexOf(s.anchor), ids.indexOf(id)].sort((x, y) => x - y);
      selected = new Set(ids.slice(a, b + 1));
    } else {
      selected = new Set([id]);
    }
    set({ selected, cursor: id, anchor: !mods.shift || !s.anchor ? id : s.anchor });
  },
  selectAll: () => set((s) => ({ selected: new Set(visible(s).map((c) => c.id)) })),
  clearSelection: () => set({ selected: new Set() }),
  openDetail: (id, at = null, tab) => set((s) => ({ gearPage: null, detailId: id, selected: new Set([id]), cursor: id, detailSeek: at, dTab: tab || s.dTab })),
  closeDetail: () => set({ detailId: null, detailSeek: null }),
  setDTab: (dTab) => set({ dTab }),
  setRenaming: (renaming) => set({ renaming }),
});

/** The clips the library shows, in the one order. */
export function visible(s: State): LibClip[] {
  return visibleClips(s.lib, s.filter, s.query, sel.sort(s));
}

export const libClip = (s: State, id: string | null | undefined) => (id ? s.lib?.clips.find((c) => c.id === id) : undefined);

/** The selected clips that still exist. */
export const selectedIds = (s: State) => [...s.selected].filter((id) => libClip(s, id));

/** Which screen shows: a Gear page, the open clip, the first-run screen for an empty
 *  library, else the grid. */
export function screenOf(s: State): Screen {
  if (s.gearPage) return "gear";
  if (s.detailId && libClip(s, s.detailId)) return "detail";
  const empty = !s.lib?.clips.length && !((s.lib?.unindexed ?? 0) > 0);
  return empty && s.lib ? "first-run" : "library";
}

/** The clips a Clip menu item acts on: the open clip, else the selection. */
export const targets = (s: State) => (screenOf(s) === "detail" && s.detailId ? [s.detailId] : selectedIds(s));
