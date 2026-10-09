// The mock core's model editors (`core/model_edit.rs`, `gear/edgetx/editors.rs`), written by
// hand: a made-up radio with two models, the editors' ops applied to a plain state, and
// `gear_model_edit` joining edits into one "Model edits" change as the core does.
import type { CalloutView, CalloutWhen, Edit, ModelDetail, ModelOp, ModelTimer, ScreenDetail, SensorLog, StagedChange } from "../types";
import type { MockGear } from "./gear";
import * as changes from "./changes";

export const MODEL_CHANGE = "Model edits";
const isModelEdit = (e: Edit) => e.kind === "model" || e.kind === "checklist";

interface State {
  name: string;
  timers: ModelTimer[];
  screens: ScreenDetail[];
  sensors: SensorLog[];
  logging: { swtch: string; period_ds: number } | null;
  rf: { warning: number; critical: number };
  callouts: CalloutView[];
  checklistOn: boolean;
  checklist: string | null;
}

const SENSORS = ["1RSS", "RQly", "RxBt", "Capa"];
const TRACKS = ["armed", "disarm", "hello", "lowbat"];

const timer = (index: number, name: string, swtch: string, mode: string): ModelTimer => ({
  index,
  name,
  swtch,
  mode,
  value: 0,
  persistent: "0",
  fields: [
    { key: "swtch", value: `"${swtch}"` },
    { key: "mode", value: mode },
    { key: "countdownBeep", value: "0" },
    { key: "minuteBeep", value: "1" },
    { key: "persistent", value: "0" },
    { key: "showElapsed", value: "0" },
    { key: "name", value: `"${name}"` },
  ],
});

const screen = (index: number, lines: string[][]): ScreenDetail => ({
  index,
  kind: "VALUES",
  script: null,
  lines: lines.map((l) => l.map((s) => (s === "{RxBt}" ? "tele(2)" : s === "{Capa}" ? "tele(3)" : s))),
  labels: lines,
});

function seed(file: string): State {
  return {
    name: file === "model00.yml" ? "ALPHA" : "BRAVO 2",
    timers: [timer(0, "TOT", "L1", "ON"), timer(1, "FLT", "L1", "ON")],
    screens: [screen(0, [["{RxBt}", "Tmr1"]])],
    sensors: SENSORS.map((label) => ({ label, logs: true })),
    logging: null,
    rf: { warning: 45, critical: 42 },
    callouts: [
      { track: "armed", swtch: "L1", repeat: "1x", when: { when: "switch", swtch: "L1" } },
      { track: "disarm", swtch: "!L1", repeat: "!1x", when: { when: "switch", swtch: "!L1" } },
    ],
    checklistOn: false,
    checklist: null,
  };
}

const field = (t: ModelTimer, key: string, value: string) => {
  const f = t.fields.find((x) => x.key === key);
  if (f) f.value = value;
  else t.fields.push({ key, value });
};

const bad = (s: string) => `Refused (bad_setting): ${s}`;

function checkTimer(key: string, value: string) {
  const num = (max: number) => {
    if (!/^\d+$/.test(value) || Number(value) > max) throw bad(`Timer ${key} takes 0-${max}, not "${value}".`);
  };
  if (["minuteBeep", "showElapsed", "extraHaptic"].includes(key)) num(1);
  else if (key === "countdownBeep") num(3);
  else if (key === "persistent") num(2);
  else if (key === "mode" && !/^[A-Z0-9_]+$/.test(value)) throw bad(`Timer mode "${value}" is not a mode name (OFF, ON, START, THR, ...).`);
}

