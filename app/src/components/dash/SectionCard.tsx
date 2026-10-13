import { useId, type ReactNode } from "react";
import styles from "./SectionCard.module.css";

/** A titled block of a page: a header row (title, optional status, actions on the right)
 *  and a body. The card is a region named by its title, or by `label` when given. With
 *  `collapse`, the title is a disclosure button that shows or hides the body and the actions. */
export function SectionCard({
  title,
  label,
  status,
  actions,
  collapse,
  children,
}: {
  title: ReactNode;
  label?: string;
  status?: ReactNode;
  actions?: ReactNode;
  collapse?: { open: boolean; onToggle: (open: boolean) => void };
  children?: ReactNode;
}) {
  const id = useId();
  const open = collapse?.open ?? true;
  return (
    <section className={styles.card} aria-labelledby={label ? undefined : id} aria-label={label} data-component="section-card">
      <header className={styles.head}>
        <h3 id={id} className={styles.title}>
          {collapse ? (
            <button type="button" className={styles.disclosure} aria-expanded={open} aria-controls={`${id}-body`} onClick={() => collapse.onToggle(!open)}>
              <span className={styles.triangle} aria-hidden="true" />
              {title}
            </button>
          ) : (
            title
          )}
        </h3>
        {status && <div className={styles.status}>{status}</div>}
        {actions && open && <div className={styles.actions}>{actions}</div>}
      </header>
      {collapse ? (
        <div id={`${id}-body`} className={styles.body} hidden={!open}>
          {open && children}
        </div>
      ) : (
        children
      )}
    </section>
  );
}
