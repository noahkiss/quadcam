// The native menu bar's side in the web view (plan 3.3): runs each item's action, the same
// functions the keys call, and keeps items enabled and checked to match the screen. The
// menu itself is built in src-tauri/src/menu.rs.
import { useEffect } from "react";
import { api, reveal } from "../ipc/api";
import { on } from "../ipc/events";
import * as lib from "../actions/library";
import { albumAction } from "../lib/library";
import { importAction, openFolderAction } from "../actions/session";
import { store, type State } from "../store";
import { libClip, screenOf, targets, visible } from "../store/library";
import { sel } from "../store/settings";
import { dialogOpen } from "../store/ui";

const S = () => store.getState();
const typingIn = () => (document.activeElement as Element | null)?.closest?.("input, textarea, select, [contenteditable]") as HTMLInputElement | null;

export const MENU_ACTIONS: Record<string, () => void> = {
  settings: () => S().openSettings("library"),
  import: () => importAction(),
  "open-folder": () => openFolderAction(),
  "reveal-library": () => {
    const out = sel.outputDir(S());
    if (out) reveal(out).catch(() => {});
  },
  undo: () => (typingIn() ? document.execCommand("undo") : lib.undoRedo("undo")),
  redo: () => (typingIn() ? document.execCommand("redo") : lib.undoRedo("redo")),
  "select-all": () => {
    const t = typingIn();
    if (t?.select) return t.select();
    if (screenOf(S()) === "library") S().selectAll();
  },
  pick: () => lib.rate(targets(S()), null, "pick"),
  reject: () => lib.rate(targets(S()), null, "reject"),
  unflag: () => lib.rate(targets(S()), null, "none"),
  rename: () => S().setRenaming(targets(S())[0] ?? null),
  "edit-details": () => {
    const id = targets(S())[0];
    if (id) S().openDetail(id, null, "details");
  },
  "reveal-clip": () => {
    const id = targets(S())[0];
    if (id) lib.revealClip(id);
  },
  share: () => lib.shareClips(targets(S())),
  "add-photos": () => lib.addLibToPhotos(targets(S())),
  trash: () => lib.trashClips(targets(S())),
  "view-grid": () => lib.setView("grid"),
  "view-list": () => lib.setView("list"),
  "thumb-bigger": () => lib.setThumbSize(sel.thumbSize(S()) + 1),
  "thumb-smaller": () => lib.setThumbSize(sel.thumbSize(S()) - 1),
  "prev-clip": () => lib.stepClip(-1),
  "next-clip": () => lib.stepClip(1),
};
for (let i = 0; i <= 5; i++) MENU_ACTIONS[`rate-${i}`] = () => lib.rate(targets(S()), i);
for (const k of ["date", "rating", "duration", "name"] as const) MENU_ACTIONS[`sort-${k}`] = () => lib.setSort(k);

/** Which items are on and checked for the current state. */
export function menuState(s: State, typing: boolean) {
  const free = !dialogOpen(s) && !s.importOpen;
  const screen = screenOf(s);
  const libOn = free && screen === "library";
  const n = targets(s).length;
  const clipOn = free && (screen === "library" || screen === "detail") && n > 0;
  const view = sel.libView(s);
  const thumb = sel.thumbSize(s);
  const album = albumOf(s);
  const enabled: Record<string, boolean> = {
    settings: free,
    import: free && !!s.env?.tools,
    "open-folder": free && !!s.env?.tools,
    "reveal-library": true,
    undo: typing || (free && s.done.length > 0),
    redo: typing || (free && s.undone.length > 0),
    "select-all": typing || libOn,
    pick: clipOn,
    reject: clipOn,
    unflag: clipOn,
    share: clipOn,
    "add-photos": clipOn && !!album,
    trash: clipOn,
    rename: clipOn && n === 1,
    "edit-details": clipOn && n === 1,
    "reveal-clip": clipOn && n === 1 && !!libClip(s, targets(s)[0]),
    "view-grid": libOn,
    "view-list": libOn,
    "thumb-bigger": libOn && view !== "list" && thumb < 5,
    "thumb-smaller": libOn && view !== "list" && thumb > 1,
    "prev-clip": free && (screen === "detail" || (libOn && visible(s).length > 0)),
    "next-clip": free && (screen === "detail" || (libOn && visible(s).length > 0)),
  };
  for (let i = 0; i <= 5; i++) enabled[`rate-${i}`] = clipOn;
  const checked: Record<string, boolean> = { "view-grid": view !== "list", "view-list": view === "list" };
  const key = sel.sort(s).key;
  for (const k of ["date", "rating", "duration", "name"]) {
    enabled[`sort-${k}`] = libOn;
    checked[`sort-${k}`] = key === k;
  }
  return { enabled, checked, albumItem: album };
}

const albumOf = (s: State) => albumAction(sel.photosAlbum(s));

let last = "";
/** Sends the items' state when it changed. */
export function syncMenu() {
  const { enabled, checked, albumItem } = menuState(S(), !!typingIn());
  const key = JSON.stringify([enabled, checked, albumItem]);
  if (key === last) return;
  last = key;
  api.menuState(enabled, checked, albumItem).catch(() => {});
}

/** Wires the menu: item clicks run actions; state changes and focus moves resync. */
export function useMenu() {
  useEffect(() => {
    let off: (() => void) | undefined;
    on("menu", (id) => MENU_ACTIONS[id]?.()).then((f) => (off = f));
    const unsub = store.subscribe(() => queueMicrotask(syncMenu));
    const onFocus = () => queueMicrotask(syncMenu);
    document.addEventListener("focusin", onFocus);
    document.addEventListener("focusout", onFocus);
    syncMenu();
    return () => {
      off?.();
      unsub();
      document.removeEventListener("focusin", onFocus);
      document.removeEventListener("focusout", onFocus);
    };
  }, []);
}
