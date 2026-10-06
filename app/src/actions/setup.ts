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

/** Matches radio logs to library clips, shows what matched, and writes it on OK. */
export async function matchLibraryLogs() {
  try {
    const r = await api.matchLogs(false);
    const count = (b: string) => r.clips.filter((c) => c.badge === b).length;
    const matched = count("matched");
    const lines = r.clips
      .filter((c) => c.badge !== "unmatched")
      .map((c) => `${c.path.split("/").pop()}: ${c.badge}${c.reason ? ` (${c.reason})` : ""}`);
    const text = [...r.warnings, ...lines, `${n(count("unmatched"), "clip")} without a match.`].join("\n");
    if (matched === 0) {
      await ask(`No clip matched a radio log.`, text, { ok: "OK" });
      return;
    }
    const ok = await ask(`${n(matched, "clip")} matched, ${count("likely")} likely`, `${text}\n\nWrite the flight numbers and moments into the matched clips? Dates and names stay as they are.`, { ok: "Write" });
    if (!ok) return;
    const w = await api.matchLogs(true);
    toast(`${n(w.clips.filter((c) => c.applied).length, "clip")} updated from radio logs.`);
    await S().loadLibrary();
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
