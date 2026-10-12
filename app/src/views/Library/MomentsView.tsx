import { useStore } from "../../store";
import type { LibClip, Moment } from "../../ipc/types";
import { Icon } from "../../components/Icon";
import { fmtDay, fmtDur } from "../../lib/format";
import { bestMoments } from "../../lib/moments";
import { stripStyle } from "./ClipCard";
import { KIND, KIND_ICON, SOURCE } from "./moments";
import styles from "./MomentsView.module.css";

/** The strip frame nearest a moment, 0 to 9. */
export const momentFrame = (c: LibClip, m: Moment) => Math.min(9, Math.floor((m.start / (c.duration || 1)) * 10));

/** Every moment of the clips shown, best first. A row opens its clip at the moment. */
export function MomentsView({ clips }: { clips: LibClip[] }) {
  const openDetail = useStore((s) => s.openDetail);
  const list = bestMoments(clips);
  const n = new Set(list.map((x) => x.clip.id)).size;
  return (
    <div className={styles.scroll}>
      <div className={styles.head}>
        <h2>Best moments</h2>
        <span className={styles.sub}>
          {list.length} moment{list.length === 1 ? "" : "s"} in {n} clip{n === 1 ? "" : "s"}
        </span>
      </div>
      {!list.length ? (
        <p className={styles.empty}>No moments.</p>
      ) : (
        <ol className={styles.list} aria-label="Best moments">
          {list.map(({ clip: c, moment: m }, i) => (
            <li key={`${c.id}-${i}`}>
              <button type="button" className={styles.row} aria-label={`${KIND[m.kind]} in ${c.name} at ${fmtDur(m.start)}`} onClick={() => openDetail(c.id, m.start)}>
                <span className={styles.thumb} style={stripStyle(c, momentFrame(c, m))} aria-hidden="true" />
                <span className={styles.kind}>
                  <Icon name={KIND_ICON[m.kind]} size={14} />
                  {KIND[m.kind]}
                </span>
                <span className={styles.name}>{c.name}</span>
                <span className={styles.muted}>{fmtDay(c.date)}</span>
                <span className="mono">{fmtDur(m.start)}</span>
                <span className={styles.muted} title={m.detail || undefined}>
                  {SOURCE[m.source]}
                </span>
                <span className={["mono", styles.score].join(" ")}>{Math.round(m.score * 100)} %</span>
              </button>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
