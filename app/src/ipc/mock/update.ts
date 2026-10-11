// The app's own update in the mock core: a release feed the spec sets (`feed`), the download
// in a few progress steps, and the restart rule (refused while `job` names a running job).
import type { UpdateStatus } from "../types";
import * as seed from "./seed";

export interface MockUpdate {
  status: UpdateStatus;
  /** The newest published version, or null for none newer. "fail" makes the check fail. */
  feed: string | null | "fail";
  /** The job that holds the restart back, as the core words it; null for none. */
  job: string | null;
  /** Restarts the app was asked for. */
  restarts: number;
  /** A download fails once. */
  failDownload: boolean;
}

export const freshUpdate = (): MockUpdate => ({
  status: { phase: "idle", current: seed.VERSION.version, version: null, notes_url: null, done: 0, total: null, error: null, checked_at: null, waiting_for: null },
  feed: null,
  job: null,
  restarts: 0,
  failDownload: false,
});

const now = () => new Date().toISOString();

export function status(u: MockUpdate): UpdateStatus {
  return { ...structuredClone(u.status), waiting_for: u.job };
}

export function check(u: MockUpdate): UpdateStatus {
  if (u.status.phase === "downloading" || u.status.phase === "ready") return status(u);
  if (u.feed === "fail") {
    u.status = { ...u.status, phase: "failed", error: "Could not check for updates: the feed did not answer", checked_at: now() };
  } else if (u.feed) {
    u.status = { ...u.status, phase: "available", version: u.feed, notes_url: `https://github.com/noahkiss/quadcam/releases/tag/v${u.feed}`, done: 0, total: null, error: null, checked_at: now() };
  } else {
    u.status = { ...u.status, phase: "current", version: null, notes_url: null, error: null, checked_at: now() };
  }
  return status(u);
}

/** Downloads in steps, emitting each one as the core does. */
export async function download(u: MockUpdate, emit: (s: UpdateStatus) => void): Promise<UpdateStatus> {
  if (!u.status.version) throw "No update to download. Check for updates first.";
  const total = 4_000_000;
  for (const done of [0, 1_000_000, 2_500_000]) {
    u.status = { ...u.status, phase: "downloading", done, total, error: null };
    emit(status(u));
    await new Promise((r) => setTimeout(r, 150));
  }
  if (u.failDownload) {
    u.failDownload = false;
    u.status = { ...u.status, phase: "failed", error: "The update did not download: connection reset" };
    return status(u);
  }
  u.status = { ...u.status, phase: "ready", done: total, total };
  return status(u);
}

export function restart(u: MockUpdate) {
  if (u.job) throw `QuadCam restarts after ${u.job} ends.`;
  if (u.status.phase !== "ready") throw "Download the update first.";
  u.restarts += 1;
  return null;
}
