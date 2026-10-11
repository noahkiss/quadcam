import type { ReactNode } from "react";
import styles from "./StatusDot.module.css";

export type StatusTone = "ok" | "warn" | "error" | "idle";

/** A state as a coloured dot and its label. The label carries the meaning; the dot only
 *  repeats it. */
export function StatusDot({ tone, children }: { tone: StatusTone; children: ReactNode }) {
  return (
    <span className={styles.status} data-tone={tone} data-component="status-dot">
      <span className={[styles.dot, styles[tone]].join(" ")} aria-hidden="true" />
      {children}
    </span>
  );
}
