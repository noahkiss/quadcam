import type { ReactNode } from "react";
import styles from "./StatRow.module.css";

export interface StatProps {
  label: string;
  value: ReactNode;
  /** A muted line under the value. */
  sub?: ReactNode;
  /** Colours the value; the tile background stays plain. */
  tone?: "ok" | "warn" | "error";
  /** 0 to 1: a thin meter under the value (credits left, space used). */
  meter?: number;
  /** A mark beside the label, such as a promo `Badge`. */
  badge?: ReactNode;
  /** Makes the tile a button that opens what it sums up. */
  onClick?: () => void;
}

/** One summary tile: a label, a large mono value, an optional sub-line and meter. */
export function Stat({ label, value, sub, tone, meter, badge, onClick }: StatProps) {
  const body = (
    <>
      <span className={styles.label}>
        {label}
        {badge}
      </span>
      <b className={[styles.value, tone && styles[tone]].filter(Boolean).join(" ")}>{value}</b>
      {meter !== undefined && (
        <span className={styles.meter} role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(Math.min(1, Math.max(0, meter)) * 100)}>
          <span className={[styles.fill, tone && styles[tone]].filter(Boolean).join(" ")} style={{ width: `${Math.min(1, Math.max(0, meter)) * 100}%` }} />
        </span>
      )}
      {sub != null && sub !== "" && <span className={styles.sub}>{sub}</span>}
    </>
  );
  return onClick ? (
    <button type="button" className={[styles.stat, styles.button].join(" ")} onClick={onClick} data-component="stat">
      {body}
    </button>
  ) : (
    <div className={styles.stat} data-component="stat">
      {body}
    </div>
  );
}

/** A row of `Stat` tiles that wraps on a narrow window. `label` names the group. */
export function StatRow({ label, children, role }: { label: string; children: ReactNode; role?: "status" }) {
  return (
    <div className={styles.row} role={role ?? "group"} aria-label={label} data-component="stat-row">
      {children}
    </div>
  );
}
