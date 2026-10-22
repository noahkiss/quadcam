// The mock core's radio Settings editor (`core/radio_settings.rs`, `gear/edgetx/radio.rs`),
// written by hand: the settings table, the made-up radio's `radio.yml` (`changes.RADIO_YML`)
// with the staged radio edits on top, and `gear_radio_settings_edit` joining edits into one
// "Radio settings" change as the core does.
import type { RadioOp, RadioSetting, RadioSettingsDetail, SettingKind, StagedChange } from "../types";
import type { MockGear } from "./gear";
import * as changes from "./changes";
import { FILES } from "./model";

export const RADIO_CHANGE = "Radio settings";

const SECTIONS = ["Sound", "Haptic", "Display", "Alarms", "Time and GPS", "Sticks and USB", "Owner"];

const choice = (list: [string, string][]): SettingKind => ({ type: "choice", choices: list.map(([value, label]) => ({ value, label })) });
const five = choice(["-2", "-1", "0", "+1", "+2"].map((l, i) => [String(i), l]));
const num = (min: number, max: number, unit = "", mul = 1, decimals = 0, invert: number | null = null): SettingKind => ({ type: "number", min, max, unit, mul, decimals, invert });
const MODES = choice([
  ["mode_quiet", "Quiet"],
  ["mode_alarms", "Alarms only"],
  ["mode_nokeys", "No keys"],
  ["mode_all", "All"],
]);

/** The 24 channel orders, in `templateSetup` order (21 is AETR). */
export function channelOrders(): string[] {
  const out: string[] = [];
  const walk = (left: string[], acc: string) => {
    if (!left.length) out.push(acc);
    left.forEach((c, i) => walk([...left.slice(0, i), ...left.slice(i + 1)], acc + c));
  };
  walk(["R", "E", "T", "A"], "");
  return out;
}

const SETTINGS: [string, string, string, SettingKind][] = [
  ["beepMode", "Sound", "Sound mode", MODES],
  ["speakerVolume", "Sound", "Volume", num(0, 23)],
  ["beepVolume", "Sound", "Beep volume", five],
  ["beepLength", "Sound", "Beep length", five],
  ["wavVolume", "Sound", "Sound file volume", five],
  ["backgroundVolume", "Sound", "Background volume", five],
  ["varioVolume", "Sound", "Vario volume", five],
  ["hapticMode", "Haptic", "Haptic mode", MODES],
  ["hapticLength", "Haptic", "Haptic length", five],
  ["hapticStrength", "Haptic", "Haptic strength", five],
  ["contrast", "Display", "Contrast", num(10, 30)],
  [
    "backlightMode",
    "Display",
    "Backlight",
    choice([
      ["backlight_mode_off", "Off"],
      ["backlight_mode_keys", "Keys"],
      ["backlight_mode_sticks", "Sticks"],
      ["backlight_mode_all", "Keys and sticks"],
      ["backlight_mode_on", "On"],
    ]),
  ],
  ["lightAutoOff", "Display", "Backlight off after", num(1, 120, "s", 5)],
  ["backlightBright", "Display", "Brightness", num(0, 95, "", 1, 0, 100)],
  ["vBatWarn", "Alarms", "Battery low", num(30, 120, "V", 1, 1)],
  ["inactivityTimer", "Alarms", "Inactivity alarm", num(0, 250, "min")],
  ["disableAlarmWarning", "Alarms", "Sound off warning", { type: "toggle", invert: true }],
  ["disableRssiPoweroffAlarm", "Alarms", "RSSI shutdown alarm", { type: "toggle", invert: true }],
  ["timezone", "Time and GPS", "Time zone", num(-12, 14, "h")],
  ["adjustRTC", "Time and GPS", "Set the clock from GPS", { type: "toggle", invert: false }],
  [
    "gpsFormat",
    "Time and GPS",
    "GPS coordinates",
    choice([
      ["0", "DMS"],
      ["1", "NMEA"],
    ]),
  ],
  [
    "imperial",
    "Time and GPS",
    "Units",
    choice([
      ["0", "Metric"],
      ["1", "Imperial"],
    ]),
  ],
  [
    "stickMode",
    "Sticks and USB",
    "Stick mode",
    choice([
      ["0", "Mode 1"],
      ["1", "Mode 2"],
      ["2", "Mode 3"],
      ["3", "Mode 4"],
    ]),
  ],
  ["templateSetup", "Sticks and USB", "Default channel order", choice(channelOrders().map((o, i) => [String(i), o]))],
  [
    "USBMode",
    "Sticks and USB",
    "USB mode",
    choice([
      ["0", "Ask"],
      ["1", "Joystick"],
      ["2", "Storage"],
      ["3", "Serial"],
    ]),
  ],
  ["ownerRegistrationID", "Owner", "Owner ID", { type: "text", max_len: 8 }],
];

