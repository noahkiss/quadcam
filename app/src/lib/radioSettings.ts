// The radio's Settings editor, pure: a setting's value as the radio shows it, the file value
// a typed number becomes (with the range EdgeTX accepts), and which settings the open
// "Radio settings" change stages.
import type { RadioOp, RadioSetting, SettingKind, StagedChange } from "../ipc/types";

/** The title of the change the Settings editor keeps its edits in (`core/radio_settings.rs`). */
export const RADIO_CHANGE = "Radio settings";

type NumberKind = Extract<SettingKind, { type: "number" }>;

/** A file value of a number setting as the radio shows it (no unit). */
export function numberShown(k: NumberKind, raw: string): string {
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n)) return raw;
  const v = k.invert != null ? k.invert - n : n;
  return ((v * k.mul) / 10 ** k.decimals).toFixed(k.decimals);
}

/** The shown range of a number setting, low to high. */
export function numberRange(k: NumberKind): [string, string] {
  const a = numberShown(k, String(k.min));
  const b = numberShown(k, String(k.max));
  return Number(a) <= Number(b) ? [a, b] : [b, a];
}

/** The step a number setting moves by, as shown. */
export const numberStep = (k: NumberKind): string => ((k.mul / 10 ** k.decimals) as number).toFixed(k.decimals);

/** The file value a typed number becomes, or why it does not fit. */
export function numberRaw(k: NumberKind, shown: string): { raw: string } | { error: string } {
  const [lo, hi] = numberRange(k);
  const range = `${lo} to ${hi}${k.unit ? ` ${k.unit}` : ""}`;
  const v = Number(shown.trim().replace(",", "."));
  if (!shown.trim() || !Number.isFinite(v)) return { error: `Type a number from ${range}.` };
  const scaled = (v * 10 ** k.decimals) / k.mul;
  const n = Math.round(scaled);
  if (Math.abs(scaled - n) > 1e-6) return { error: `Use steps of ${numberStep(k)}${k.unit ? ` ${k.unit}` : ""}.` };
  const raw = k.invert != null ? k.invert - n : n;
  if (raw < k.min || raw > k.max) return { error: `Use a value from ${range}.` };
  return { raw: String(raw) };
}

/** A setting's value as the radio shows it, with its unit. */
export function shownValue(s: Pick<RadioSetting, "kind">, raw: string | null | undefined): string {
  if (raw == null) return "Not on this card";
  const k = s.kind;
  switch (k.type) {
    case "choice":
      return k.choices.find((c) => c.value === raw)?.label ?? raw;
    case "number": {
      const v = numberShown(k, raw);
      return k.unit ? `${v} ${k.unit}` : v;
    }
    case "toggle":
      return (raw.trim() === "1") !== k.invert ? "On" : "Off";
    case "text":
      return raw.trim() ? raw : "None";
  }
}

/** Whether a toggle's file value means on. */
export const toggleOn = (k: Extract<SettingKind, { type: "toggle" }>, raw: string | null | undefined) => (raw?.trim() === "1") !== k.invert;

/** The file value of a toggle that is on or off. */
export const toggleRaw = (k: Extract<SettingKind, { type: "toggle" }>, on: boolean) => (on !== k.invert ? "1" : "0");

/** Why a typed text does not fit, or null. */
export function textError(maxLen: number, t: string): string | null {
  if ([...t].length > maxLen) return `Use ${maxLen} characters at most.`;
  if (!/^[A-Za-z0-9 ._-]*$/.test(t)) return "Use letters, digits, spaces, - _ and . only.";
  return null;
}

/** The radio's open "Radio settings" change. */
export const openRadioChange = (changes: StagedChange[], device: string | null) =>
  changes.find((c) => c.device === device && c.title === RADIO_CHANGE && (c.status === "draft" || c.status === "ready")) ?? null;

/** The ops of a change's radio edits. */
export const radioOps = (c: StagedChange | null): RadioOp[] => (c?.edits ?? []).flatMap((e) => (e.kind === "radio" ? e.ops : []));

/** The keys a change stages (`currModel` for the selected model). */
export const stagedKeys = (c: StagedChange | null): Set<string> => new Set(radioOps(c).map((o) => (o.op === "set_scalar" ? o.key : "currModel")));
