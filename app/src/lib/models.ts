// Pure helpers for the radio model editors (`views/Gear/Models/`).
import type { CalloutView, CalloutWhen, ModelTimer } from "../ipc/types";

/** A timer's raw field, unquoted. */
export function timerField(t: ModelTimer, key: string): string {
  const v = t.fields.find((f) => f.key === key)?.value ?? "";
  return v.replace(/^"(.*)"$/, "$1");
}

/** The timer modes the editor offers; a file's own mode joins the list when it is another. */
export const TIMER_MODES = ["OFF", "ON", "START", "THR", "THR_REL", "THR_TRG"];

export const COUNTDOWN_BEEPS: [string, string][] = [
  ["0", "Silent"],
  ["1", "Beeps"],
  ["2", "Voice"],
  ["3", "Haptic"],
];

export const PERSISTENT: [string, string][] = [
  ["0", "Off"],
  ["1", "Per flight"],
  ["2", "Until reset"],
];

/** The first timer slot (0-2) that no timer holds, or null. */
export function freeTimer(timers: ModelTimer[]): number | null {
  for (let i = 0; i < 3; i++) if (!timers.some((t) => t.index === i)) return i;
  return null;
}

/** `{RxBt}, Tmr1` -> ["{RxBt}", "Tmr1"]. */
export function parseSources(text: string): string[] {
  return text
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

/** The first screen index (0-3) that follows the screens in order, or null. */
export function nextScreen(indexes: number[]): number | null {
  const n = indexes.length ? Math.max(...indexes) + 1 : 0;
  return n <= 3 ? n : null;
}

/** A callout's condition in words. */
export function whenText(c: CalloutView): string {
  const w = c.when;
  if (!w) return `on ${c.swtch} (edit in EdgeTX)`;
  const secs = (d?: number) => (d ? ` for ${d / 10} s` : "");
  if (w.when === "switch") return `while ${w.swtch} is on`;
  const src = w.source.replace(/^\{(.*)\}$/, "$1");
  return `${src} ${w.when === "below" ? "below" : "above"} ${w.value}${secs(w.delay_ds)}`;
}

/** `1x` -> "once"; `!1x` -> "once, not at power-on"; seconds -> "every N s". */
export function repeatText(r: string): string {
  if (r === "1x") return "once";
  if (r === "!1x") return "once, not at power-on";
  return /^\d+$/.test(r) ? `every ${r} s` : r;
}

/** The form's state for a callout (or a fresh one). */
export interface CalloutForm {
  track: string;
  kind: CalloutWhen["when"];
  swtch: string;
  source: string;
  value: string;
  delay: string;
  repeat: "1x" | "!1x" | "every";
  every: string;
}

export function formOf(c: CalloutView | null, sensors: string[]): CalloutForm {
  const w = c?.when;
  const r = c?.repeat ?? "1x";
  return {
    track: c?.track ?? "",
    kind: w?.when ?? "below",
    swtch: w?.when === "switch" ? w.swtch : "ON",
    source: w && w.when !== "switch" ? w.source : sensors.includes("RxBt") ? "{RxBt}" : sensors[0] ? `{${sensors[0]}}` : "",
    value: w && w.when !== "switch" ? w.value : "",
    delay: w && w.when !== "switch" ? String((w.delay_ds ?? 0) / 10) : "2",
    repeat: r === "1x" || r === "!1x" ? r : "every",
    every: /^\d+$/.test(r) ? r : "5",
  };
}

/** The op a callout form stages, or the reason it cannot. */
export function calloutOp(f: CalloutForm): { op: import("../ipc/types").ModelOp } | { error: string } {
  if (!f.track.trim()) return { error: "Pick a sound for the callout." };
  const repeat = f.repeat === "every" ? f.every.trim() : f.repeat;
  if (f.repeat === "every" && !/^\d+$/.test(repeat)) return { error: "Seconds between repeats is a whole number." };
  if (f.kind === "switch") {
    if (!f.swtch.trim()) return { error: "Name the switch." };
    return { op: { op: "set_callout", callout: { track: f.track.trim(), when: "switch", swtch: f.swtch.trim(), repeat } } };
  }
  if (!f.source.trim()) return { error: "Pick the sensor." };
  if (f.value.trim() === "" || Number.isNaN(Number(f.value))) return { error: "The value is a number." };
  const delay = Number(f.delay || "0");
  if (Number.isNaN(delay) || delay < 0) return { error: "The delay is a number of seconds." };
  return { op: { op: "set_callout", callout: { track: f.track.trim(), when: f.kind, source: f.source.trim(), value: f.value.trim(), delay_ds: Math.round(delay * 10), repeat } } };
}

/** Checklist text <-> rows. A line that starts with `=` has a tick box. */
export interface CheckItem {
  tick: boolean;
  text: string;
}

export function parseChecklist(text: string | null | undefined): CheckItem[] {
  return (text ?? "")
    .split(/\r?\n/)
    .filter((l, i, all) => l.trim() !== "" || i < all.length - 1)
    .filter((l) => l.trim() !== "")
    .map((l) => (l.startsWith("=") ? { tick: true, text: l.slice(1) } : { tick: false, text: l }));
}

export function checklistText(items: CheckItem[]): string {
  return items.map((i) => `${i.tick ? "=" : ""}${i.text}`).join("\n") + (items.length ? "\n" : "");
}

/** The longest line against the width, if any is over it. */
export function overWidth(items: CheckItem[], width: number | null | undefined): number {
  if (!width) return 0;
  return items.filter((i) => (i.tick ? 1 : 0) + i.text.length > width).length;
}
