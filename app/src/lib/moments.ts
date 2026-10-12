// Moments across clips: the density strip of one clip, and the best moments of many.
import type { LibClip, Moment } from "../ipc/types";

/** A moment worth showing: anything but dead air. */
export const interesting = (m: Moment) => m.kind !== "dead_air";

/** Where a clip's moments are, as `bins` values from 0 to 1. Each moment adds its score over
 *  the bins it covers, spread a little to each side; the busiest bin is 1. Empty when the
 *  clip has no moments. */
export function heat(moments: Moment[], duration: number, bins: number): number[] {
  const ms = (moments || []).filter(interesting);
  if (!ms.length || !(duration > 0) || bins < 1) return [];
  const out = new Array<number>(bins).fill(0);
  const w = duration / bins;
  const spread = [0.25, 0.5, 1, 0.5, 0.25];
  for (const m of ms) {
    const a = Math.max(0, Math.min(bins - 1, Math.floor(m.start / w)));
    const b = Math.max(a, Math.min(bins - 1, Math.floor(Math.max(m.start, m.end - 1e-9) / w)));
    for (let i = a; i <= b; i++)
      spread.forEach((k, j) => {
        const at = i + j - 2;
        if (at >= 0 && at < bins) out[at] += k * m.score;
      });
  }
  const max = Math.max(...out);
  return max > 0 ? out.map((v) => v / max) : [];
}

/** The strip as a CSS gradient: the accent colour, stronger where the moments are. A bin
 *  with any moment shows at 30 % or more, so a single weak moment still reads. */
export function heatGradient(values: number[], color = "var(--accent)"): string | undefined {
  if (!values.length) return undefined;
  const n = values.length;
  const stops = values.map((v, i) => `color-mix(in srgb, ${color} ${v > 0 ? Math.round(30 + v * 70) : 0}%, transparent) ${((i / n) * 100).toFixed(2)}% ${(((i + 1) / n) * 100).toFixed(2)}%`);
  return `linear-gradient(to right, ${stops.join(", ")})`;
}

export interface ClipMoment {
  clip: LibClip;
  moment: Moment;
}

/** Every interesting moment of `clips`, best score first; ties go to the newer day, then
 *  the earlier moment. `limit` keeps the first few. */
export function bestMoments(clips: LibClip[], limit?: number): ClipMoment[] {
  const all = clips.flatMap((clip) => (clip.moments || []).filter(interesting).map((moment) => ({ clip, moment })));
  all.sort((a, b) => b.moment.score - a.moment.score || b.clip.date.localeCompare(a.clip.date) || a.moment.start - b.moment.start);
  return limit == null ? all : all.slice(0, limit);
}
