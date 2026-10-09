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

/** The box an element's cells take on the grid, before clipping. The horizon draws its level line at x-4, y+4. */
function rect(e: OsdElement) {
  return e.name === "ah" ? { x: e.x - 4, y: e.y + 4, w: 9, h: 1 } : { x: e.x, y: e.y, w: e.width, h: e.height };
}

function profile(base: OsdProfile, els: OsdElement[], w: number, h: number): OsdProfile {
  const on = els.filter((e) => e.profiles.includes(base.index));
  const rows = Array.from({ length: h }, () => Array.from({ length: w }, () => " "));
  const boxes: OsdBox[] = [];
  const problems: OsdProblem[] = [];
  for (const e of on) {
    const r = rect(e);
    const x0 = Math.max(0, r.x), y0 = Math.max(0, r.y);
    const x1 = Math.min(w, r.x + r.w), y1 = Math.min(h, r.y + r.h);
    if (x1 > x0 && y1 > y0) boxes.push({ element: e.name, label: e.label, x: x0, y: y0, width: x1 - x0, height: y1 - y0 });
    if (r.h === 1) [...e.sample].forEach((ch, i) => {
      const cx = r.x + i;
      if (cx >= 0 && cx < w && r.y >= 0 && r.y < h) rows[r.y][cx] = ch;
    });
    const off = r.w * r.h - Math.max(0, x1 - x0) * Math.max(0, y1 - y0);
    if (off > 0) problems.push({ kind: "off_screen", element: e.name, other: null, cells: off, message: `${e.name}: ${off} cells off screen.` });
  }
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i], b = boxes[j];
      // The crosshairs sit on the horizon's level line by design.
      if ([a.element, b.element].sort().join() === "ah,crosshairs") continue;
      const cells = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));
      if (cells > 0) problems.push({ kind: "overlap", element: a.element, other: b.element, cells, message: `${a.element} overlaps ${b.element} (${cells} cells).` });
    }
  }
  return { ...base, elements: on.map((e) => e.name), rows: rows.map((r) => r.join("")), boxes, problems, notes: [] };
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
