// The mock core's radio setup (`core/radio_setup.rs`, `gear/edgetx/setup/`), written by hand:
// the jobs of design 5.1, the controls of the two boards QuadCam knows (`edgetx/board.rs`),
// fitted parts, the check with each problem at its job, the job rows, the slots, and the
// edit that saves a setup, copies models from another radio and stages one "Radio setup"
// change. The generated card edits are a stand-in: one mix per job, and the slot types.
import type { Assignment, BoardControlKind, Edit, FittedSlot, JobRow, RadioControlKind, RadioSetup, RadioSetupDetail, RadioSetupEdited, SetupControl, SetupCopy, SetupProblem, SlotRow, StagedChange } from "../types";
import type { MockGear } from "./gear";
import * as changes from "./changes";
import { FILES } from "./model";

export const SETUP_CHANGE = "Radio setup";

/** Saved setups by device id. */
export type MockSetups = Record<string, RadioSetup>;

type Slot = [name: string, kind: BoardControlKind, type: string];

const BOARDS: Record<string, Slot[]> = {
  pocket: [
    ["SA", "switch", "2POS"],
    ["SB", "switch", "3POS"],
    ["SC", "switch", "3POS"],
    ["SD", "switch", "2POS"],
    ["SE", "switch", "TOGGLE"],
    ["P1", "pot", "without_detent"],
  ],
  gx12: [
    ["SA", "switch", "2POS"],
    ["SB", "switch", "3POS"],
    ["SC", "switch", "3POS"],
    ["SD", "switch", "2POS"],
    ["SE", "switch", "3POS"],
    ["SF", "switch", "3POS"],
    ["SG", "switch", "TOGGLE"],
    ["SH", "switch", "TOGGLE"],
    ...["SW1", "SW2", "SW3", "SW4", "SW5", "SW6"].map((n): Slot => [n, "custom", "2POS"]),
    ["P1", "pot", "with_detent"],
    ["P2", "pot", "with_detent"],
    ["SL1", "slider", "slider"],
    ["SL2", "slider", "slider"],
  ],
};

const TRIMS = ["TrimRudLeft", "TrimRudRight", "TrimEleDown", "TrimEleUp", "TrimThrDown", "TrimThrUp", "TrimAilLeft", "TrimAilRight"];

const TYPES: Record<BoardControlKind, string[]> = {
  switch: ["2POS", "3POS", "TOGGLE", "NONE"],
  pot: ["with_detent", "without_detent", "none"],
  slider: ["slider", "none"],
  custom: [],
};

const KIND_WORDS: Record<RadioControlKind, string> = {
  two_pos: "2-position switch",
  three_pos: "3-position switch",
  momentary: "momentary switch",
  custom: "customizable switch",
  pot: "pot",
  slider: "slider",
  trim: "trim",
};

const job = (id: string, label: string, channel: number, fits: RadioControlKind[], states: string[]) => ({ id, label, channel, fits, states });

function jobs() {
  return [
    job("arm", "Arm", 4, ["two_pos"], ["disarmed", "armed"]),
    job("osd", "OSD layout", 5, ["three_pos"], ["OSD profile 1", "OSD profile 2", "OSD profile 3"]),
    job("rates", "Rates", 6, ["three_pos"], ["rate profile 1", "rate profile 2", "rate profile 3"]),
    job("finder", "Finder", 7, ["two_pos", "custom"], ["off", "beeper on"]),
    job("turtle", "Turtle", 8, ["momentary"], ["off", "turtle while held"]),
    job("blackbox", "Blackbox erase", 9, ["trim", "momentary"], ["off", "erase while pressed"]),
    job("vtx", "VTX power", 10, ["pot", "slider", "three_pos"], ["25 mW", "200 mW", "400 mW"]),
    ...[1, 2, 3, 4].map((i) => job(`sim${i}`, `Sim button ${i}`, 10 + i, ["trim", "momentary"], ["released", "pressed"])),
  ];
}

const a = (j: string, control: string, high: number | null = null): Assignment => ({ job: j, control, high });

