import type { ReactNode } from "react";
import styles from "./SummaryLine.module.css";

export interface SummaryPart {
  text: ReactNode;
  /** Colours this part: a count that needs action. */
  tone?: "ok" | "warn" | "error";
}

/** A page's counts as one muted line, parts joined by "·": "4 plugged in · 1 needs attention".
 *  It takes the place of a row of stat tiles in a `PageHeader`'s subtitle. Empty parts drop. */
export function SummaryLine({ parts }: { parts: (SummaryPart | string | null | false | undefined)[] }) {
  const shown = parts.filter((p): p is SummaryPart | string => !!p).map((p) => (typeof p === "string" ? { text: p } : p));
  if (!shown.length) return null;
  return (
    <span className={styles.line} data-component="summary-line">
      {shown.map((p, i) => (
        <span key={i} className={p.tone ? styles[p.tone] : undefined}>
          {p.text}
        </span>
      ))}
    </span>
  );
}
