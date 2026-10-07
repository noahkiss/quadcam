import { useEffect } from "react";
import { useStore } from "../../store";
import { clipsOf } from "../../store/session";
import { Button } from "../../components/Button";
import { Chip } from "../../components/Chip";
import { Dialog } from "../../components/Dialog";
import { ProgressRing } from "../../components/ProgressRing";
import { Stepper } from "../../components/Stepper";
import { base, fmtBytes, SOURCE_LABEL } from "../../lib/format";
import { doneImport, runExport, setStep, startOver, stepReview, toggleSkip } from "../../actions/session";
import { screenKeys } from "../../keys";
import { Review, reviewTrim } from "./Review";
import { Export } from "./Export";
import { Finish } from "./Finish";
import styles from "./ImportSheet.module.css";

const STEPS = [
  { id: "load" as const, label: "Load" },
  { id: "review" as const, label: "Review" },
  { id: "export" as const, label: "Add to Library" },
  { id: "finish" as const, label: "Finish" },
];

/** Import: a sheet over the library with the steps Load, Review, Add to Library, Finish. */
export function ImportSheet() {
  const open = useStore((s) => s.importOpen);
  const step = useStore((s) => s.step);
  const busy = useStore((s) => s.busy);
  const session = useStore((s) => s.session);
  const setOpen = useStore((s) => s.setImportOpen);
  const staging = useStore((s) => s.staging);
  const hasCard = !!session?.card;

  // Keys go to the clip list, not to the first button in the sheet.
  useEffect(() => {
    if (open) requestAnimationFrame(() => (document.querySelector('dialog[open] [role="grid"][aria-label="Clips"]') as HTMLElement | null)?.focus());
  }, [open]);

  useEffect(() => {
    screenKeys.sheet = sheetKeys;
    return () => {
      screenKeys.sheet = undefined;
    };
  }, []);

  const source = session ? session.card_volume?.info.volume_name || base(session.source) : null;
  const frac = staging?.total ? (staging.index + (staging.size ? staging.done / staging.size : 0)) / staging.total : 0;
  const left = staging ? Math.max(0, staging.total - staging.index) : 0;
  return (
    <Dialog
      open={open}
      kind="sheet"
      title="Import"
      blockEscape={busy}
      onClose={() => setOpen(false)}
      header={
        <>
          {session && source && (
            <Chip icon={hasCard ? "sd-card" : "folder-open"} tint="green">
              {source}
              <span className={styles.muted}>
                {" "}
                · {SOURCE_LABEL[session.kind]} {hasCard ? "card" : "folder"} · {clipsOf(session).length} clips
              </span>
            </Chip>
          )}
          <Stepper label="Import steps" steps={STEPS} current={step} />
          <span className={styles.grow} />
          {step === "load" && staging && (
            <span className={styles.load}>
              <ProgressRing value={frac} label={staging.phase === "stage" ? "Copying" : "Checking"} />
              <span>
                <b>{staging.phase === "stage" ? (staging.total ? `Copying · ${left} left` : "Copying…") : `Checking · ${left} left`}</b>
                <span className={`${styles.muted} mono`}>{staging.phase === "stage" ? (staging.size ? `${fmtBytes(staging.done)} of ${fmtBytes(staging.size)}` : "") : "Probing and finding dead air"}</span>
              </span>
            </span>
          )}
          {session && step !== "load" && step !== "export" && (
            <Button size="sm" variant="ghost" title="Forgets the loaded clips. Imported files stay." onClick={startOver}>
              Start over
            </Button>
          )}
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => setOpen(false)}>
            Close
          </Button>
        </>
      }
      actions={
        step === "finish" || step === "export" ? (
          <>
            {step === "finish" && (
              <Button variant="ghost" size="sm" icon="arrow-left" className={styles.back} onClick={() => setStep("review")}>
                Back to review
              </Button>
            )}
            <span className={styles.footNote}>{step === "finish" && hasCard ? "The card stays mounted until you make it safe to remove." : ""}</span>
            {step === "finish" && (
              <Button variant="primary" onClick={doneImport}>
                Done · show in Library
              </Button>
            )}
          </>
        ) : undefined
      }
    >
      {open && (step === "review" || step === "load") && <Review />}
      {open && step === "export" && <Export />}
      {open && step === "finish" && <Finish />}
    </Dialog>
  );
}

/** The sheet's keys: Command-Return adds to the library, even from a field; in the review,
 * Up and Down move, S skips, Space plays, and the trim editor's keys. */
function sheetKeys(e: KeyboardEvent): boolean {
  const s = useStore.getState();
  const otherDialog = !!(s.askReq || s.removedReq || s.formatConfirm || s.settingsOpen);
  if (e.metaKey && e.key === "Enter" && s.step === "review" && !otherDialog) {
    if (s.env?.tools && !s.busy && s.session) runExport();
    return true;
  }
  const t = e.target as Element | null;
  if (otherDialog || t?.closest?.("input, select, textarea, [contenteditable]")) return false;
  if (s.step !== "review") return false;
  const plain = !e.metaKey && !e.ctrlKey && !e.altKey;
  if (plain && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
    stepReview(e.key === "ArrowUp" ? -1 : 1);
    return true;
  }
  if (plain && e.key.toLowerCase() === "s") {
    toggleSkip(s.selectedClip);
    return true;
  }
  return reviewTrim.handleKey?.(e, !!t?.closest?.("button")) ?? false;
}

