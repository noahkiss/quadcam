import { useEffect, useId, useRef, type ReactNode } from "react";
import { Button } from "../Button";
import styles from "./DetailDrawer.module.css";

/** A page with a detail drawer: the page's own content on the left and, while `drawer` is
 *  set, the drawer on the right, so the table stays in view and narrows. In a narrow content
 *  column the drawer is a full-width sheet over the page instead. */
export function WithDrawer({ drawer, children }: { drawer?: ReactNode; children: ReactNode }) {
  return (
    <div className={styles.split} data-drawer={drawer ? "open" : undefined}>
      <div className={styles.main}>{children}</div>
      {drawer}
    </div>
  );
}

/** The drawer itself: a title, a Close button, and the selected row's detail. Escape closes
 *  it. As a sheet (narrow) it takes focus when it opens and gives it back when it closes. */
export function DetailDrawer({ title, subtitle, actions, onClose, children }: { title: ReactNode; subtitle?: ReactNode; actions?: ReactNode; onClose: () => void; children: ReactNode }) {
  const id = useId();
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const back = document.activeElement as HTMLElement | null;
    const sheet = getComputedStyle(el).position === "absolute";
    if (sheet) el.querySelector<HTMLButtonElement>("[data-action=close-drawer]")?.focus();
    return () => {
      if (sheet && back?.isConnected) back.focus();
    };
  }, []);
  return (
    <aside
      ref={ref}
      className={styles.drawer}
      aria-labelledby={id}
      data-component="detail-drawer"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <header className={styles.head}>
        <div className={styles.text}>
          <h3 id={id} className={styles.title}>
            {title}
          </h3>
          {subtitle != null && subtitle !== "" && <span className={styles.sub}>{subtitle}</span>}
        </div>
        {actions}
        <Button size="sm" variant="ghost" icon="close" aria-label="Close" title="Close" data-action="close-drawer" onClick={onClose} />
      </header>
      <div className={styles.body}>{children}</div>
    </aside>
  );
}
