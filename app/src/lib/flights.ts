// Formatting for flights, packs and the "Pack up" check.
import type { ChargeState, Chemistry, RowState } from "../ipc/types";

/** `m:ss`, or a dash for no value. */
export function mmss(s: number | null | undefined): string {
  if (s == null || !Number.isFinite(s)) return "–";
  const t = Math.max(0, Math.round(s));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, "0")}`;
}

/** A number with `digits` decimals and a unit, or a dash. */
export const num = (v: number | null | undefined, digits: number, unit = "") => (v == null || !Number.isFinite(v) ? "–" : `${v.toFixed(digits)}${unit ? ` ${unit}` : ""}`);

/** `HH:MM` of a `YYYY-MM-DDTHH:MM:SS` local time. */
export const hhmm = (t: string) => t.slice(11, 16);

export const CHARGE_LABEL: Record<ChargeState, string> = { charged: "Charged", flown: "Flown", unknown: "Not marked" };
export const CHEMISTRY_LABEL: Record<Chemistry, string> = { lipo: "LiPo", lihv: "LiHV", liion: "Li-ion" };
export const ROW_LABEL: Record<RowState, string> = { pass: "Pass", warn: "Check", unknown: "Unknown" };

/** Copies text to the clipboard; false when the web view refused. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const t = document.createElement("textarea");
    t.value = text;
    t.style.position = "fixed";
    t.style.opacity = "0";
    document.body.appendChild(t);
    t.select();
    const ok = document.execCommand("copy");
    t.remove();
    return ok;
  }
}