/** `defaults::setup`. */
export function defaultSetup(board: string, model = "model01.yml"): RadioSetup | null {
  const b = board.toLowerCase();
  if (!BOARDS[b]) return null;
  const [high, turtle] = b === "pocket" ? [0, "SE"] : [2, "SH"];
  return {
    board: b,
    fitted: [],
    jobs: jobs(),
    assignments: [a("arm", "SA", 2), a("osd", "SB", high), a("rates", "SC", high), a("finder", "SD", 2), a("turtle", turtle, 2), a("blackbox", "TrimRudRight"), a("vtx", "P1"), a("sim1", "TrimAilRight"), a("sim2", "TrimEleUp"), a("sim3", "TrimEleDown"), a("sim4", "TrimAilLeft")],
    template: { timers: [], checklist_on: null, flags: [], mixes: [], logical_switches: [], functions: [], keep_functions: [], owns: { funcs: [], tracks: [], switch_funcs: [], def_prefixes: [] }, switch_warnings: ["arm"], screens: [], lua: [] },
    aircraft: [{ file: model, name: null, was_named: [], model_id: null, copy_from: null, checklist: null, drop_timers: [], logical_switches: [], drop_functions: [], functions: [] }],
    radio: { scalars: [], select_model: null, remove_functions: [], control_types: true, delete_other_models: false },
  };
}

const factory = (s: Slot) => s[2];

function slotType(kind: BoardControlKind, raw: string): string {
  const t = TYPES[kind].find((x) => x.toLowerCase() === raw.trim().toLowerCase());
  if (!t) throw kind === "custom" ? "A customizable switch is set per model, not fitted." : `A slot of this kind is ${TYPES[kind].join(", ")}, not ${JSON.stringify(raw)}.`;
  return t;
}

/** `controls::fitted_controls`. */
function controlsOf(setup: RadioSetup): SetupControl[] {
  const slots = BOARDS[setup.board];
  const out: SetupControl[] = [];
  for (const s of slots) {
    const f = (setup.fitted ?? []).find((x) => x.control === s[0]);
    const t = f ? slotType(s[1], f.radio_type) : factory(s);
    if (t.toLowerCase() === "none") continue;
    const kind: RadioControlKind = s[1] === "switch" ? (t === "3POS" ? "three_pos" : t === "TOGGLE" ? "momentary" : "two_pos") : s[1] === "custom" ? "custom" : s[1];
    out.push({ name: s[0], kind, radio_type: s[1] === "custom" ? null : t });
  }
  for (const t of TRIMS) out.push({ name: t, kind: "trim", radio_type: null });
  return out;
}

const problem = (job: string | null, control: string | null, message: string): SetupProblem => ({ job, control, message });

/** `generate::problems`. */
function problemsOf(setup: RadioSetup): { controls: SetupControl[]; problems: SetupProblem[] } {
  if (!BOARDS[setup.board]) return { controls: [], problems: [problem(null, null, `QuadCam has no control list for the board "${setup.board}"; a setup needs one.`)] };
  let controls: SetupControl[];
  try {
    controls = controlsOf(setup);
  } catch (e) {
    return { controls: [], problems: [problem(null, null, String(e))] };
  }
  const out: SetupProblem[] = [];
  const used = new Map<string, string>();
  for (const x of setup.assignments) {
    const j = setup.jobs.find((y) => y.id === x.job);
    if (!j) {
      out.push(problem(null, x.control, `The assignment names "${x.job}", which is not a job.`));
      continue;
    }
    const c = controls.find((y) => y.name === x.control);
    if (!c) {
      const blanked = (setup.fitted ?? []).some((f) => f.control === x.control && f.radio_type.toLowerCase() === "none");
      out.push(problem(x.job, x.control, blanked ? `${x.control} is set as not fitted, so it cannot do ${j.label}.` : `${x.control} is not a control of the ${setup.board}.`));
      continue;
    }
    if (!j.fits.includes(c.kind)) {
      out.push(problem(x.job, x.control, `${c.name} cannot do ${j.label}: it is a ${KIND_WORDS[c.kind]}, and ${j.label} takes ${j.fits.map((k) => KIND_WORDS[k]).join(" or ")}.`));
      continue;
    }
    if (c.kind === "custom") {
      out.push(problem(x.job, x.control, `${c.name} is a customizable switch; QuadCam does not set those up yet.`));
      continue;
    }
    const other = used.get(c.name);
    if (other) out.push(problem(x.job, x.control, `${c.name} does two jobs: ${other} and ${j.label}.`));
    used.set(c.name, j.label);
  }
  return { controls, problems: out };
}

const switchy = (k: RadioControlKind | undefined) => k === "two_pos" || k === "three_pos" || k === "momentary" || k === "custom";

