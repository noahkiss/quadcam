import type { PlaceTrend } from "../../../ipc/types";
import { fmtDay } from "../../../lib/format";
import { hhmm, num } from "../../../lib/flights";
import styles from "./RangeTrend.module.css";

const W = 560;
const H = 160;
const PAD = { l: 40, r: 12, t: 10, b: 24 };
const TICKS = [0, 50, 100];

/** Gear > Flights: the worst link quality of each flight at each place, oldest first. */
export function RangeTrend({ places }: { places: PlaceTrend[] }) {
  if (!places.length) return null;
  return (
    <section className={styles.section} aria-label="Range by place">
      <h3 className={styles.title}>Range by place</h3>
      {places.map((p) => (
        <PlaceChart key={p.place} trend={p} />
      ))}
    </section>
  );
}

function PlaceChart({ trend }: { trend: PlaceTrend }) {
  const pts = [...trend.points].sort((a, b) => a.start.localeCompare(b.start));
  const withLq = pts.filter((p) => p.worst_lq != null);
  const x = (i: number) => PAD.l + (withLq.length < 2 ? (W - PAD.l - PAD.r) / 2 : (i * (W - PAD.l - PAD.r)) / (withLq.length - 1));
  const y = (v: number) => PAD.t + (1 - Math.min(100, Math.max(0, v)) / 100) * (H - PAD.t - PAD.b);
  const first = withLq[0];
  const last = withLq[withLq.length - 1];
  const summary = first
    ? `Worst link quality at ${trend.place}: ${withLq.length} ${withLq.length === 1 ? "flight" : "flights"}, from ${num(first.worst_lq, 0, "%")} on ${fmtDay(first.start.slice(0, 10), true)} to ${num(last.worst_lq, 0, "%")} on ${fmtDay(last.start.slice(0, 10), true)}.`
    : `No link quality was logged at ${trend.place}.`;
  return (
    <figure className={styles.figure}>
      <figcaption className={styles.caption}>{trend.place}</figcaption>
      {first ? (
        <svg className={styles.chart} viewBox={`0 0 ${W} ${H}`} role="img" aria-label={summary}>
          {TICKS.map((t) => (
            <g key={t}>
              <line className={styles.grid} x1={PAD.l} x2={W - PAD.r} y1={y(t)} y2={y(t)} />
              <text className={styles.tick} x={PAD.l - 6} y={y(t)} textAnchor="end" dominantBaseline="middle">
                {t} %
              </text>
            </g>
          ))}
          <polyline className={styles.line} fill="none" points={withLq.map((p, i) => `${x(i)},${y(p.worst_lq as number)}`).join(" ")} />
          {withLq.map((p, i) => (
            <circle key={p.flight} className={styles.dot} cx={x(i)} cy={y(p.worst_lq as number)} r={3.5} />
          ))}
          <text className={styles.tick} x={PAD.l} y={H - 6}>
            {fmtDay(first.start.slice(0, 10), true)}
          </text>
          {withLq.length > 1 && (
            <text className={styles.tick} x={W - PAD.r} y={H - 6} textAnchor="end">
              {fmtDay(last.start.slice(0, 10), true)}
            </text>
          )}
        </svg>
      ) : (
        <p className={styles.muted}>{summary}</p>
      )}
      <details className={styles.details}>
        <summary>Values for {trend.place}</summary>
        <table className={styles.table} aria-label={`Worst link at ${trend.place}`}>
          <thead>
            <tr>
              <th scope="col">Flight</th>
              <th scope="col">Worst LQ</th>
              <th scope="col">Worst RSSI</th>
            </tr>
          </thead>
          <tbody>
            {pts.map((p) => (
              <tr key={p.flight}>
                <th scope="row">
                  {fmtDay(p.start.slice(0, 10), true)} {hhmm(p.start)}
                </th>
                <td>{num(p.worst_lq, 0, "%")}</td>
                <td>{num(p.worst_rssi_db, 0, "dB")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}
