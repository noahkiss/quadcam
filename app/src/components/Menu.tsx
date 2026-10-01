import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { Icon, type IconName } from "./Icon";
import styles from "./Menu.module.css";

export interface MenuItem {
  label: string;
  icon?: IconName;
  /** The shortcut shown at the right. */
  keys?: string;
  danger?: boolean;
  disabled?: boolean;
  run: () => void;
}

interface Props {
  label: string;
  /** Where it opens, in window coordinates. */
  at: { x: number; y: number };
  /** `null` draws a separator. */
  items: (MenuItem | null)[];
  /** `refocus` is true when the menu closed from the keyboard or an item. */
  onClose: (refocus: boolean) => void;
}

/** A context menu: arrows, Home and End move; Return runs; Escape and Tab close. */
export function Menu({ label, at, items, onClose }: Props) {
  const ref = useRef<HTMLUListElement>(null);
  const [pos, setPos] = useState(at);

  useLayoutEffect(() => {
    const r = ref.current!.getBoundingClientRect();
    setPos({ x: Math.max(8, Math.min(at.x, innerWidth - r.width - 8)), y: Math.max(8, Math.min(at.y, innerHeight - r.height - 8)) });
    ref.current!.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [at]);

  useEffect(() => {
    const away = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose(false);
    };
    document.addEventListener("mousedown", away, true);
    return () => document.removeEventListener("mousedown", away, true);
  }, [onClose]);

  const onKeyDown = (e: KeyboardEvent) => {
    const buttons = [...ref.current!.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const go = ({ ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: buttons.length - 1 } as Record<string, number>)[e.key];
    if (go != null) buttons[(go + buttons.length) % buttons.length]?.focus();
    else if (e.key === "Escape" || e.key === "Tab") onClose(true);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <ul ref={ref} role="menu" aria-label={label} className={styles.menu} style={{ left: pos.x, top: pos.y }} onKeyDown={onKeyDown} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) =>
        it ? (
          <li key={i} role="none">
            <button
              type="button"
              role="menuitem"
              disabled={it.disabled}
              className={it.danger ? styles.danger : undefined}
              onClick={() => {
                onClose(true);
                it.run();
              }}
            >
              {it.icon ? <Icon name={it.icon} size={15} /> : <span className={styles.noIcon} />}
              <span className={styles.label}>{it.label}</span>
              <span className={styles.keys} aria-hidden="true">
                {it.keys}
              </span>
            </button>
          </li>
        ) : (
          <li key={i} role="separator" className={styles.sep} />
        ),
      )}
    </ul>
  );
}
