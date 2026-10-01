import { useEffect, useEffectEvent, useId, useRef, type ReactNode } from "react";
import styles from "./Dialog.module.css";

interface Props {
  open: boolean;
  title: ReactNode;
  /** Called when the dialog closes by Escape or a `close` button; `value` is the button's. */
  onClose: (value: string) => void;
  children?: ReactNode;
  /** The buttons. A `<button value="x">` closes the dialog with "x". */
  actions?: ReactNode;
  /** A sheet is the large panel over the window (Import, Settings). */
  kind?: "dialog" | "sheet";
  /** Keep Return from submitting (the erase confirmation). */
  blockReturn?: boolean;
  /** Keep Escape from closing, for example while a step runs. */
  blockEscape?: boolean;
  className?: string;
  /** Extra content in the header row, after the title. */
  header?: ReactNode;
}

/** A modal dialog on the native `<dialog>`: focus stays inside, Escape closes, the page
 * behind is inert. The heading names it for assistive tech. */
export function Dialog({ open, title, onClose, children, actions, kind = "dialog", blockReturn, blockEscape, className, header }: Props) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const close = useEffectEvent((value: string) => onClose(value));

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      d.returnValue = "";
      d.showModal();
    } else if (!open && d.open) d.close();
  }, [open]);

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    const onCancel = (e: Event) => {
      e.preventDefault();
      if (!blockEscape) close("cancel");
    };
    d.addEventListener("cancel", onCancel);
    return () => d.removeEventListener("cancel", onCancel);
  }, [blockEscape]);

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      className={[styles.dialog, kind === "sheet" ? styles.sheet : styles.small, className].filter(Boolean).join(" ")}
      onKeyDown={(e) => {
        if (blockReturn && e.key === "Enter") e.preventDefault();
      }}
    >
      {open && (
        <form
          method="dialog"
          className={styles.form}
          onSubmit={(e) => {
            e.preventDefault();
            const v = (e.nativeEvent as SubmitEvent).submitter as HTMLButtonElement | null;
            onClose(v?.value || "");
          }}
        >
          <header className={styles.head}>
            <h2 id={titleId} className={styles.title}>
              {title}
            </h2>
            {header}
          </header>
          <div className={styles.body}>{children}</div>
          {actions && <menu className={styles.actions}>{actions}</menu>}
        </form>
      )}
    </dialog>
  );
}
