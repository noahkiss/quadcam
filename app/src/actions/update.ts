// The app's own update: check (the app menu, Settings), download, restart. The core holds the
// state and refuses the restart while a job runs; errors show as a toast, never a dialog.
import { api, errText } from "../ipc/api";
import { store } from "../store";
import { toast } from "../components/toastStore";

const S = () => store.getState();

/** Check for Updates: a toast says when there is nothing newer or the check failed. */
export async function checkForUpdates() {
  S().clearLater();
  try {
    const u = await api.updateCheck();
    S().setUpdate(u);
    if (u.phase === "current") toast(`QuadCam ${u.current} is the newest version.`);
    if (u.phase === "failed") toast(u.error || "Could not check for updates.", true);
  } catch (e) {
    toast(errText(e), true);
  }
}

/** Restart to update: the core checks once more and downloads the newest release (a download
 * of that version already done is kept), then restarts unless a job runs. */
export async function restartToUpdate() {
  try {
    const u = await api.updateDownload();
    S().setUpdate(u);
    if (u.phase !== "ready" || u.waiting_for) return;
    await api.updateRestart();
  } catch (e) {
    toast(errText(e), true);
  }
}
