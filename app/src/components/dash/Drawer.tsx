import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import styles from "./Drawer.module.css";

/** A block that shows only its top `closedHeight` (px) until opened, like a drawer pulled
 *  out: a fade marks the cut, and a button below opens or closes it. Content that fits shows
 *  whole, with no button. Focus moving into the hidden part opens it, so a keyboard never
 *  lands on something out of sight. */
export function Drawer({ open, onToggle, closedHeight, more, less = "Show less", children }: { open: boolean; onToggle: (open: boolean) => void; closedHeight: number; more: string; less?: string; children: ReactNode }) {
  const id = useId();
  const inner = useRef<HTMLDivElement>(null);
  const [tall, setTall] = useState(false);
  useEffect(() => {
    const el = inner.current;
    if (!el) return;
    const measure = () => setTall(el.scrollHeight > closedHeight + 24);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    for (const c of Array.from(el.children)) ro.observe(c);
    return () => ro.disconnect();
  }, [closedHeight, children]);
  const shut = tall && !open;
  return (
    <div className={styles.drawer} data-component="drawer">
      <div id={id} ref={inner} className={shut ? styles.closed : styles.open} style={shut ? { maxHeight: closedHeight } : undefined} onFocus={shut ? () => onToggle(true) : undefined}>
        {children}
      </div>
      {tall && (
        <button type="button" className={styles.handle} aria-expanded={open} aria-controls={id} onClick={() => onToggle(!open)}>
          <span className={styles.chevron} aria-hidden="true" />
          {open ? less : more}
        </button>
      )}
    </div>
  );
}
