import type { ReactNode } from "react";
import styles from "./FactList.module.css";

export interface Fact {
  label: string;
  value: ReactNode;
  /** The value can be selected and copied (an id, a path, a version). */
  selectable?: boolean;
}

/** Facts about one object as label and value rows; two columns of pairs once the box is
 *  720 px wide. `label` names the list. */
export function FactList({ facts, label }: { facts: Fact[]; label?: string }) {
  return (
    <div className={styles.wrap}>
      <dl className={styles.list} aria-label={label} data-component="fact-list">
        {facts.map((f) => (
          <div key={f.label}>
            <dt>{f.label}</dt>
            <dd className={f.selectable ? "selectable" : undefined}>{f.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