/** Choices the core checks as a range (the 5-step settings, the channel order). */
const INT_CHOICES = new Set(["beepVolume", "beepLength", "wavVolume", "backgroundVolume", "varioVolume", "hapticLength", "hapticStrength", "templateSetup"]);
const PROTECTED = ["currModel", "currModelFilename", "semver", "board", "checksum", "manuallyEdited"];
const bad = (reason: string) => ({ code: "bad_setting", reason });
const unquote = (v: string) => v.replace(/^"(.*)"$/, "$1");
const fileOf = (n: string) => `model${n.padStart(2, "0")}.yml`;

/** The line value a set_scalar writes, or the core's refusal. */
function lineValue(key: string, value: string): string {
  if (PROTECTED.includes(key)) throw bad(`${key} is not set this way${key.startsWith("currModel") ? "; use select_model" : ""}.`);
  const def = SETTINGS.find((s) => s[0] === key);
  if (!(key in changes.RADIO_YML)) throw { code: "shape_unknown", reason: `radio.yml has no ${key} line; QuadCam does not add it; nothing was written.` };
  if (!def) return value;
  const [, , label, k] = def;
  const v = value.trim();
  const int = (min: number, max: number) => {
    if (!/^-?\d+$/.test(v) || +v < min || +v > max) throw bad(`${key} (${label}) is a whole number from ${min} to ${max}, not "${v}".`);
    return String(+v);
  };
  switch (k.type) {
    case "choice":
      if (!k.choices.some((c) => c.value === v)) {
        if (INT_CHOICES.has(key)) return int(0, k.choices.length - 1);
        throw bad(`${key} (${label}) is one of ${k.choices.map((c) => c.value).join(", ")}, not "${v}".`);
      }
      return v;
    case "number":
      return int(k.min, k.max);
    case "toggle":
      return int(0, 1);
    case "text": {
      const t = unquote(value);
      if ([...t].length > k.max_len) throw bad(`${key} (${label}) holds ${k.max_len} characters at most.`);
      if (!/^[A-Za-z0-9 ._-]*$/.test(t)) throw bad(`${key} (${label}) holds letters, digits, spaces, -, _ and . only.`);
      return `"${t}"`;
    }
  }
}

