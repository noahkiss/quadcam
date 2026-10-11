import type { ReactNode } from "react";
import styles from "./Badge.module.css";

export type BadgeTone = "neutral" | "accent" | "ok" | "warn" | "error";

/** A short text mark beside a value or a row: a state, a promo, "For your quads". Tinted
 *  text on a faint fill of the same hue; never a coloured dot alone. */
export function Badge({ tone = "neutral", title, children }: { tone?: BadgeTone; title?: string; children: ReactNode }) {
  return (
    <span className={[styles.badge, styles[tone]].join(" ")} title={title} data-component="badge">
      {children}
    </span>
  );
}
