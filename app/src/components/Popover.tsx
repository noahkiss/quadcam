import { useEffect, useRef, type ReactNode, type RefObject } from "react";
import styles from "./Popover.module.css";

interface Props {
  /** Names the panel for assistive tech. */
  label: string;
  /** The button that opens it. A press on it is the button's to handle, not an outside click. */
  anchor: RefObject<HTMLElement | null>;
  onClose: () => void;
  children: ReactNode;
}

/** A panel under its button. A press outside it closes it, as does Escape, which returns focus
 *  to the button. */
export function Popover({ label, anchor, onClose, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const away = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor.current?.contains(t)) onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onClose();
      anchor.current?.focus();
    };
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("keydown", key, true);
    };
  }, [anchor, onClose]);

  return (
    <div ref={ref} role="dialog" aria-label={label} className={styles.popover}>
      {children}
    </div>
  );
}