/** `yml` with one op applied; throws the core's refusal. */
function applyOp(yml: Record<string, string>, op: RadioOp): Record<string, string> {
  if (op.op === "select_model") {
    const f = op.file.replace(/^MODELS\//, "");
    if (!FILES.some((x) => x.file === f)) throw bad(`The card has no MODELS/${f}.`);
    return { ...yml, currModel: String(Number(f.replace(/\D/g, ""))) };
  }
  // Switch and pot types and radio functions are the radio setup's; the Settings editor's mock leaves them.
  if (op.op !== "set_scalar") return yml;
  return { ...yml, [op.key]: lineValue(op.key, op.value) };
}

const isRadioOnly = (c: StagedChange) => c.edits.every((e) => e.kind === "radio");
const opsOf = (cs: StagedChange[]) => cs.flatMap((c) => c.edits.flatMap((e) => (e.kind === "radio" ? e.ops : [])));

function mounted(g: MockGear, device: string) {
  const d = g.devices.find((x) => x.id === device);
  if (!d) throw `No device "${device}" in QuadCam's list (quadcam-cli gear devices).`;
  if (d.kind !== "radio") throw `"${device}" is not a radio.`;
  const card = g.connected.some((c) => c.id === device);
  if (!card && !d.last_backup) throw `No backup of "${device}" yet: back it up, or plug the radio in (USB Storage).`;
  return { d, card };
}

/** `gear_radio_settings`. */
export function view(g: MockGear, p: { device: string; staged?: boolean }): RadioSettingsDetail {
  const { d, card } = mounted(g, p.device);
  let yml: Record<string, string> = { ...changes.RADIO_YML };
  let staged = 0;
  const notes: string[] = [];
  if (p.staged !== false) {
    for (const op of opsOf(changes.list(g, p.device, false).filter((c) => ["draft", "ready"].includes(c.status)))) {
      try {
        yml = applyOp(yml, op);
        staged += 1;
      } catch (e) {
        notes.push(`A staged edit does not apply: ${(e as { reason: string }).reason}`);
      }
    }
    if (staged) notes.push(`Shows ${staged} staged ${staged === 1 ? "edit" : "edits"}: not on the radio until applied.`);
  }
  const settings: RadioSetting[] = SETTINGS.map(([key, section, label, kind]) => {
    const raw = yml[key];
    const value = raw == null ? null : unquote(raw);
    const k = structuredClone(kind);
    if (k.type === "choice" && value != null && !k.choices.some((c) => c.value === value)) k.choices.push({ value, label: value });
    return { key, section, label, kind: k, value };
  });
  const selected = yml.currModel != null ? fileOf(yml.currModel) : null;
  return {
    device: p.device,
    source: card ? "card" : `backup ${d.last_backup} (2026-10-06 18:20)`,
    board: d.identity?.board ?? null,
    version: d.identity?.version ?? null,
    view: { sections: SECTIONS, settings, selected_model: selected },
    models: FILES.map((f) => ({ file: f.file, name: f.name, selected: f.file === selected })),
    staged,
    notes,
  };
}

/** `gear_radio_settings_edit`: one open "Radio settings" change per device. */
export function edit(g: MockGear, p: { device: string; ops?: RadioOp[] }, editor: "user" | "agent"): StagedChange {
  const ops = p.ops ?? [];
  if (!ops.length) throw "Pass at least one radio op.";
  mounted(g, p.device);
  const open = changes.list(g, p.device, false).find((c) => c.title === RADIO_CHANGE && ["draft", "ready"].includes(c.status) && isRadioOnly(c));
  const key = (o: RadioOp) => (o.op === "set_scalar" ? o.key : "currModel");
  let merged = open ? opsOf([open]) : [];
  for (const op of ops) merged = [...merged.filter((o) => key(o) !== key(op)), op];
  const base = changes.RADIO_YML;
  const fail = (e: unknown) => {
    throw typeof e === "string" ? e : (e as { reason: string }).reason;
  };
  try {
    merged.reduce(applyOp, { ...base });
  } catch (e) {
    fail(e);
  }
  const kept = merged.filter((o) => {
    const one = applyOp({ ...base }, o);
    return JSON.stringify(one) !== JSON.stringify(base);
  });
  if (!kept.length) {
    if (open) return changes.discard(g, open.id);
    throw "Nothing changes: the radio already has that.";
  }
  const edits = [{ kind: "radio" as const, ops: kept }];
  if (open) return changes.update(g, { id: open.id, edits });
  return changes.stage(g, p.device, edits, RADIO_CHANGE, editor);
}
