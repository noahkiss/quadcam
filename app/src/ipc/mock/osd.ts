// The mock core's OSD editor (`core/osd.rs`, `gear/osd.rs`), written by hand: the working view
// is the recorded PAL view with the staged OSD edits on top, redrawn and checked as the core
// does, and `gear_osd_edit` joins moves into one "OSD layout" change.
import type { Edit, OsdBox, OsdElement, OsdProblem, OsdProfile, OsdView, StagedChange } from "../types";
import type { MockGear } from "./gear";
import * as changes from "./changes";
import * as seed from "./seed";

type OsdEdit = Extract<Edit, { kind: "osd_element" }>;
export const OSD_CHANGE = "OSD layout";
const isOsd = (e: Edit): e is OsdEdit => e.kind === "osd_element";
const name = (s: string) => s.trim().toLowerCase().replace(/^osd_/, "").replace(/_pos$/, "");

// The drawing and the check follow `gear/osd.rs` `render_profile`: each element draws its
// cells in order (the camera frame first, the crosshairs last, the rest by name), a cell
// another element already drew counts as an overlap, and the horizon draws its level line
// while its full pitch sweep is checked against the screen on its own.
const HORIZON_HALF = 4;
const HORIZON_ROWS = 10;
const STICK = { w: 7, h: 5 };

/** The cells an element draws, as `gear/osd.rs` `cells` does. */
function cells(e: OsdElement): [number, number, string][] {
  const out: [number, number, string][] = [];
  if (e.name === "ah") {
    for (let dx = -HORIZON_HALF; dx <= HORIZON_HALF; dx++) out.push([e.x + dx, e.y + HORIZON_ROWS / 2 - 1, "="]);
  } else if (e.name.startsWith("stick_overlay")) {
    for (let dy = 0; dy < STICK.h; dy++) for (let dx = 0; dx < STICK.w; dx++) out.push([e.x + dx, e.y + dy, dx === (STICK.w >> 1) || dy === (STICK.h >> 1) ? "+" : " "]);
  } else if (e.name === "camera_frame") {
    for (let dy = 0; dy < e.height; dy++) for (let dx = 0; dx < e.width; dx++) {
      const ex = dx === 0 || dx === e.width - 1, ey = dy === 0 || dy === e.height - 1;
      if (ex || ey) out.push([e.x + dx, e.y + dy, ex && ey ? "+" : ey ? "-" : "|"]);
    }
  } else {
    const chars = [...e.sample.padEnd(e.width, " ")].slice(0, e.width);
    chars.forEach((ch, i) => out.push([e.x + i, e.y, ch]));
  }
  return out;
}

const mayOverlap = (a: string, b: string) => [a, b].sort().join() === "ah,crosshairs" || a === "camera_frame" || b === "camera_frame";
const drawOrder = (n: string) => (n === "camera_frame" ? 0 : n === "crosshairs" ? 2 : 1);

function profile(base: OsdProfile, els: OsdElement[], w: number, h: number): OsdProfile {
  const on = els.filter((e) => e.profiles.includes(base.index)).sort((a, b) => drawOrder(a.name) - drawOrder(b.name) || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  const rows = Array.from({ length: h }, () => Array.from({ length: w }, () => " "));
  const owner: (string | null)[][] = Array.from({ length: h }, () => Array.from({ length: w }, () => null));
  const boxes: OsdBox[] = [];
  const problems: OsdProblem[] = [];
  const notes: string[] = [];
  const off = new Map<string, number>();
  const overlaps = new Map<string, number>();
  for (const e of on) {
    if (e.name === "ah_sbar") {
      notes.push(`${e.name} is on: drawn around the screen centre, not shown here.`);
      continue;
    }
    if (e.name === "ah") {
      const left = e.x - HORIZON_HALF, right = e.x + HORIZON_HALF, bottom = e.y + HORIZON_ROWS - 1;
      if (left < 0 || right >= w || bottom >= h) problems.push({ kind: "off_screen", element: e.name, other: null, cells: 0, message: `${e.name}: the moving horizon reaches off screen (columns ${left}-${right}, rows ${e.y}-${bottom}).` });
      notes.push(`${e.name}: the level line is drawn; at full pitch it sweeps rows ${e.y}-${bottom}.`);
    }
    let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const [cx, cy, ch] of cells(e)) {
      if (cx < 0 || cx >= w || cy < 0 || cy >= h) {
        if (e.name !== "ah") off.set(e.name, (off.get(e.name) ?? 0) + 1);
        continue;
      }
      const prev = owner[cy][cx];
      if (prev && prev !== e.name && !mayOverlap(prev, e.name)) overlaps.set(`${e.name}\n${prev}`, (overlaps.get(`${e.name}\n${prev}`) ?? 0) + 1);
      owner[cy][cx] = e.name;
      rows[cy][cx] = ch;
      [x0, y0, x1, y1] = [Math.min(x0, cx), Math.min(y0, cy), Math.max(x1, cx), Math.max(y1, cy)];
    }
    if (x0 <= x1) boxes.push({ element: e.name, label: e.label, x: x0, y: y0, width: x1 - x0 + 1, height: y1 - y0 + 1 });
  }
  for (const [name, n] of [...off].sort()) problems.push({ kind: "off_screen", element: name, other: null, cells: n, message: `${name}: ${n} cells off screen.` });
  for (const [key, n] of [...overlaps].sort()) {
    const [a, b] = key.split("\n");
    problems.push({ kind: "overlap", element: a, other: b, cells: n, message: `${a} overlaps ${b} (${n} cells).` });
  }
  return { ...base, elements: on.map((e) => e.name), rows: rows.map((r) => r.join("")), boxes, problems, notes };
}

