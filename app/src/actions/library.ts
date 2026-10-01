// Library actions: each one calls the core and, when it can be undone, records its step in
// one place. The toolbar, the keys, the clip menu and the native menu all call these.
import { api, errText, reveal } from "../ipc/api";
import type { Flag, LibEdit } from "../ipc/types";
import { toast } from "../components/toastStore";
import { albumAction, nextSort, type SortKey } from "../lib/library";
import { ask, store } from "../store";
import { libClip, screenOf, visible } from "../store/library";
import { sel } from "../store/settings";

const S = () => store.getState();
const n = (k: number, one: string, many = `${one}s`) => `${k} ${k === 1 ? one : many}`;

export async function rate(ids: string[], rating: number | null, flag?: Flag | null) {
  if (!ids.length) return;
  const before = ids
    .map((id) => libClip(S(), id))
    .filter((c) => !!c)
    .map((c) => ({ id: c.id, rating: c.rating || 0, flag: c.flag || "none" }));
  const apply = () => api.rate(ids, rating ?? null, flag ?? null);
  try {
    await apply();
    S().pushStep({ label: flag ? "Flag" : "Rating", redo: apply, undo: () => restoreRatings(before) });
  } catch (e) {
    toast(errText(e), true);
  }
}

/** Puts ratings and flags back, one core call per distinct pair. */
async function restoreRatings(before: { id: string; rating: number; flag: Flag }[]) {
  const groups = new Map<string, { rating: number; flag: Flag; ids: string[] }>();
  for (const b of before) {
    const k = `${b.rating}|${b.flag}`;
    if (!groups.has(k)) groups.set(k, { rating: b.rating, flag: b.flag, ids: [] });
    groups.get(k)!.ids.push(b.id);
  }
  for (const g of groups.values()) await api.rate(g.ids, g.rating, g.flag);
}

export async function trashClips(ids: string[]) {
  if (!ids.length) return;
  const name = ids.length === 1 ? libClip(S(), ids[0])?.name || "the clip" : `${ids.length} clips`;
  const ok = await ask(`Move ${name} to the Trash?`, "Cuts and kept originals go too.", { ok: "Move to Trash", danger: true });
  if (!ok) return;
  try {
    const r = await api.trash(ids);
    if (r.failed.length) toast(`${n(r.failed.length, "file")} did not move: ${r.failed[0][1]}`, true);
    else toast(`${n(r.trashed.length, "file")} moved to the Trash.`);
    if (S().detailId && ids.includes(S().detailId!)) S().closeDetail();
    if (r.moved.length) {
      let moved = r.moved;
      S().pushStep({
        label: "Move to Trash",
        undo: () => api.untrash(moved),
        redo: async () => {
          moved = (await api.trash(ids)).moved;
        },
      });
    }
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function renameClip(id: string, name: string) {
  const c = libClip(S(), id);
  name = name.trim();
  if (!c || !name || name === (c.title || c.name)) return;
  const old = c.title || c.name;
  try {
    await api.rename(id, name);
    S().pushStep({ label: "Rename", undo: () => api.rename(id, old), redo: () => api.rename(id, name) });
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function editClip(id: string, patch: LibEdit) {
  const c = libClip(S(), id);
  const before: LibEdit = {};
  if (c) {
    if ("note" in patch) before.note = c.note || "";
    if ("keywords" in patch) before.keywords = c.keywords || [];
    if ("author" in patch) before.author = c.author || "";
    if ("place" in patch || "location" in patch) {
      if (c.location) before.location = { ...c.location, name: c.place || c.location.name || null };
      else before.place = "";
    }
  }
  try {
    await api.edit(id, patch);
    if (c) S().pushStep({ label: "Edit", undo: () => api.edit(id, before), redo: () => api.edit(id, patch) });
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function addLibToPhotos(ids: string[]) {
  try {
    const r = await api.libraryPhotos(ids, sel.photosAlbum(S()) ?? "");
    const where = r.album ? `the ${r.album} album` : "Photos";
    if (r.failed.length) toast(`${r.added.length} added to ${where}; ${r.failed.length} failed: ${r.failed[0][1]}`, true);
    else toast(`${n(r.added.length, "file")} added to ${where}.`);
  } catch (e) {
    toast(errText(e), true);
  }
}

/** The macOS Share menu for clips and their cuts, next to `anchor`. */
export async function shareClips(ids: string[], anchor?: Element | null) {
  const s = S();
  const root = s.lib?.root || "";
  const files = ids
    .map((id) => libClip(s, id))
    .filter((c) => !!c)
    .flatMap((c) => [c.file, ...c.cuts.map((k) => `${root}/${k.path}`)]);
  if (!files.length) return;
  const el = anchor || document.querySelector(screenOf(s) === "detail" ? "[data-share]" : `[data-id="${CSS.escape(ids[0])}"]`);
  const r = el?.getBoundingClientRect() || { left: innerWidth / 2, top: 60, width: 1, height: 1 };
  try {
    await api.share(files, { x: r.left, y: r.top, w: r.width, h: r.height });
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function rescan(id: string) {
  try {
    await api.rescan(id);
    toast("Dead air found again.");
  } catch (e) {
    toast(errText(e), true);
  }
}

export function revealClip(id: string) {
  const c = libClip(S(), id);
  if (c) reveal(c.file).catch((e) => toast(errText(e), true));
}

export function setSort(key: SortKey) {
  S().saveSetting("libSort", nextSort(sel.sort(S()), key));
}

export function setView(v: "grid" | "list") {
  S().saveSetting("libView", v);
}

export function setThumbSize(size: number) {
  S().saveSetting("thumbSize", Math.min(5, Math.max(1, size)));
}

export async function undoRedo(which: "undo" | "redo") {
  const s = S();
  const label = (which === "undo" ? s.done : s.undone).at(-1)?.label || "";
  try {
    const step = await s[which]();
    if (step) toast(`${which === "undo" ? "Undo" : "Redo"} ${label.toLowerCase()}`);
  } catch (e) {
    toast(errText(e), true);
  }
}

/** Previous or next clip in the library's order: in the open clip it opens that clip,
 * in the library it moves the selection. */
export function stepClip(delta: number) {
  const s = S();
  const list = visible(s).map((c) => c.id);
  if (!list.length) return;
  if (screenOf(s) === "detail") {
    const next = list[list.indexOf(s.detailId!) + delta];
    if (next != null) s.openDetail(next);
    return;
  }
  const i = list.indexOf(s.cursor ?? s.anchor ?? "");
  const next = list[Math.min(list.length - 1, Math.max(0, i < 0 ? 0 : i + delta))];
  s.select(next);
  scrollToClip(next);
}

export function scrollToClip(id: string) {
  requestAnimationFrame(() => document.querySelector(`[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "nearest" }));
}

export const albumLabel = (count = 1) => albumAction(sel.photosAlbum(S()), count);
