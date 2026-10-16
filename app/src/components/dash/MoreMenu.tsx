import { useRef, useState } from "react";
import { Button } from "../Button";
import { Menu, type MenuItem } from "../Menu";

/** A `…` button that opens a menu under it: the place for a page's rare and destructive
 *  actions (Forget…), so the header keeps one row of plain buttons. With `text` it is a
 *  pop-up button with that text instead ("Open…"), named by its text. */
export function MoreMenu({ items, label = "More actions", text }: { items: (MenuItem | null)[]; label?: string; text?: string }) {
  const ref = useRef<HTMLButtonElement>(null);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  // A click on the button while the menu is open closes it on mousedown; the click that
  // follows must not open it again.
  const closedAt = useRef(0);
  const open = () => {
    if (performance.now() - closedAt.current < 300) return;
    const r = ref.current!.getBoundingClientRect();
    setAt({ x: r.left, y: r.bottom + 4 });
  };
  return (
    <>
      {text ? (
        <Button ref={ref} iconEnd="chev-down" aria-haspopup="menu" aria-expanded={at !== null} onClick={open}>
          {text}
        </Button>
      ) : (
        <Button ref={ref} size="sm" variant="ghost" icon="dots" aria-label={label} aria-haspopup="menu" aria-expanded={at !== null} onClick={open} />
      )}
      {at && (
        <Menu
          label={label}
          at={at}
          items={items}
          onClose={(refocus) => {
            setAt(null);
            closedAt.current = refocus ? 0 : performance.now();
            if (refocus) ref.current?.focus();
          }}
        />
      )}
    </>
  );
}
