import { useStore } from "../../store";
import { clipsOf, planOf, resultOf } from "../../store/session";
import { sel } from "../../store/settings";
import { Icon } from "../../components/Icon";
import { base, fmtBytes, tilde } from "../../lib/format";
import styles from "./Export.module.css";

/** Add to Library: each clip as it converts and verifies. */
export function Export() {
  const s = useStore((x) => x.session);
  const progress = useStore((x) => x.progress);
  const total = useStore((x) => x.exportTotal);
  const done = useStore((x) => x.exportDone);
  const out = useStore(sel.outputDir);
  const format = useStore(sel.format);
  const home = useStore((x) => x.home);
  const defaultName = useStore(sel.defaultName);
  if (!s) return null;
  const running = Object.values(progress)[0] || 0;
  return (
    <div className={styles.export}>
      <div className={styles.head}>
        <h3>
          Adding {total} clip{total === 1 ? "" : "s"} to the library
        </h3>
        <progress max={1} value={total ? Math.min(1, (done + running) / total) : 1} aria-label="Adding to the library" />
        <p className="selectable">
          To {tilde(out, home)} · {format.toUpperCase()}
        </p>
      </div>
      <ol className={styles.files}>
        {clipsOf(s)
          .filter((c) => !planOf(s, c.id)?.skip)
          .map((c) => {
            const p = planOf(s, c.id)!;
            const r = resultOf(s, c.id);
            const prog = progress[c.id];
            return (
              <li key={c.id}>
                <b>{p.name || defaultName}</b>
                <span className={styles.file}>
                  {r ? <Icon name={r.outcome === "verified" ? "check-circle" : "close-circle"} tint={r.outcome === "verified" ? "green" : "red"} /> : prog != null ? <Icon name="refresh-moments" tint="blue" /> : <Icon name="clock" tint="muted" />}
                  <span className="mono selectable">{r?.output ? base(r.output) : c.name}</span>
                  {prog != null && !r ? <progress max={1} value={prog} aria-label={`Converting ${c.name}`} /> : <span className={styles.muted}>{r?.size ? fmtBytes(r.size) : r?.error || ""}</span>}
                </span>
              </li>
            );
          })}
      </ol>
    </div>
  );
}