function applyOp(s: State, op: ModelOp) {
  switch (op.op) {
    case "set_timer": {
      if (op.index > 2) throw bad("Timers are 1-3.");
      let t = s.timers.find((x) => x.index === op.index);
      if (!t) {
        t = timer(op.index, "", "NONE", "OFF");
        s.timers.push(t);
        s.timers.sort((a, b) => a.index - b.index);
      }
      for (const f of op.fields) {
        if (f.key === "value") throw bad("A timer's stored value is never set; QuadCam keeps it.");
        checkTimer(f.key, f.value);
        field(t, f.key, ["swtch", "name"].includes(f.key) ? `"${f.value}"` : f.value);
        if (f.key === "name") t.name = f.value;
        if (f.key === "swtch") t.swtch = f.value;
        if (f.key === "mode") t.mode = f.value;
        if (f.key === "persistent") t.persistent = f.value;
      }
      break;
    }
    case "remove_timer":
      s.timers = s.timers.filter((t) => t.index !== op.index);
      break;
    case "set_screen": {
      s.screens = s.screens.filter((x) => x.index !== op.index);
      if (op.script != null) {
        if (op.script.length > 6) throw bad(`Script "${op.script}": a telemetry script name is 1-6 characters.`);
        s.screens.push({ index: op.index, kind: "SCRIPT", script: op.script, lines: [], labels: [] });
      }
      s.screens.sort((a, b) => a.index - b.index);
      break;
    }
    case "set_screen_values": {
      if (op.index > 3) throw bad("Telemetry screens are 1-4.");
      if (op.lines.length > 4) throw bad(`A value screen holds 4 lines; this has ${op.lines.length}.`);
      const labels = op.lines.map((l) => l.map((x) => x.trim()).filter(Boolean));
      if (labels.some((l) => l.length > 3)) throw bad("A value screen line holds 3 sources.");
      if (!labels.some((l) => l.length)) throw bad("A value screen needs at least one source; remove the screen instead.");
      for (const l of labels) for (const x of l) {
        const m = /^\{(.*)\}$/.exec(x);
        if (m && !SENSORS.includes(m[1])) throw bad(`The model has no telemetry sensor labelled "${m[1]}"; discover sensors on the radio first.`);
      }
      if (op.index > 0 && !s.screens.some((x) => x.index === op.index - 1)) throw bad(`Telemetry screen ${op.index} is missing; screen ${op.index + 1} cannot come after it.`);
      s.screens = s.screens.filter((x) => x.index !== op.index);
      s.screens.push({ index: op.index, kind: "VALUES", script: null, labels, lines: labels.map((l) => l.map((x) => (/^\{(.*)\}$/.test(x) ? `tele(${SENSORS.indexOf(x.slice(1, -1))})` : x))) });
      s.screens.sort((a, b) => a.index - b.index);
      break;
    }
    case "set_logging":
      if (op.logging && (op.logging.period_ds < 1 || op.logging.period_ds > 255)) throw bad("A logging period is 0.1-25.5 s.");
      if (op.logging && !op.logging.swtch) throw bad("Logging needs a switch (ON for always).");
      s.logging = op.logging ? { ...op.logging } : null;
      break;
    case "set_sensor_logs":
      for (const x of op.sensors) {
        const t = s.sensors.find((y) => y.label === x.label);
        if (!t) throw bad(`The model has no telemetry sensor labelled "${x.label}"; discover sensors on the radio first.`);
        t.logs = x.logs;
      }
      break;
    case "set_rf_alarms":
      if (op.warning < 1 || op.warning > 127 || op.critical < 1 || op.critical > 127) throw bad("RSSI alarms take 1-127.");
      if (op.critical > op.warning) throw bad(`The critical level (${op.critical}) is over the warning level (${op.warning}); signal falls from warning to critical.`);
      s.rf = { warning: op.warning, critical: op.critical };
      break;
    case "set_callout": {
      const c = op.callout;
      if (!/^[A-Za-z0-9_-]{1,8}$/.test(c.track)) throw bad(`Track "${c.track}": a track name is 1-8 letters, digits, - or _.`);
      const repeat = c.repeat ?? "1x";
      if (!/^(!?1x|\d+)$/.test(repeat)) throw bad(`Repeat "${repeat}": use 1x, !1x (once, not at power-on) or seconds.`);
      let when: CalloutWhen;
      let swtch: string;
      if (c.when === "switch") {
        if (!c.swtch) throw bad("A callout needs a switch.");
        when = { when: "switch", swtch: c.swtch };
        swtch = c.swtch;
      } else {
        const m = /^\{(.*)\}$/.exec(c.source);
        if (m && !SENSORS.includes(m[1])) throw bad(`The model has no telemetry sensor labelled "${m[1]}"; discover sensors on the radio first.`);
        if (Number.isNaN(Number(c.value)) || c.value.trim() === "") throw bad(`"${c.value}" is not a number.`);
        when = { when: c.when, source: c.source, value: String(Number(c.value)), delay_ds: c.delay_ds ?? 0 };
        swtch = "L3";
      }
      const next: CalloutView = { track: c.track, swtch, repeat, when };
      const i = s.callouts.findIndex((x) => x.track === c.track);
      if (i >= 0) s.callouts[i] = next;
      else s.callouts.push(next);
      break;
    }
    case "remove_callout":
      s.callouts = s.callouts.filter((x) => x.track !== op.track);
      break;
    case "set_checklist":
      s.checklistOn = op.enabled;
      break;
    case "rename":
      if (op.name.length > 15) throw bad(`A model name "${op.name}" is over 15 characters.`);
      s.name = op.name;
      break;
    default:
      break;
  }
}

