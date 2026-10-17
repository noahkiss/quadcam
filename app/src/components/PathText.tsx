import type { ClipboardEvent } from "react";
import { useStore } from "../store";
import { splitPath } from "../lib/format";
import styles from "./PathText.module.css";

/** A file path on one line, in the mono face, with `~` for the home folder. When it does not
 *  fit, the middle gives way to an ellipsis and the last segment stays, as in Finder. The
 *  title holds the whole path, and a copy from inside it copies the whole path;
 *  `selectable={false}` inside a button. */
export function PathText({ path, className, selectable = true }: { path: string | null | undefined; className?: string; selectable?: boolean }) {
  const home = useStore((s) => s.home);
  const { head, tail } = splitPath(path, home);
  // Head and tail are two boxes, which would read and copy as two words: assistive tech reads
  // the path from `spoken` (generated text, outside textContent and a selection).
  const copy = (e: ClipboardEvent<HTMLSpanElement>) => {
    const sel = getSelection();
    if (!path || !sel || sel.isCollapsed || !e.currentTarget.contains(sel.anchorNode) || !e.currentTarget.contains(sel.focusNode)) return;
    e.preventDefault();
    e.clipboardData.setData("text/plain", path);
  };
  return (
    <span className={[styles.path, "mono", selectable && "selectable", className].filter(Boolean).join(" ")} title={path || undefined} onCopy={copy} data-component="path">
      <span className={styles.spoken} data-path={head + tail} />
      <span className={styles.shown} aria-hidden="true">
        {head && <span className={styles.head}>{head}</span>}
        <span className={styles.tail}>{tail}</span>
      </span>
    </span>
  );
}
