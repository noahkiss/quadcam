import { useEffect, useRef, type MouseEvent } from "react";
import { store, useStore } from "../../store";
import type { LibClip } from "../../ipc/types";
import { renameClip } from "../../actions/library";
import styles from "./NameEdit.module.css";

/** The clip name as a text field: Return or leaving saves, Escape cancels. */
export function NameEdit({ clip }: { clip: LibClip }) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  const setRenaming = useStore((s) => s.setRenaming);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const finish = async (keep: boolean) => {
    if (done.current) return;
    done.current = true;
    const value = ref.current?.value ?? "";
    setRenaming(null);
    if (keep) await renameClip(clip.id, value);
    requestAnimationFrame(() => (document.querySelector(`[data-id="${CSS.escape(clip.id)}"]`) as HTMLElement | null)?.focus());
  };
  const stop = (e: MouseEvent) => e.stopPropagation();
  return (
    <input
      ref={ref}
      className={styles.edit}
      type="text"
      defaultValue={clip.title || clip.name}
      aria-label="Clip name"
      spellCheck={false}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          finish(true);
        }
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      onClick={stop}
      onDoubleClick={stop}
      onMouseDown={stop}
    />
  );
}

let renameTimer: ReturnType<typeof setTimeout> | undefined;

/** A click on the name of the one selected clip renames it, unless a double-click follows. */
export function useNameClick(id: string) {
  return {
    onClick: (e: MouseEvent) => {
      const s = store.getState();
      if (s.selected.size !== 1 || !s.selected.has(id) || e.metaKey || e.shiftKey || s.renaming) return;
      e.stopPropagation();
      clearTimeout(renameTimer);
      renameTimer = setTimeout(() => store.getState().setRenaming(id), 450);
    },
    cancel: () => clearTimeout(renameTimer),
  };
}
