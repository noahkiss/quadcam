// The radio setup editor's pure helpers: the words for controls, positions and fitted parts,
// the pop-ups' choices, the next setup after one change, and Learn (which control moved on
// the radio in USB Joystick mode, read through the switch map of the radio's model).
import type { BoardControlKind, JobRow, RadioControlKind, RadioSetup, RadioSetupDetail, SetupProblem, SwitchMap } from "../ipc/types";
import { live } from "./controls";

/** Switches have positions (`SA0`..`SA2`); pots, sliders and trims do not. */
export const hasPositions = (k: RadioControlKind | null | undefined) => k === "two_pos" || k === "three_pos" || k === "momentary" || k === "custom";

const TRIM_WORDS: Record<string, string> = {
  TrimRudLeft: "Rudder trim left",
  TrimRudRight: "Rudder trim right",
  TrimEleDown: "Elevator trim down",
  TrimEleUp: "Elevator trim up",
  TrimThrDown: "Throttle trim down",
  TrimThrUp: "Throttle trim up",
  TrimAilLeft: "Aileron trim left",
  TrimAilRight: "Aileron trim right",
};

/** A control as the radio labels it (`SA`, `P1`), a trim in words. */
export const controlText = (name: string) => TRIM_WORDS[name] ?? name;

export const KIND_TEXT: Record<RadioControlKind, string> = {
  two_pos: "2-position switch",
  three_pos: "3-position switch",
  momentary: "Momentary switch",
  custom: "Customizable switch",
  pot: "Pot",
  slider: "Slider",
  trim: "Trim",
};

/** A switch position's name: 0 is up, 1 the middle, 2 down. */
export const POSITION_WORDS = ["up", "middle", "down"];
/** A momentary switch's: 0 is released, 2 pressed. */
export const MOMENTARY_WORDS = ["released", "", "pressed"];

/** A control at a position: `SC down`, `SH pressed`. */
export const atText = (control: string, p: number, kind?: RadioControlKind | null) => `${control} ${(kind === "momentary" ? MOMENTARY_WORDS : POSITION_WORDS)[p]}`;

/** `SC2` as `SC down`; anything else as it is. */
export function positionText(position: string, kind?: RadioControlKind | null): string {
  const m = /^(S[A-Z]|SW\d)([0-2])$/.exec(position);
  return m ? atText(m[1], Number(m[2]), kind) : controlText(position);
}

/** A fitted part in words, from the type `radio.yml` takes. */
export function slotTypeText(kind: BoardControlKind, t: string): string {
  switch (t.toLowerCase()) {
    case "2pos":
      return "2-position switch";
    case "3pos":
      return "3-position switch";
    case "toggle":
      return "Momentary switch";
    case "with_detent":
      return "Pot with centre detent";
    case "without_detent":
      return kind === "pot" ? "Pot without detent" : "Wheel";
    case "slider":
      return "Slider";
    case "none":
      return "Not fitted";
    default:
      return t;
  }
}

/** The Control pop-up's choices for a job: its control, then the free ones that fit. */
export function controlChoices(row: JobRow): string[] {
  const out = row.control ? [row.control] : [];
  for (const c of row.free_controls) if (!out.includes(c)) out.push(c);
  return out;
}

/** The problems that belong to a job's row. */
export const problemsFor = (problems: SetupProblem[] | undefined, job: string) => (problems ?? []).filter((p) => p.job === job);

/** Problems that belong to no row. */
export const generalProblems = (problems: SetupProblem[] | undefined) => (problems ?? []).filter((p) => !p.job);

/** The direction a new switch assignment starts with: the job's last one, else down. */
export const defaultHigh = (row: JobRow) => (row.high === 0 ? 0 : 2);

/** The setup with a job on `control` (null: no control), and `high` for a switch. */
export function assign(setup: RadioSetup, kinds: Record<string, RadioControlKind>, job: string, control: string | null, high: number | null): RadioSetup {
  const rest = setup.assignments.filter((a) => a.job !== job);
  if (!control) return { ...setup, assignments: rest };
  const a = { job, control, high: hasPositions(kinds[control]) ? (high ?? 2) : null };
  const order = setup.jobs.map((j) => j.id);
  const assignments = [...rest, a].sort((x, y) => order.indexOf(x.job) - order.indexOf(y.job));
  return { ...setup, assignments };
}

/** The setup with a slot fitted with `type`; the factory part takes the slot off the list. */
export function fit(setup: RadioSetup, control: string, type: string, factory: string): RadioSetup {
  const fitted = (setup.fitted ?? []).filter((f) => f.control !== control);
  if (type.toLowerCase() !== factory.toLowerCase()) fitted.push({ control, radio_type: type });
  return { ...setup, fitted };
}

/** The controls' kinds by name. */
export const kindsOf = (d: RadioSetupDetail): Record<string, RadioControlKind> => Object.fromEntries(d.controls.map((c) => [c.name, c.kind]));

/** One line of the status above the table. */
export function statusText(d: RadioSetupDetail): string {
  if (d.problems?.length) return "Not saved: fix the problems below.";
  if (d.card_problem) return "The radio's card does not take this setup yet.";
  if (!d.files.length) return "The radio matches this setup.";
  return d.files.length === 1 ? "1 file on the radio differs from this setup." : `${d.files.length} files on the radio differ from this setup.`;
}

/** Where the setup was read, in words. */
export function sourceWords(source: string): string {
  if (source === "card") return "From the card";
  const b = /^backup \S+ \((.+)\)$/.exec(source);
  if (b) return `From backup · ${b[1]}`;
  if (source.startsWith("backup ")) return "From a backup";
  return source.startsWith("folder ") ? "From a card folder" : source;
}

// ----- Learn -----

/** Where each control of the map sits now, by row id. */
export type LearnBaseline = Record<string, number | null>;

/** The positions the radio shows now. */
export const learnBaseline = (map: SwitchMap, channels: number[]): LearnBaseline => live(map, "radio", channels).positions;

/** What moved: the control, and for a switch the position it moved to. */
export interface LearnPick {
  control: string;
  /** 0 or 2 for a switch moved to an end; null for a trim. */
  high: number | null;
}

/** The control that moved away from `baseline`, once it rests at an end (a switch) or is
 *  pressed (a trim). Sticks never count. Null while nothing has. */
export function learnPick(map: SwitchMap, baseline: LearnBaseline, channels: number[]): LearnPick | null {
  const now = live(map, "radio", channels).positions;
  for (const r of map.rows) {
    if (r.kind === "stick") continue;
    const at = now[r.id];
    if (at == null || at === baseline[r.id]) continue;
    const source = r.positions[at]?.source;
    if (!source) continue;
    if (r.kind === "trim") return { control: r.id, high: null };
    const m = /^(S[A-Z]|SW\d)([0-2])$/.exec(source);
    if (!m || m[2] === "1") continue;
    return { control: m[1], high: Number(m[2]) };
  }
  return null;
}

/** What a Learn pick does to a job: the assignment, or why it cannot. */
export function learnResult(row: JobRow, pick: LearnPick, kinds: Record<string, RadioControlKind>): { control: string; high: number | null } | { error: string } {
  const kind = kinds[pick.control];
  if (!kind) return { error: `${controlText(pick.control)} moved, but it is not a control of this setup's radio.` };
  if (pick.control !== row.control && !row.free_controls.includes(pick.control)) {
    return { error: `${controlText(pick.control)} moved, but it cannot do ${row.label}: it is a ${KIND_TEXT[kind].toLowerCase()}, or another job uses it.` };
  }
  return { control: pick.control, high: hasPositions(kind) ? pick.high : null };
}
