// Backups as the UI shows them: why each was taken, its time, a device's running job and
// latest card check.
import type { BackupSummary, CardCheck, GearJob, GearStatus, StepFailure } from "../ipc/types";

/** Why a backup was taken (`Trigger`), in words. */
export const TRIGGER_LABEL: Record<BackupSummary["trigger"], string> = {
  connect: "Plug-in",
  manual: "Manual",
  before_apply: "Before apply",
  before_flash: "Before flash",
  import: "Imported",
};

/** The time in a backup id (`<device>/<YYYY-MM-DDTHHMMSS>-<trigger>`, UTC) as ISO, or null. */
export function backupTime(id: string | null | undefined): string | null {
  const m = /\/(\d{4}-\d{2}-\d{2})T(\d{2})(\d{2})(\d{2})-/.exec(id || "");
  return m ? `${m[1]}T${m[2]}:${m[3]}:${m[4]}Z` : null;
}

/** A time as the app shows one. */
export const fmtWhen = (t: string | null | undefined) => (t ? new Date(t).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "");

/** The backup or card check running on a device now. */
export const jobFor = (s: GearStatus | null, device: string | null | undefined): GearJob | null => (device && s?.jobs?.find((j) => j.device === device)) || null;

/** A card's latest check, when it is plugged in. */
export const checkFor = (s: GearStatus | null, device: string | null | undefined): CardCheck | null => (device && s?.card_checks?.find((c) => c.device === device)) || null;

/** A card's latest check in a line: "Card check: passed, Oct 7, 9:00 AM". */
export function checkLine(c: CardCheck): string {
  const what = { ok: "passed", failed: "found problems", stopped: "stopped", error: "did not run" }[c.state];
  return `Card check: ${what}, ${fmtWhen(c.at)}`;
}

/** True when a card's latest check found a problem. */
export const checkFailed = (c: CardCheck | null) => !!c && (c.state === "failed" || c.state === "error");

/** A job's progress in words: "12 of 40 files". */
export function jobText(j: GearJob): string {
  const p = j.progress;
  if (j.stopping) return "Stopping";
  if (p.stage === "logs") return "Logs";
  if (!p.files_total) return j.step;
  return `${p.files_done} of ${p.files_total} files`;
}

/** The last step that failed on a link (an unmount, a backup), with its reason. */
export const failureFor = (s: GearStatus | null, handle: string | null | undefined): StepFailure | null => (handle && s?.failures?.find((f) => f.handle === handle)) || null;
