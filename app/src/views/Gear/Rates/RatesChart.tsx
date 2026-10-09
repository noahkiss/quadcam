// One rate or throttle curve: stick from centre to full on the x axis, a solid line for the
// profile and a dashed one for what it is compared with. The curve is odd (the other half
// mirrors it), so only the positive half is drawn.
import { points, ticks } from "../../../lib/rates";
import styles from "./Rates.module.css";

const W = 280;
const H = 170;
const PAD = { l: 44, r: 10, t: 10, b: 26 };
const BOX = { x: PAD.l, y: PAD.t, w: W - PAD.l - PAD.r, h: H - PAD.t - PAD.b };

interface Props {
  title: string;
  /** The accessible summary. */
  summary: string;
  main: number[];
  compare?: number[] | null;
  /** The value at the top of the chart. */
  max: number;
  /** How to print a tick. */
  tick: (v: number) => string;
}

export function RatesChart({ title, summary, main, compare, max, tick }: Props) {
  const y = (v: number) => BOX.y + BOX.h - (v / max) * BOX.h;
  return (
    <figure className={styles.figure}>
      <figcaption className={styles.caption}>{title}</figcaption>
      <svg className={styles.chart} viewBox={`0 0 ${W} ${H}`} role="img" aria-label={summary}>
        {ticks(max).map((t) => (
          <g key={t}>
            <line className={styles.grid} x1={BOX.x} x2={BOX.x + BOX.w} y1={y(t)} y2={y(t)} />
            <text className={styles.tick} x={BOX.x - 6} y={y(t)} textAnchor="end" dominantBaseline="middle">
              {tick(t)}
            </text>
          </g>
        ))}
        {[0, 0.5, 1].map((s) => (
          <text key={s} className={styles.tick} x={BOX.x + s * BOX.w} y={H - 8} textAnchor={s === 0 ? "start" : s === 1 ? "end" : "middle"}>
            {s === 0 ? "centre" : s === 1 ? "full stick" : "half"}
          </text>
        ))}
        {compare && compare.length > 1 && <polyline className={styles.compare} fill="none" points={points(compare, BOX, max)} />}
        <polyline className={styles.line} fill="none" points={points(main, BOX, max)} />
      </svg>
    </figure>
  );
}
