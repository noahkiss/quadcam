import { useId, type ReactNode } from "react";
import styles from "./SectionCard.module.css";

/** A titled block of a page: a header row (title, optional status, actions on the right)
 *  and a body. The card is a region named by its title, or by `label` when given. */
export function SectionCard({ title, label, status, actions, children }: { title: ReactNode; label?: string; status?: ReactNode; actions?: ReactNode; children?: ReactNode }) {
  const id = useId();
  return (
    <section className={styles.card} aria-labelledby={label ? undefined : id} aria-label={label} data-component="section-card">
      <header className={styles.head}>
        <h3 id={id} className={styles.title}>
          {title}
        </h3>
        {status && <div className={styles.status}>{status}</div>}
        {actions && <div className={styles.actions}>{actions}</div>}
      </header>
      {children}
    </section>
  );
}
