// Import actions: loading a card or folder, review edits, export, Photos, eject and the
// format step. Ported from the legacy app.js; the core owns the session throughout.
import { api, errText, pickFolder } from "../ipc/api";
import { on } from "../ipc/events";
import type { FormatPlan, PlanPatch, Session } from "../ipc/types";
import { toast } from "../components/toastStore";
import { fmtBytes, tilde } from "../lib/format";
import { ask, store } from "../store";
import { planOf, resultOf, sessionLeft, unfinished, type Step } from "../store/session";
import { sel } from "../store/settings";

const S = () => store.getState();

/** Opens the sheet at `step`, or where the session stands. */
export function openImport(step?: Step) {
  const s = S();
  s.setImportOpen(true);
  setStep(step ?? (s.session ? (s.session.results.length && !sessionLeft(s.session) ? "finish" : "review") : "load"));
}

export function setStep(step: Step) {
  S().setStep(step);
  if (step === "finish") checkFormat();
}

/** The toolbar's Import…: the unfinished import, else the card, else a folder. */
export async function importAction() {
  const s = S();
  if (unfinished(s.session)) return openImport();
  const card = s.volumes.find((v) => v.is_card);
  if (card) return importFrom(card.mount);
  return openFolderAction();
}

export async function openFolderAction() {
  const p = await pickFolder("Folder with DVR clips");
  if (p) importFrom(p);
}

export async function importFrom(mount: string) {
  const s = S();
  const vol = s.volumes.find((v) => v.mount === mount);
  if (s.session && vol && s.session.card?.volume_uuid === vol.info.volume_uuid) return openImport();
  if (!(await replaceOk())) return;
  loadSource(mount);
}

/** Asks before an unexported import is replaced. True to go ahead. */
async function replaceOk() {
  const s = S().session;
  if (!(s && sessionLeft(s) && s.results.length === 0 && s.clips.length)) return true;
  return !!(await ask("Replace the unfinished import?", `${s.clips.length} clips are loaded and not in the library. Their names, dates and cuts are lost.`, { ok: "Replace", danger: true }));
}

/** A folder or AVI files dropped on the window: the folder loads like a card. */
export async function importDropped(paths: string[]) {
  const s = S();
  if (!paths.length || s.busy || !s.env?.tools) return;
  if (!(await replaceOk())) return;
  loadSource(paths.length === 1 ? paths[0] : `${paths.length} files`, paths);
}

export async function loadSource(path: string, dropped: string[] | null = null) {
  if (S().busy) return;
  store.setState({ busy: true, session: null, staging: { phase: "stage", index: 0, total: 0, done: 0, size: 0 }, loadingFrom: path });
  openImport("load");
  try {
    const s = dropped ? await api.loadDropped(dropped) : await api.loadSource(path);
    if (!s.clips.length) {
      toast("No clips found there.", true);
      S().setImportOpen(false);
      return;
    }
    S().setSession(s);
    store.setState({ selectedClip: s.clips[0]?.id ?? null });
    setStep("review");
  } catch (e) {
    toast(errText(e), true);
    const s = await api.getSession();
    S().setSession(s);
    if (s) setStep("review");
    else S().setImportOpen(false);
  } finally {
    store.setState({ busy: false, staging: null, loadingFrom: null });
    S().endTask("stage");
    S().endTask("analyse");
  }
}

/** A new session from the core: after an agent's change, an export or an erase. */
export function sessionChanged(s: Session | null) {
  const st = S();
  const hadCard = !!st.session?.card;
  const oldResults = st.session?.results.length || 0;
  if (hadCard && s && !s.card && s.warnings.some((w) => w.includes("erased"))) {
    toast("Card erased and ejected.");
    st.setSession(null);
    api.clearSession().catch(() => {});
    st.refreshVolumes();
    st.setImportOpen(false);
    return;
  }
  st.setSession(s);
  if (!s || !st.importOpen || st.busy) return;
  // Results that arrive from an agent's export open the finish step.
  if (s.results.length && s.results.length !== oldResults && st.step === "review") setStep("finish");
}

export async function edit(patch: PlanPatch) {
  try {
    S().setSession(await api.editPlan(patch));
  } catch (e) {
    toast(errText(e), true);
  }
}

/** One core call for many clips. */
export async function editAll(patches: PlanPatch[]) {
  if (!patches.length) return true;
  try {
    S().setSession(await api.editPlans(patches));
    return true;
  } catch (e) {
    toast(errText(e), true);
    return false;
  }
}

