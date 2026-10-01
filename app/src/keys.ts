// The app's keys, ported from the legacy keydown handler. A text field, a select, or a
// dialog other than the import sheet keeps its own keys.
import { useEffect } from "react";
import * as lib from "./actions/library";
import { store } from "./store";
import { screenOf, selectedIds, visible } from "./store/library";
import { sel } from "./store/settings";
import { dialogOpen } from "./store/ui";
import { gridColumns } from "./views/Library/LibraryView";

const S = () => store.getState();

/** Keys a screen handles before the library's: the import review, the open clip. */
export const screenKeys: {
  sheet?: (e: KeyboardEvent) => boolean;
  detail?: (e: KeyboardEvent) => boolean;
} = {};

/** Rate, flag and trash keys for `ids`. True when handled. */
export function clipKeys(e: KeyboardEvent, ids: string[]) {
  if (e.metaKey && e.key === "Backspace") {
    lib.trashClips(ids);
    return true;
  }
  if (e.metaKey || e.ctrlKey || e.altKey) return false;
  const k = e.key.toLowerCase();
  if (/^[0-5]$/.test(k)) lib.rate(ids, +k);
  else if (k === "p") lib.rate(ids, null, "pick");
  else if (k === "x") lib.rate(ids, null, "reject");
  else if (k === "u") lib.rate(ids, null, "none");
  else return false;
  return true;
}

function onKey(e: KeyboardEvent) {
  const s = S();
  const t = e.target as Element | null;
  if (s.importOpen && screenKeys.sheet?.(e)) return e.preventDefault();
  const typing = t?.closest?.("input, select, textarea, [contenteditable]");
  if (typing || dialogOpen(s) || s.menuAt) return;
  if (s.importOpen) return;
  if (e.metaKey && e.key.toLowerCase() === "z") {
    lib.undoRedo(e.shiftKey ? "redo" : "undo");
    return e.preventDefault();
  }
  const screen = screenOf(s);
  if (e.key === "Escape") {
    if (screen === "detail") return s.closeDetail();
    if (screen === "library" && s.selected.size) return s.clearSelection();
  }
  if (screen === "detail") {
    if (screenKeys.detail?.(e)) return e.preventDefault();
    if (clipKeys(e, [s.detailId!])) e.preventDefault();
    return;
  }
  if (screen !== "library") return;
  if (e.metaKey && e.key === "a") {
    s.selectAll();
    return e.preventDefault();
  }
  if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) {
    const list = visible(s).map((c) => c.id);
    if (!list.length) return;
    const i = Math.max(0, list.indexOf(s.cursor ?? s.anchor ?? ""));
    const cols = sel.libView(s) === "list" ? 1 : gridColumns();
    const step = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -cols, ArrowDown: cols } as Record<string, number>)[e.key];
    const next = list[Math.min(list.length - 1, Math.max(0, i + step))];
    s.select(next, { shift: e.shiftKey });
    focusClip(next);
    return e.preventDefault();
  }
  const ids = selectedIds(s);
  if (!ids.length) return;
  const cur = s.cursor && ids.includes(s.cursor) ? s.cursor : ids[0];
  if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
    const r = document.querySelector(`[data-id="${CSS.escape(cur)}"]`)?.getBoundingClientRect();
    if (r) s.setMenu({ id: cur, x: r.left + 24, y: r.top + 24 });
    return e.preventDefault();
  }
  if (e.key === "Enter") {
    s.setRenaming(ids[0]);
    return e.preventDefault();
  }
  if (e.key === " " || e.key === "t") {
    s.openDetail(ids[0]);
    return e.preventDefault();
  }
  if (e.metaKey && e.key === "i") {
    s.openDetail(ids[0], null, "details");
    return e.preventDefault();
  }
  if (e.metaKey && e.key === "r") {
    lib.revealClip(ids[0]);
    return e.preventDefault();
  }
  if (clipKeys(e, ids)) e.preventDefault();
}

/** Moves focus to a clip once its card or row is on screen. */
export function focusClip(id: string) {
  let tries = 0;
  const go = () => {
    const el = document.querySelector(`[data-id="${CSS.escape(id)}"]`) as HTMLElement | null;
    if (el) el.focus({ preventScroll: false });
    else if (tries++ < 5) requestAnimationFrame(go);
  };
  requestAnimationFrame(go);
}

export function useKeys() {
  useEffect(() => {
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
}
