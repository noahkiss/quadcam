import { useState, type ReactNode } from "react";
import { useStoredFlag } from "../../lib/stored";
import styles from "./Disclosure.module.css";

/** A closed part of a page: a triangle and a summary ("5 passed", "History") with an optional
 *  count, which opens the rest. Rows with nothing to do and rarely used detail go here. With
 *  `remember` (a storage key) the open state is kept on this Mac. */
export function Disclosure({ summary, count, defaultOpen = false, remember, children }: { summary: ReactNode; count?: number; defaultOpen?: boolean; remember?: string; children: ReactNode }) {
  const [kept, setKept] = useStoredFlag(remember ?? "quadcam.disclosure", defaultOpen);
  const [own, setOwn] = useState(defaultOpen);
  const open = remember ? kept : own;
  const setOpen = remember ? setKept : setOwn;
  return (
    <details
      className={styles.disclosure}
      open={open}
      data-component="disclosure"
      onToggle={(e) => {
        const now = (e.currentTarget as HTMLDetailsElement).open;
        if (now !== open) setOpen(now);
      }}
    >
      <summary className={styles.summary}>
        <span className={styles.triangle} aria-hidden="true" />
        <span>{summary}</span>
        {count != null && <span className={styles.count}>{count}</span>}
      </summary>
      {open && <div className={styles.body}>{children}</div>}
    </details>
  );
}