/** The same change for every clip that is not skipped. */
export function forAll(fn: (id: number) => Omit<PlanPatch, "id">) {
  const s = S().session;
  if (!s) return;
  return editAll(s.clips.filter((c) => !planOf(s, c.id)?.skip).map((c) => ({ id: c.id, ...fn(c.id) })));
}

export function toggleSkip(id: number | null) {
  const s = S().session;
  const c = s?.clips.find((x) => x.id === id);
  const p = s && id != null ? planOf(s, id) : undefined;
  if (!c || !p || c.status === "empty" || c.stage_error) return;
  edit({ id: c.id, skip: !p.skip });
}

export function stepReview(delta: number) {
  const s = S();
  const ids = s.session?.clips.map((c) => c.id) || [];
  const next = ids[ids.indexOf(s.selectedClip ?? -1) + delta];
  if (next == null) return;
  s.selectClip(next);
  requestAnimationFrame(() => document.querySelector(`[data-clip="${next}"]`)?.scrollIntoView({ block: "nearest" }));
}

/** Copies the clip's profile, location, keywords and author to every other clip. */
export async function metaToAll(id: number) {
  const s = S().session;
  const p = s && planOf(s, id);
  if (!s || !p) return;
  const m = p.meta;
  const patch = { profile: m.profile || "", keywords: m.keywords || [], author: m.author || "", ...(m.location ? { location: m.location } : { place: "" }) };
  if (await editAll(s.clips.filter((c) => c.id !== id).map((c) => ({ id: c.id, ...patch })))) toast("Applied to every clip.");
}

export async function savePlace(id: number, name: string) {
  const s = S();
  const l = s.session && planOf(s.session, id)?.meta.location;
  name = name.trim();
  if (!l || !name) return toast("Type a name for the place first.", true);
  if (sel.places(s).some((x) => x.name.toLowerCase() === name.toLowerCase())) return toast(`A place named ${name} exists already.`, true);
  try {
    await api.placeSave(name, l.lat, l.lon);
  } catch (e) {
    return toast(errText(e), true);
  }
  await S().loadSettings();
  await edit({ id, place: name });
}

export async function setLogDir(dir: string | null) {
  await S().saveSetting("logDir", dir);
  if (S().session) S().setSession(await api.planDates(dir, null));
  if (dir) toast(`Radio logs: ${tilde(dir, S().home)}`);
}

export async function pickLogs() {
  const p = await pickFolder("Radio log folder (LOGS or the radio's root)", sel.logDir(S()));
  if (p) setLogDir(p);
}

export async function setLogDay(day: string) {
  S().setSession(await api.planDates(sel.logDir(S()), day));
}

export async function runExport() {
  const st = S();
  const out = sel.outputDir(st);
  if (!out) return toast("Choose a library folder in Settings first.", true);
  // A field that still has focus commits first.
  (document.activeElement as HTMLElement | null)?.blur?.();
  await new Promise((r) => setTimeout(r, 0));
  const plans = S().session!.plans;
  const todo = plans.filter((p) => !p.skip).length;
  if (!todo && !(await ask("Every clip is skipped", "Record them all as skipped?", { ok: "Record" }))) return;
  store.setState({ busy: true, progress: {}, exportTotal: todo, exportDone: 0 });
  setStep("export");
  const offProgress = await on("import-progress", (p) => {
    const frac = p.duration ? Math.min(1, p.seconds / p.duration) : 0;
    store.setState((s) => ({ progress: { ...s.progress, [p.id]: frac } }));
    S().setTask("export", S().exportDone, todo);
  });
  const offResult = await on("import-result", (r) => {
    store.setState((s) => {
      const progress = { ...s.progress };
      delete progress[r.id];
      const session = s.session ? { ...s.session, results: [...s.session.results.filter((x) => x.id !== r.id), r] } : s.session;
      return { progress, session, exportDone: s.exportDone + (r.outcome !== "skipped" ? 1 : 0) };
    });
  });
  try {
    await api.importClips({ output_dir: out, format: sel.format(st), encoder: sel.encoder(st), keep_originals: sel.keepOriginals(st), add_time: sel.addTime(st) });
    S().setSession(await api.getSession());
    store.setState({ busy: false });
    setStep("finish");
    await S().loadLibrary();
    api.strips().catch(() => {});
  } catch (e) {
    toast(errText(e), true);
    store.setState({ busy: false });
    setStep("review");
  } finally {
    offProgress();
    offResult();
    store.setState({ busy: false, progress: {} });
    S().endTask("export");
  }
}