/** `generate::job_rows`. */
function jobRows(setup: RadioSetup, controls: SetupControl[]): JobRow[] {
  const used = new Set(setup.assignments.map((x) => x.control));
  return setup.jobs.map((j) => {
    const asg = setup.assignments.find((x) => x.job === j.id);
    const c = asg ? controls.find((x) => x.name === asg.control) : undefined;
    const n = j.states?.length ?? 0;
    const states = (j.states ?? []).map((words, i) => {
      const us = n > 1 ? 1000 + Math.floor((1000 * i) / (n - 1)) : null;
      let position: string | null = null;
      if (c && switchy(c.kind) && asg?.high != null) {
        const atHigh = n > 1 ? Math.floor((i * 2) / (n - 1)) : 2;
        const p = asg.high === 2 ? atHigh : 2 - atHigh;
        if (c.kind === "three_pos" || p !== 1) position = `${c.name}${p}`;
      }
      return { words, position, us };
    });
    return {
      job: j.id,
      label: j.label,
      channel: j.channel != null ? `CH${j.channel + 1}` : null,
      control: c?.name ?? null,
      kind: c?.kind ?? null,
      high: asg?.high ?? null,
      states,
      free_controls: controls.filter((x) => j.fits.includes(x.kind) && x.kind !== "custom" && !used.has(x.name)).map((x) => x.name),
    };
  });
}

function slotRows(setup: RadioSetup): SlotRow[] {
  return BOARDS[setup.board]
    .filter((s) => s[1] !== "custom")
    .map((s) => {
      const f = (setup.fitted ?? []).find((x) => x.control === s[0]);
      return { control: s[0], kind: s[1], factory: factory(s), fitted: f ? slotType(s[1], f.radio_type) : factory(s), on_radio: factory(s), choices: TYPES[s[1]] };
    });
}

function boardOf(g: MockGear, device: string): { board: string; source: string } {
  const d = g.devices.find((x) => x.id === device);
  if (!d) throw `No device "${device}" in QuadCam's list (quadcam-cli gear devices).`;
  if (d.kind !== "radio") throw `"${device}" is not a radio.`;
  const c = g.connected.find((x) => x.id === device && x.link.kind === "volume");
  const board = (c?.identity?.board ?? d.identity?.board ?? "").toLowerCase();
  if (c) return { board, source: "card" };
  if (!d.last_backup) throw `No backup of "${device}" yet: back it up, or plug the radio in (USB Storage).`;
  return { board, source: `backup ${d.last_backup} (2026-10-06 18:20)` };
}

function openChange(g: MockGear, device: string): StagedChange | undefined {
  return g.changeStore.changes.find((c) => c.device === device && c.title === SETUP_CHANGE && (c.status === "draft" || c.status === "ready"));
}

const importsOf = (c: StagedChange | undefined): Edit[] => (c?.edits ?? []).filter((e) => e.kind === "model_import");

function detail(device: string, source: string, setup: RadioSetup, origin: string, prefix: Edit[]): RadioSetupDetail {
  const { controls, problems } = problemsOf(setup);
  const copies = prefix.flatMap((e) => (e.kind === "model_import" ? [{ to: e.to, name: e.name, from_board: e.from_board }] : []));
  const d: RadioSetupDetail = {
    device,
    source,
    origin,
    setup,
    controls,
    slots: BOARDS[setup.board] ? slotRows(setup) : [],
    jobs: jobRows(setup, controls),
    problems,
    card_problem: null,
    copies,
    sounds: [],
    warnings: [],
    edits: [],
    files: [],
    diff: [],
  };
  if (problems.length) return d;
  const mixes = setup.assignments.flatMap((x) => {
    const j = setup.jobs.find((y) => y.id === x.job);
    return j?.channel != null ? [{ op: "set_mixes" as const, channel: j.channel, lines: [{ source: x.high === 0 ? `-${x.control}` : x.control, weight: 100, swtch: "", mltpx: "ADD" }] }] : [];
  });
  const fitted = (setup.fitted ?? []).map((f) => ({ op: "set_control_type" as const, control: f.control, value: f.radio_type }));
  const edits: Edit[] = [...prefix, ...setup.aircraft.map((x): Edit => ({ kind: "model", file: x.file, name: null, ops: mixes }))];
  if (fitted.length) edits.push({ kind: "radio", ops: fitted });
  d.edits = edits;
  d.files = [...copies.map((c) => `MODELS/${c.to}`), ...setup.aircraft.map((x) => `MODELS/${x.file}`).filter((f) => !copies.some((c) => `MODELS/${c.to}` === f)), ...(fitted.length ? ["RADIO/radio.yml"] : [])];
  return d;
}