/** What a staged op is about; a later op with the same key replaces the earlier one. */
export function opKey(op: ModelOp): string {
  switch (op.op) {
    case "set_timer":
    case "remove_timer":
      return `timer:${op.index}`;
    case "set_screen":
    case "set_screen_values":
      return `screen:${op.index}`;
    case "set_callout":
      return `callout:${op.callout.track}`;
    case "remove_callout":
      return `callout:${op.track}`;
    case "set_logging":
      return "logging";
    case "set_sensor_logs":
      return "sensor_logs";
    case "set_rf_alarms":
      return "rf_alarms";
    case "set_checklist":
      return "checklist";
    case "rename":
      return "rename";
    default:
      return JSON.stringify(op);
  }
}

export function mergeOps(kept: ModelOp[], fresh: ModelOp[]): ModelOp[] {
  const out = [...kept];
  for (const op of fresh) {
    const i = out.findIndex((o) => opKey(o) === opKey(op));
    let merged: ModelOp = op;
    const old = i >= 0 ? out[i] : null;
    if (old?.op === "set_timer" && op.op === "set_timer") {
      const fields = old.fields.map((f) => ({ ...f }));
      for (const f of op.fields) {
        const x = fields.find((y) => y.key === f.key);
        if (x) x.value = f.value;
        else fields.push({ ...f });
      }
      merged = { op: "set_timer", index: op.index, fields };
    } else if (old?.op === "set_sensor_logs" && op.op === "set_sensor_logs") {
      const sensors = old.sensors.map((x) => ({ ...x }));
      for (const s of op.sensors) {
        const x = sensors.find((y) => y.label === s.label);
        if (x) x.logs = s.logs;
        else sensors.push({ ...s });
      }
      merged = { op: "set_sensor_logs", sensors };
    }
    if (i >= 0) out.splice(i, 1);
    out.push(merged);
  }
  return out;
}

const editsOf = (cs: StagedChange[]) => cs.flatMap((c) => c.edits.filter(isModelEdit));

function applied(g: MockGear, device: string): Edit[] {
  return editsOf(g.changeStore.changes.filter((c) => c.device === device && ["applied", "verified"].includes(c.status)));
}

function build(file: string, edits: Edit[]): { state: State; used: number } {
  const state = seed(file);
  let used = 0;
  for (const e of edits) {
    if (e.kind === "model" && e.file === file) {
      for (const op of e.ops) {
        applyOp(state, op);
        used += 1;
      }
    } else if (e.kind === "checklist" && e.model === file) {
      state.checklist = e.text;
      used += 1;
    }
  }
  return { state, used };
}

const FILES = [
  { file: "model00.yml", name: "ALPHA" },
  { file: "model01.yml", name: "BRAVO 2" },
];

function detail(g: MockGear, device: string, file: string, state: State, staged: number, source: string): ModelDetail {
  return {
    device,
    source,
    models: FILES.map((f) => ({ file: f.file, name: build(f.file, applied(g, device)).state.name, selected: f.file === "model01.yml" })),
    view: {
      file,
      name: state.name,
      model_ids: [],
      timers: structuredClone(state.timers),
      mixes: [],
      logical_switches: [],
      special_functions: [],
      switch_warnings: [],
      legacy_switch_warning: false,
      sensors: SENSORS.map((label, slot) => ({ slot, label })),
      screens: state.screens.map((s) => ({ index: s.index, kind: s.kind, script: s.script ?? null })),
      checklist: state.checklistOn,
      checklist_interactive: state.checklistOn,
    },
    editors: {
      logging: { logging: state.logging ? { ...state.logging } : null, sensors: structuredClone(state.sensors) },
      rf_alarms: { ...state.rf },
      callouts: structuredClone(state.callouts),
      screens: structuredClone(state.screens),
    },
    checklist: state.checklist,
    checklist_width: 20,
    tracks: TRACKS,
    staged,
    notes: staged ? [`Shows ${staged} staged ${staged === 1 ? "edit" : "edits"}: not on the radio until applied.`] : [],
  };
}

