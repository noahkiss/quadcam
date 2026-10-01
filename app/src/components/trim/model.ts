// The trim editor's model and its pure rules, shared by the open clip and the import
// review. Ported from the legacy trim.js.
import type { Moment, Span } from "../../ipc/types";

export interface TrimCut extends Span {
  state: "saved" | "new" | "failed";
  file?: string | null;
  error?: string | null;
  /** An agent suggested it. */
  agent?: boolean;
}

export interface TrimModel {
  /** Which clip; the in and out points reset when it changes. */
  key: string;
  duration: number;
  moments: Moment[];
  deadAir: Span[];
  keep: Span[];
  cuts: TrimCut[];
  hasLog: boolean;
  logOffset: number;
  logInterval?: number | null;
}

export interface Sel {
  in: number | null;
  out: number | null;
}

export const FRAME = 1 / 30;
export const MIN_CUT = 0.5;

/** In and out points around a moment: a second either side, none for dead air. */
export function frameMoment(m: Moment, duration: number): Sel {
  const pad = m.kind === "dead_air" ? 0 : 1;
  return { in: +Math.max(0, m.start - pad).toFixed(1), out: +Math.min(duration, m.end + pad).toFixed(1) };
}

/** The cut list with one more cut, or an error message. */
export function withCut(cuts: TrimCut[], sel: Sel): Span[] | string {
  const a = sel.in;
  const b = sel.out;
  if (a == null || b == null || b - a < MIN_CUT) return "Set an in and an out point at least 0.5 s apart.";
  return [...cuts.map(({ start, end }) => ({ start, end })), { start: a, end: b }];
}

export const without = (cuts: TrimCut[], i: number): Span[] => cuts.filter((_, j) => j !== i).map(({ start, end }) => ({ start, end }));

/** The cuts plus every keep range that is not a cut yet. */
export function withKeep(cuts: TrimCut[], keep: Span[]): Span[] {
  const have = cuts.map(({ start, end }) => ({ start, end }));
  const fresh = keep.filter((k) => !have.some((c) => Math.abs(c.start - k.start) < 0.05 && Math.abs(c.end - k.end) < 0.05));
  return [...have, ...fresh];
}

/** About eight ruler ticks at round steps. */
export function rulerStep(duration: number) {
  return [1, 2, 5, 10, 15, 30, 60, 120, 300, 600].find((s) => duration / s <= 8) || 600;
}

/** A dragged point, kept 0.1 s from the other one and inside the clip. */
export function dragPoint(sel: Sel, which: "in" | "out", t: number, duration: number): Sel {
  const v = Math.max(0, Math.min(duration, Math.round(t * 10) / 10));
  return which === "in" ? { ...sel, in: Math.min(v, (sel.out ?? v) - 0.1) } : { ...sel, out: Math.max(v, (sel.in ?? v) + 0.1) };
}