/** `gear_radio_setup`. */
export function view(g: MockGear, setups: MockSetups, p: { device?: string | null; setup?: RadioSetup | null }): RadioSetupDetail {
  if (!p.device) throw "Name a radio (device) or a card folder (mount).";
  const { board, source } = boardOf(g, p.device);
  const saved = setups[p.device];
  const def = defaultSetup(board);
  const [setup, origin] = p.setup ? [p.setup, "given"] : saved ? [saved, "saved"] : def ? [def, "default"] : [null, ""];
  if (!setup) throw `QuadCam has no control list for the board "${board}", so it has no setup for it.`;
  return detail(p.device, source, structuredClone(setup), origin, importsOf(openChange(g, p.device)));
}

/** `gear_radio_setup_edit`. */
export function edit(
  g: MockGear,
  setups: MockSetups,
  p: { device: string; setup?: RadioSetup | null; reset?: boolean; assign?: Assignment[]; free?: string[]; fit?: FittedSlot[]; copy_from?: SetupCopy | null; stage?: boolean; editor?: string | null },
): RadioSetupEdited {
  const { board, source } = boardOf(g, p.device);
  if (p.setup && p.reset) throw "Pass a setup or reset, not both.";
  const def = defaultSetup(board);
  let setup = structuredClone(p.setup ?? (p.reset ? def : (setups[p.device] ?? def)));
  if (!setup) throw `QuadCam has no control list for the board "${board}", so it has no setup for it.`;
  const notes: string[] = [];
  let prefix = importsOf(openChange(g, p.device));
  if (p.copy_from) {
    const from = g.devices.find((d) => d.id === p.copy_from!.from);
    if (!from) throw `"${p.copy_from.from}" is not a saved radio or a backup id.`;
    if (from.id === p.device) throw "Copy from another radio, not this one.";
    const fromBoard = (from.identity?.board ?? "").toLowerCase();
    const want = p.copy_from.models ?? [];
    const models = want.length ? FILES.filter((f) => want.includes(f.file)) : FILES;
    const taken = new Set(FILES.map((f) => f.file));
    prefix = models.map((m) => {
      let n = 0;
      while (taken.has(`model${String(n).padStart(2, "0")}.yml`)) n++;
      const to = `model${String(n).padStart(2, "0")}.yml`;
      taken.add(to);
      return { kind: "model_import", from_board: fromBoard, text: `header:\n  name: "${m.name}"\n`, to, name: m.name, links: p.copy_from!.links ?? [], template: null } as Edit;
    });
    const src = setups[from.id];
    if (src) {
      setup = { ...setup, jobs: src.jobs, template: src.template, assignments: src.assignments.map((x) => ({ ...x, control: fromBoard === "pocket" && x.control === "SE" ? "SH" : x.control })) };
    }
    setup.aircraft = prefix.flatMap((e) => (e.kind === "model_import" ? [{ ...setup!.aircraft[0], file: e.to, name: null, copy_from: null }] : []));
    notes.push(`Copies ${prefix.map((e) => (e.kind === "model_import" ? `${e.name} to ${e.to}` : "")).join(", ")} from the ${from.name || from.id} (${fromBoard}) first.`);
  }
  for (const f of p.fit ?? []) {
    const s = BOARDS[setup.board]?.find((x) => x[0] === f.control && x[1] !== "custom");
    if (!s) throw `${f.control} is not a switch or pot slot of the ${setup.board}.`;
    const t = slotType(s[1], f.radio_type);
    setup.fitted = (setup.fitted ?? []).filter((x) => x.control !== f.control);
    if (t !== factory(s)) setup.fitted.push({ control: f.control, radio_type: t });
  }
  for (const x of p.assign ?? []) setup.assignments = [...setup.assignments.filter((y) => y.job !== x.job), x];
  for (const j of p.free ?? []) {
    if (!setup.jobs.some((y) => y.id === j)) throw `"${j}" is not a job of this setup.`;
    setup.assignments = setup.assignments.filter((y) => y.job !== j);
  }
  const order = setup.jobs.map((j) => j.id);
  setup.assignments.sort((x, y) => order.indexOf(x.job) - order.indexOf(y.job));
  const first = problemsOf(setup).problems[0];
  if (first) throw first.message;
  const d = detail(p.device, source, setup, "saved", prefix);
  setups[p.device] = setup;
  const stage = !!p.stage || !!p.copy_from;
  let change: StagedChange | null = null;
  if (stage) {
    const open = openChange(g, p.device);
    if (open) change = changes.update(g, { id: open.id, edits: d.edits });
    else change = changes.stage(g, p.device, d.edits, SETUP_CHANGE, p.editor === "agent" ? "agent" : "user");
  }
  return { detail: d, change, notes };
}
