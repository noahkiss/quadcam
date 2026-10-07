import styles from "./gear.module.css";

/** `gear::model::DiffItem`: CLI lines or a text file's line diff, a card's file list, or a
 *  firmware version pair. */
export type DiffEntry =
  | { kind: "lines"; label: string; lines: { op: "same" | "add" | "remove"; text: string }[] }
  | { kind: "files"; label: string; put: string[]; delete: string[] }
  | { kind: "version"; label: string; before?: string | null; after: string };

const MARK = { same: " ", add: "+", remove: "−" } as const;
const OP_LABEL = { same: "Unchanged", add: "Added", remove: "Removed" } as const;

/** The before/after of a staged change. Diffs use the monospace face; nothing else does. */
export function DiffView({ items }: { items: DiffEntry[] }) {
  return (
    <div className={styles.diff} data-component="diff-view">
      {items.map((d, i) => (
        <section key={i} aria-label={d.label}>
          <h4>{d.label}</h4>
          {d.kind === "lines" && (
            <pre className={`${styles.lines} mono selectable`}>
              {d.lines.map((l, j) => (
                <span key={j} className={styles[l.op]} title={OP_LABEL[l.op]}>
                  {MARK[l.op]} {l.text}
                  {"\n"}
                </span>
              ))}
            </pre>
          )}
          {d.kind === "files" && (
            <ul className={`${styles.files} selectable`}>
              {d.put.map((f) => (
                <li key={`p${f}`} className={styles.add}>
                  <span className="visually-hidden">Put </span>
                  <span className="mono">{f}</span>
                </li>
              ))}
              {d.delete.map((f) => (
                <li key={`d${f}`} className={styles.remove}>
                  <span className="visually-hidden">Delete </span>
                  <span className="mono">{f}</span>
                </li>
              ))}
            </ul>
          )}
          {d.kind === "version" && (
            <p className={`${styles.version} selectable`}>
              <span className="mono">{d.before || "None"}</span> → <span className="mono">{d.after}</span>
            </p>
          )}
        </section>
      ))}
    </div>
  );
}
