// Dialogs and floating things that any screen can raise.
import { useEffect, useRef, useState } from "react";
import { useStore } from "../../store";
import { libClip, selectedIds } from "../../store/library";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Icon } from "../../components/Icon";
import { Menu, type MenuItem } from "../../components/Menu";
import { Toast } from "../../components/Toast";
import { base } from "../../lib/format";
import { albumAction } from "../../lib/library";
import { addLibToPhotos, rescan, revealClip, shareClips, trashClips } from "../../actions/library";
import { cancelErase, confirmErase } from "../../actions/session";
import { api, errText } from "../../ipc/api";
import styles from "./Overlays.module.css";

export function Overlays() {
  return (
    <>
      <AskDialog />
      <RemovedCutsDialog />
      <FormatConfirm />
      <NoticesDialog />
      <ClipMenu />
      <DropTarget />
      <Toast />
    </>
  );
}

function AskDialog() {
  const req = useStore((s) => s.askReq);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (req?.input != null) requestAnimationFrame(() => input.current?.select());
  }, [req]);
  return (
    <Dialog
      open={!!req}
      title={req?.title}
      onClose={(v) => req?.resolve(v === "ok" ? (req.input == null ? true : input.current?.value ?? "") : null)}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="ok" variant={req?.danger ? "danger" : "primary"} autoFocus={req?.input == null}>
            {req?.ok || "OK"}
          </Button>
        </>
      }
    >
      {req?.text && <p className={styles.askText}>{req.text}</p>}
      {req?.body}
      {req?.input != null && <input ref={input} className={styles.askInput} type="text" defaultValue={req.input} aria-label={req.title} spellCheck={false} />}
    </Dialog>
  );
}

/** Acknowledgements: the third-party notices the app ships. */
function NoticesDialog() {
  const open = useStore((s) => s.noticesOpen);
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    let live = true;
    api.thirdPartyNotices().then(
      (t) => live && setText(t),
      (e) => live && setText(errText(e)),
    );
    return () => {
      live = false;
    };
  }, [open]);
  return (
    <Dialog
      open={open}
      kind="sheet"
      className={styles.notices}
      title="Acknowledgements"
      onClose={() => {
        useStore.getState().setNoticesOpen(false);
        setText(null);
      }}
      actions={
        <Button type="submit" value="done" variant="primary">
          Done
        </Button>
      }
    >
      <pre className={`${styles.noticesText} mono selectable`} tabIndex={0} aria-label="Third-party notices">
        {text ?? "Loading…"}
      </pre>
    </Dialog>
  );
}

function RemovedCutsDialog() {
  const req = useStore((s) => s.removedReq);
  const files = req?.files || [];
  return (
    <Dialog
      open={!!req}
      title={
        <>
          <Icon name="scissors" />
          Remove the cut?
        </>
      }
      onClose={(v) => req?.resolve(v === "keep" || v === "trash" ? v : null)}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost" autoFocus>
            Cancel
          </Button>
          <Button type="submit" value="keep">
            Keep the file
          </Button>
          <Button type="submit" value="trash" variant="danger">
            Move to Trash
          </Button>
        </>
      }
    >
      <p>{files.length === 1 ? "This cut is already a file." : `These ${files.length} cuts are already files.`}</p>
      <ul className={styles.files}>
        {files.map((f) => (
          <li key={f} className="mono">
            {base(f)}
          </li>
        ))}
      </ul>
    </Dialog>
  );
}

function FormatConfirm() {
  const c = useStore((s) => s.formatConfirm);
  return (
    <Dialog
      open={!!c}
      blockReturn
      title={
        <>
          <Icon name="danger-triangle" tint="red" />
          Erase the card?
        </>
      }
      onClose={() => cancelErase()}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost" autoFocus>
            Cancel
          </Button>
          <Button variant="danger" onClick={() => confirmErase()}>
            Erase
          </Button>
        </>
      }
    >
      {c?.agent && <p className={styles.agent}>An agent asked to erase this card. It goes ahead only if you click Erase.</p>}
      <p>{c?.text}</p>
      <p className={styles.muted}>The card is ejected right after the erase.</p>
    </Dialog>
  );
}

/** The clip's context menu; items that do not apply are left out. */
function ClipMenu() {
  const at = useStore((s) => s.menuAt);
  const setMenu = useStore((s) => s.setMenu);
  const s = useStore();
  if (!at) return null;
  const ids = selectedIds(s);
  const many = ids.length > 1;
  const id = at.id;
  const album = albumAction(sel.photosAlbum(s), ids.length);
  const anchor = () => document.querySelector(`[data-id="${CSS.escape(id)}"]`);
  const items: (MenuItem | null)[] = [
    ...(many
      ? []
      : [
          { label: "Rename", icon: "pen" as const, keys: "⏎", run: () => s.setRenaming(id) },
          { label: "Edit details", icon: "tag" as const, keys: "⌘I", run: () => s.openDetail(id, null, "details") },
          { label: "Trim and cuts", icon: "scissors" as const, keys: "T", run: () => s.openDetail(id) },
          null,
        ]),
    { label: "Share…", icon: "share", run: () => shareClips(ids, anchor()) },
    ...(album ? [{ label: album, icon: "photos" as const, run: () => addLibToPhotos(ids) }] : []),
    ...(many || !libClip(s, id)
      ? []
      : [
          { label: "Show in Finder", icon: "finder" as const, keys: "⌘R", run: () => revealClip(id) },
          { label: "Find dead air again", icon: "refresh-moments" as const, run: () => rescan(id) },
        ]),
    null,
    { label: many ? `Move ${ids.length} to Trash` : "Move to Trash", icon: "trash-bin-trash", keys: "⌘⌫", danger: true, run: () => trashClips(ids) },
  ];
  return (
    <Menu
      label="Clip actions"
      at={at}
      items={items}
      onClose={(refocus) => {
        setMenu(null);
        if (refocus) requestAnimationFrame(() => (document.querySelector(`[data-id="${CSS.escape(id)}"]`) as HTMLElement | null)?.focus());
      }}
    />
  );
}

function DropTarget() {
  const on = useStore((s) => s.dropping);
  if (!on) return null;
  return (
    <div className={styles.drop} aria-hidden="true">
      <div>
        <Icon name="import" size={32} />
        <b>Import</b>
        <span>A folder, or AVI clips</span>
      </div>
    </div>
  );
}