/** The recorded view with `edits` applied and every profile redrawn and checked. */
export function withEdits(view: OsdView, edits: Edit[]): OsdView {
  const osd = edits.filter(isOsd);
  if (!osd.length) return view;
  const els = view.elements.map((e) => ({ ...e }));
  for (const o of osd) {
    const e = els.find((x) => x.name === name(o.element));
    if (e) Object.assign(e, { x: o.x, y: o.y, profiles: [...o.profiles].sort(), value: changes.encodePos(o.x, o.y, o.profiles) });
  }
  const profiles = view.profiles.map((p) => profile(p, els, view.grid.width, view.grid.height));
  const n = osd.length;
  return { ...view, elements: els, profiles, ok: profiles.every((p) => !p.problems.length), notes: [...view.notes, `Shows ${n} staged OSD ${n === 1 ? "edit" : "edits"}: not on the FC until applied.`] };
}

/** The OSD edits the FC holds from applied changes, then the staged ones. */
function applied(g: MockGear, device: string): Edit[] {
  return g.changeStore.changes.filter((c) => c.device === device && ["applied", "verified"].includes(c.status)).flatMap((c) => c.edits.filter(isOsd));
}
function stagedEdits(g: MockGear, device: string): Edit[] {
  return changes.list(g, device, false).flatMap((c) => c.edits.filter(isOsd));
}

/** `gear_osd`. */
export function view(g: MockGear, device: string | null, staged: boolean): OsdView {
  const v = withEdits(seed.osd(), device ? applied(g, device) : []);
  v.notes = [];
  return staged && device ? withEdits(v, stagedEdits(g, device)) : v;
}

interface Move { element: string; x?: number | null; y?: number | null; profiles?: number[] | null }

/** `gear_osd_edit`: one open "OSD layout" change per device. */
export function edit(g: MockGear, p: { device: string; moves?: Move[]; copy?: { from: number; to: number } | null }, editor: "user" | "agent"): StagedChange {
  const moves = p.moves ?? [];
  if (!moves.length && !p.copy) throw "Pass at least one move, or a profile copy.";
  const d = g.devices.find((x) => x.id === p.device);
  if (!d?.last_backup) throw `No FC backup of "${p.device}" yet: back it up, or pass a dump file.`;
  const base = view(g, p.device, false);
  const open = changes.list(g, p.device, false).find((c) => c.title === OSD_CHANGE && ["draft", "ready"].includes(c.status) && c.edits.every(isOsd));
  let now = open ? withEdits(base, open.edits) : base;
  const fresh: OsdEdit[] = [];
  const push = (e: OsdEdit) => {
    fresh.push(e);
    now = withEdits(now, [e]);
  };
  for (const m of moves) {
    const el = now.elements.find((e) => e.name === name(m.element));
    if (!el) throw `The layout lists no osd_${name(m.element)}_pos; check the name with \`gear osd\`.`;
    const profiles = m.profiles ?? el.profiles;
    if (profiles.some((q) => q < 1 || q > 3)) throw `OSD profile ${profiles.find((q) => q < 1 || q > 3)} does not exist: Betaflight has profiles 1-3.`;
    const x = m.x ?? el.x, y = m.y ?? el.y;
    if (x > 63 || y > 31 || x < 0 || y < 0) throw `osd_${el.name}_pos takes x 0-63 and y 0-31, not ${x},${y}.`;
    push({ kind: "osd_element", element: el.name, x, y, profiles });
  }
  if (p.copy) {
    const { from, to } = p.copy;
    for (const q of [from, to]) if (q < 1 || q > 3) throw `OSD profile ${q} does not exist: Betaflight has profiles 1-3.`;
    for (const el of now.elements) {
      if (el.profiles.includes(from) === el.profiles.includes(to)) continue;
      const profiles = el.profiles.includes(from) ? [...el.profiles, to] : el.profiles.filter((q) => q !== to);
      push({ kind: "osd_element", element: el.name, x: el.x, y: el.y, profiles: profiles.sort() });
    }
  }
  const keep: OsdEdit[] = [];
  for (const e of [...(open?.edits.filter(isOsd) ?? []), ...fresh]) {
    const i = keep.findIndex((k) => k.element === e.element);
    if (i >= 0) keep.splice(i, 1);
    keep.push(e);
  }
  const differs = (e: OsdEdit) => {
    const b = base.elements.find((x) => x.name === e.element)!;
    return b.x !== e.x || b.y !== e.y || b.profiles.join() !== [...e.profiles].sort().join();
  };
  const edits = keep.filter(differs);
  if (!edits.length) {
    if (open) return changes.discard(g, open.id);
    throw "Nothing changes: the FC already has that layout.";
  }
  if (open) return changes.update(g, { id: open.id, edits });
  return changes.stage(g, p.device, edits, OSD_CHANGE, editor);
}
