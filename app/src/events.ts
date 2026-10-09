// Startup and the core's events. Each event refetches what it names; the store re-renders
// only the parts whose data changed.
import { api } from "./ipc/api";
import { on, onDragDrop } from "./ipc/events";
import { confirmText, importDropped, sessionChanged } from "./actions/session";
import { store } from "./store";
import { dialogOpen } from "./store/ui";

const S = () => store.getState();

export async function start(): Promise<() => void> {
  const offs = await Promise.all([
    on("library-changed", () => S().loadLibrary()),
    on("settings-changed", async () => {
      // The Settings sheet reads them fresh when it closes.
      if (S().settingsOpen) return;
      await S().loadSettings();
      await S().loadGear();
      await S().loadLibrary();
    }),
    on("session-changed", async () => sessionChanged(await api.getSession())),
    on("volumes-changed", () => S().refreshVolumes()),
    on("gear-changed", () => S().loadGear()),
    on("device-changed", (p) => {
      S().deviceChanged(p);
      S().loadGear();
    }),
    on("library-task", (t) => {
      S().setTask(t.task, t.done, t.total);
      if (t.done >= t.total) setTimeout(() => S().endTask(t.task), 800);
    }),
    on("progress", (p) => {
      store.setState({ staging: p });
      S().setTask(p.phase, p.index, p.total);
      if (S().importOpen && S().busy && S().step !== "load") S().setStep("load");
    }),
    // An agent asked to erase the card: the person must click Erase here.
    on("agent-format-request", ({ id, plan }) => {
      store.setState({ agentFormat: id });
      S().setFormatConfirm({ text: confirmText(plan), agent: true });
    }),
    on("agent-format-closed", (id) => {
      if (S().agentFormat === id) {
        store.setState({ agentFormat: null });
        S().setFormatConfirm(null);
      }
    }),
    // An agent asked to apply a staged change: the person must click Apply in the sheet.
    on("agent-apply-request", ({ id, change, plan }) => S().agentApply(id, change, plan)),
    on("agent-apply-closed", (id) => S().agentApplyClosed(id)),
    onDragDrop((e) => {
      const s = S();
      const blocked = s.busy || !s.env?.tools || dialogOpen(s);
      s.setDropping((e.type === "enter" || e.type === "over") && !blocked);
      if (e.type === "drop" && !blocked) importDropped(e.paths || []);
    }),
  ]);
  const onFocus = () => S().loadLibrary(true);
  window.addEventListener("focus", onFocus);

  await S().loadSettings();
  await loadEnv();
  const out = (S().values.outputDir as string | undefined) || (await api.defaultOutputDir()) || "";
  store.setState({ home: (out.match(/^\/Users\/[^/]+/) || [""])[0] });
  api.libraryScope().catch(() => {});
  const s = await api.getSession();
  if (s) S().setSession(s);
  await S().loadLibrary();
  await S().refreshVolumes();
  await S().loadGear();
  makeStrips();
  return () => {
    offs.forEach((off) => off());
    window.removeEventListener("focus", onFocus);
  };
}

let stripsRunning = false;
/** Clips that came in while the app ran (an agent, the CLI) need thumbnails too. */
export async function makeStrips() {
  const s = S();
  if (stripsRunning || !s.env?.tools) return;
  if (!s.lib?.clips.some((c) => !c.strip && !c.poster && !c.no_picture)) return;
  stripsRunning = true;
  try {
    await api.strips();
  } catch (e) {
    console.warn(e);
  } finally {
    stripsRunning = false;
  }
}

/** Finds ffmpeg again (after a module install or removal). */
export async function loadEnv() {
  store.setState({ env: await api.envCheck() });
}