/** `gear_model`. */
export function view(g: MockGear, p: { device: string; model?: string | null; staged?: boolean }): ModelDetail {
  const d = g.devices.find((x) => x.id === p.device);
  if (!d) throw `No device "${p.device}" in QuadCam's list (quadcam-cli gear devices).`;
  if (d.kind !== "radio") throw `"${p.device}" is not a radio.`;
  const mounted = g.connected.some((c) => c.id === p.device);
  if (!mounted && !d.last_backup) throw `No backup of "${p.device}" yet: back it up, or plug the radio in (USB Storage).`;
  const file = (p.model ?? "model01.yml").replace(/^MODELS\//, "");
  if (!/^model\d+\.yml$/i.test(file)) throw `"${file}" is not a model file name (model01.yml).`;
  if (!FILES.some((f) => f.file === file)) throw `The ${mounted ? "card" : "backup"} has no MODELS/${file}.`;
  const staged = p.staged !== false ? editsOf(changes.list(g, p.device, false)) : [];
  const { state, used } = build(file, [...applied(g, p.device), ...staged]);
  const base = build(file, applied(g, p.device));
  return detail(g, p.device, file, state, used - base.used, mounted ? "card" : `backup ${d.last_backup} (2026-10-06 18:20)`);
}

const state = (e: Edit[], file: string) => JSON.stringify(build(file, e).state);

/** `gear_model_edit`: one open "Model edits" change per device. */
export function edit(g: MockGear, p: { device: string; model: string; ops?: ModelOp[]; checklist?: string | null }, editor: "user" | "agent"): StagedChange {
  const ops = p.ops ?? [];
  if (!ops.length && p.checklist == null) throw "Pass at least one model op, or a checklist.";
  const v = view(g, { device: p.device, model: p.model, staged: false });
  const file = v.view.file;
  const base = applied(g, p.device);
  const open = changes.list(g, p.device, false).find((c) => c.title === MODEL_CHANGE && ["draft", "ready"].includes(c.status) && c.edits.every(isModelEdit));
  const earlier = (open?.edits ?? []).filter((e) => e.kind === "model" && e.file === file).flatMap((e) => (e.kind === "model" ? e.ops : []));
  const earlierText = (open?.edits ?? []).find((e) => e.kind === "checklist" && e.model === file);
  let fresh = [...ops];
  if (p.checklist != null) {
    const w = 20;
    const lines = p.checklist.split("\n");
    if (lines.length > 99) throw bad(`A checklist holds 99 lines at most; this has ${lines.length}.`);
    const long = lines.find((l) => l.trimEnd().length > w);
    if (long) throw bad(`Checklist line "${long}" is over ${w} characters, the screen's width.`);
    if (p.checklist.trim() && !fresh.some((o) => o.op === "set_checklist")) fresh = [...fresh, { op: "set_checklist", enabled: true }];
  }
  const merged = mergeOps(earlier, fresh);
  const text = p.checklist ?? (earlierText?.kind === "checklist" ? earlierText.text : null);
  const mk = (e: Edit[]) => state(e, file);
  const candidate: Edit[] = [{ kind: "model", file, name: v.view.name, ops: merged }, ...(text != null ? [{ kind: "checklist" as const, model: file, text }] : [])];
  // Throws the op's own refusal when it does not apply.
  build(file, [...base, ...candidate]);
  const normal = (t: string | null | undefined) => (t ?? "").split("\n").map((l) => l.trimEnd()).join("\n").trimEnd();
  const modelChanges = mk([...base, candidate[0]]) !== mk(base);
  const textChanges = text != null && normal(text) !== normal(build(file, base).state.checklist);
  const edits: Edit[] = [
    ...(open?.edits ?? []).filter((e) => !(e.kind === "model" && e.file === file) && !(e.kind === "checklist" && e.model === file)),
    ...(modelChanges ? [candidate[0]] : []),
    ...(textChanges && text != null ? [{ kind: "checklist" as const, model: file, text }] : []),
  ];
  if (!edits.length) {
    if (open) return changes.discard(g, open.id);
    throw "Nothing changes: the radio already has that.";
  }
  if (open) return changes.update(g, { id: open.id, edits });
  return changes.stage(g, p.device, edits, MODEL_CHANGE, editor);
}
