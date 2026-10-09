// The radio's controls as the UI draws them: which hand holds which stick in each mode,
// what each stick axis does, the sticks read back from channel values, and a switch map
// matched to live channel values. `live` follows `gear::switchmap::live` in the core: the
// core marks MSP reads, this marks the joystick stream between them.
import type { Live, Stick, StickChannel, SwitchMap } from "../ipc/types";

/** The joystick's buttons (EdgeTX: 24) and axes (8, CH1-8). */
export const BUTTONS = 24;

export type StickMode = 1 | 2 | 3 | 4;
export const STICK_MODES: StickMode[] = [1, 2, 3, 4];
export const DEFAULT_MODE: StickMode = 2;

/** -1..1 per stick: up and right are positive. */
export type StickValues = Record<Stick, number>;

/** One hand's stick: the axis it moves up and down, and the one left and right. */
export interface Gimbal {
  v: Stick;
  h: Stick;
}

/** The two sticks in each mode. Mode 2: throttle and yaw on the left. */
export const MODE_LAYOUT: Record<StickMode, { left: Gimbal; right: Gimbal }> = {
  1: { left: { v: "pitch", h: "yaw" }, right: { v: "throttle", h: "roll" } },
  2: { left: { v: "throttle", h: "yaw" }, right: { v: "pitch", h: "roll" } },
  3: { left: { v: "pitch", h: "roll" }, right: { v: "throttle", h: "yaw" } },
  4: { left: { v: "throttle", h: "roll" }, right: { v: "pitch", h: "yaw" } },
};

/** Each axis' name and what moving it does. */
export const AXIS_TEXT: Record<Stick, { name: string; motion: string }> = {
  throttle: { name: "Throttle", motion: "Up and down: power" },
  yaw: { name: "Yaw", motion: "Left and right: turns the nose" },
  pitch: { name: "Pitch", motion: "Up and down: tilts forward and back" },
  roll: { name: "Roll", motion: "Left and right: banks" },
};

/** CH1-4 as AETR, the EdgeTX default, for a radio with no model read. */
export const DEFAULT_STICKS: StickChannel[] = [
  { stick: "roll", ch: 1, weight: 100 },
  { stick: "pitch", ch: 2, weight: 100 },
  { stick: "throttle", ch: 3, weight: 100 },
  { stick: "yaw", ch: 4, weight: 100 },
];

export const asMode = (v: unknown): StickMode => (STICK_MODES.includes(v as StickMode) ? (v as StickMode) : DEFAULT_MODE);

const clamp = (x: number) => Math.max(-1, Math.min(1, x));

/** The sticks from channel values (µs, CH1 first) and the channels the model gives them. */
export function sticksFrom(channels: number[], map: StickChannel[] = DEFAULT_STICKS): StickValues {
  const out: StickValues = { roll: 0, pitch: 0, throttle: -1, yaw: 0 };
  for (const s of map) {
    const us = channels[s.ch - 1];
    if (us == null || !s.weight) continue;
    out[s.stick] = clamp((us - 1500) / 512 / (s.weight / 100));
  }
  return out;
}

/** A percent for a stick value. */
export const pct = (x: number) => `${x > 0 ? "+" : ""}${Math.round(x * 100)}%`;

/** Channel values within this many µs of a position's value count as that position. */
export const LIVE_TOLERANCE_US = 60;

const inRange = (v: number, start: number, end: number) => v >= start && (v < end || end >= 2100);
const selectPosition = (us: number) => Math.min(2, Math.floor(Math.max(0, us - 900) / 400));

/** The map matched to channel values: each row's position, the modes on, the selections. */
export function live(map: SwitchMap, source: string, channels: number[]): Live {
  const at = (ch: number): number | undefined => channels[ch - 1];
  const positions: Record<string, number | null> = {};
  for (const r of map.rows) {
    let best: [number, number] | null = null;
    r.positions.forEach((p, i) => {
      // The position alone, or with other controls moved: a combination's channels replace the position's own.
      const variants = [p.channels, ...p.combos.map((c) => [...p.channels.filter((cv) => !c.channels.some((x) => x.ch === cv.ch)), ...c.channels])];
      for (const chs of variants) {
        if (!chs.length) continue;
        let total = 0;
        let ok = true;
        for (const cv of chs) {
          const v = at(cv.ch);
          if (v == null || Math.abs(v - cv.us) > LIVE_TOLERANCE_US) ok = false;
          else total += Math.abs(v - cv.us);
        }
        if (ok && (!best || total < best[1])) best = [i, total];
      }
    });
    positions[r.id] = best ? best[0] : null;
  }
  const modes: string[] = [];
  for (const m of map.modes) {
    const v = at(m.ch);
    if (m.linked == null && v != null && inRange(v, m.start, m.end) && !modes.includes(m.name)) modes.push(m.name);
  }
  const adjustments: string[] = [];
  for (const a of map.adjustments) {
    const r = at(a.range_ch);
    const s = at(a.select_ch);
    if (r == null || s == null || !inRange(r, a.start, a.end)) continue;
    adjustments.push(a.select ? `${a.name} ${selectPosition(s) + 1}` : `${a.name} adjust`);
  }
  return { source, channels, positions, modes, adjustments };
}