/** Adds the session's verified outputs to Photos. */
export async function addToPhotos(album: string) {
  const s = S().session;
  if (!s) return;
  const ids = s.results.filter((r) => r.outcome === "verified" && !s.in_photos.includes(r.id)).map((r) => r.id);
  if (!ids.length) return toast("Already in Photos.");
  store.setState({ photosStatus: "Adding to Photos…" });
  try {
    const r = await api.addToPhotos(ids, album.trim() || null);
    const where = r.album ? `the ${r.album} album` : "your Photos library";
    const msg = r.failed.length ? `${r.added.length} added to ${where}; ${r.failed.length} failed: ${r.failed[0][1]}` : `${r.added.length} added to ${where}.`;
    store.setState({ photosStatus: msg });
    toast(msg, !!r.failed.length);
  } catch (e) {
    store.setState({ photosStatus: errText(e) });
    toast(errText(e), true);
  }
  S().setSession(await api.getSession());
}

export async function eject() {
  try {
    await api.eject(null);
    toast("Card ejected.");
    S().refreshVolumes();
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function doneImport() {
  S().setImportOpen(false);
  store.setState({ filter: { group: "last_import" }, detailId: null });
  await S().loadLibrary();
}

export async function startOver() {
  const st = S();
  if (st.busy || !st.session) return;
  const left = sessionLeft(st.session);
  const ok = await ask(
    "Start over?",
    left ? `${left} clip${left > 1 ? "s are" : " is"} not in the library yet; their names, dates and cuts are lost. Clips already in the library stay.` : "This clears the loaded clips. Clips already in the library stay.",
    { ok: "Start over", danger: true },
  );
  if (!ok) return;
  try {
    await api.clearSession();
    st.setSession(null);
    st.setImportOpen(false);
    st.refreshVolumes();
  } catch (e) {
    toast(errText(e), true);
  }
}

// ---------- format ----------

/** Why the card may not be erased yet, or null when every clip is safe. */
export function formatBlocker(s: Session | null): string | null {
  if (!s?.card) return "Clips came from a folder, not a card.";
  if (!s.results.length) return "Add the clips to the library first.";
  for (const c of s.clips) {
    if (c.stage_error) return `${c.name} did not copy off the card.`;
    const r = resultOf(s, c.id);
    if (!r) return `${c.name} is not in the library.`;
    if (r.outcome === "failed") return `${c.name} was not added to the library.`;
  }
  return null;
}

/** Runs the core's format guards for the finish step. */
export async function checkFormat() {
  const s = S().session;
  store.setState({ formatPlan: null, formatError: null });
  if (!s?.card || formatBlocker(s)) return;
  try {
    store.setState({ formatPlan: await api.formatPlan(S().formatLabelDraft || null) });
  } catch (e) {
    store.setState({ formatError: errText(e) });
  }
}

export function confirmText(plan: FormatPlan) {
  const media = plan.media_name ? `, ${plan.media_name}` : "";
  const n = plan.clip_count;
  return `Erase ${plan.disk} (${plan.volume_name || "untitled"}, ${fmtBytes(plan.size)}${media}) and delete ${n} clip${n === 1 ? "" : "s"}? This cannot be undone.`;
}

export async function askFormat() {
  let plan: FormatPlan;
  try {
    plan = await api.formatPlan(S().formatLabelDraft || null);
  } catch (e) {
    return toast(errText(e), true);
  }
  store.setState({ agentFormat: null });
  S().setFormatConfirm({ text: confirmText(plan), agent: false });
}

/** The Erase click in the confirmation. */
export async function confirmErase() {
  const st = S();
  if (st.agentFormat != null) {
    const id = st.agentFormat;
    store.setState({ agentFormat: null });
    st.setFormatConfirm(null);
    await api.answerFormatRequest(id, true);
    toast("Erasing the card…");
    return;
  }
  try {
    await api.formatCard(st.formatLabelDraft || sel.formatLabel(st));
    st.setFormatConfirm(null);
    store.setState({ formatUnlocked: false });
  } catch (e) {
    st.setFormatConfirm(null);
    toast(errText(e), true);
  }
}

/** The confirmation closed without Erase. */
export function cancelErase() {
  const st = S();
  if (st.agentFormat != null) {
    api.answerFormatRequest(st.agentFormat, false);
    store.setState({ agentFormat: null });
  }
  st.setFormatConfirm(null);
}
