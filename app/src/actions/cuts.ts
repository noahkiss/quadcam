// Sends a cut list. When the core answers that exported files would lose their cut, asks
// the person what happens to them, then sends it again with the answer.
import { api, errText } from "../ipc/api";
import type { CutChange, RemovedCuts, Span } from "../ipc/types";
import { toast } from "../components/toastStore";
import { askRemoved, store } from "../store";

export async function applyCuts(call: (cuts: Span[], removed: RemovedCuts | null) => Promise<CutChange>, cuts: Span[]) {
  let r = await call(cuts, null);
  if (r.status === "confirm") {
    const decision = await askRemoved(r.files);
    if (!decision) return null;
    r = await call(cuts, decision);
  }
  return r;
}

export async function setLibraryCuts(id: string, cuts: Span[]) {
  try {
    const r = await applyCuts((k, removed) => api.libraryCuts(id, k, removed), cuts);
    if (r?.status === "applied" && (r.trashed.length || r.kept.length)) toast(r.trashed.length ? "Cut file moved to the Trash." : "Cut file kept as its own clip.");
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function saveLibraryCuts(id: string) {
  try {
    const made = await api.exportCuts(id);
    toast(`${made.length} cut${made.length === 1 ? "" : "s"} saved.`);
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function setSessionCuts(id: number, cuts: Span[]) {
  try {
    await applyCuts((k, removed) => api.sessionCuts(id, k, removed), cuts);
  } catch (e) {
    toast(errText(e), true);
  }
}

/** Remembers a typed value for the field's suggestions (12 per kind). */
export function remember(kind: "keywords" | "authors" | "notes", value: string) {
  value = (value || "").trim();
  if (!value) return;
  const s = store.getState();
  const recents = s.values.recents || {};
  const list = [value, ...(recents[kind] || []).filter((v) => v !== value)].slice(0, 12);
  s.saveSetting("recents", { ...recents, [kind]: list });
}
