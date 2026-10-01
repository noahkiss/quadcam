// Library-folder and settings actions: rebuild, rename to the date format, reveal.
import { api, errText, openFolder } from "../ipc/api";
import { toast } from "../components/toastStore";
import { ask, store } from "../store";
import { sel } from "../store/settings";
import { makeStrips } from "../events";

const S = () => store.getState();
const n = (k: number, one: string) => `${k} ${one}${k === 1 ? "" : "s"}`;

export async function rebuildLibrary() {
  try {
    const r = await api.rebuild();
    toast(`${n(r.clips, "clip")} and ${n(r.cuts, "cut")} in the library.${r.problems.length ? ` ${n(r.problems.length, "file")} left out.` : ""}`, r.problems.length > 0);
    await S().loadLibrary();
    makeStrips();
  } catch (e) {
    toast(errText(e), true);
  }
}

export function revealLibrary() {
  const out = sel.outputDir(S());
  if (out) openFolder(out).catch((e) => toast(errText(e), true));
}

export async function applyNameFormat(fmt: "YYYY-MM-DD" | "YY.MM.DD") {
  const k = S().lib?.clips.length || 0;
  const ok = await ask(`Rename ${n(k, "clip")} to ${fmt === "YY.MM.DD" ? "26.09.25_name" : "2026-09-25_name"}?`, "Cuts and kept originals are renamed too. Folders stay as they are.", { ok: "Rename" });
  if (!ok) return;
  try {
    await S().saveSetting("nameDateFormat", fmt);
    const r = await api.applyNameFormat();
    toast(`${n(r.renamed.length, "clip")} renamed.${r.failed.length ? ` ${r.failed.length} failed: ${r.failed[0][1]}` : ""}`, r.failed.length > 0);
    await S().loadLibrary();
  } catch (e) {
    toast(errText(e), true);
  }
}
