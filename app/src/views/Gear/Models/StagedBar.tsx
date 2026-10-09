import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { useStore } from "../../../store";

/** The title of the change the model editors keep their edits in (`core/model_edit.rs`). */
export const MODEL_CHANGE = "Model edits";

/** The radio's staged model edits, with Review and Undo. */
export function StagedBar({ device }: { device: string | null }) {
  const change = useStore((s) => s.changes.find((c) => c.device === device && c.title === MODEL_CHANGE && (c.status === "draft" || c.status === "ready")));
  const open = useStore((s) => s.openApply);
  const discard = useStore((s) => s.discardChange);
  if (!device || !change) return null;
  const n = change.edits.reduce((k, e) => k + (e.kind === "model" ? e.ops.length : e.kind === "checklist" ? 1 : 0), 0);
  return (
    <Banner
      kind="info"
      icon="info"
      tint="blue"
      action={
        <>
          <Button size="sm" onClick={() => open(device, change.id)}>
            Review…
          </Button>
          <Button size="sm" variant="danger-ghost" onClick={() => discard(change.id)}>
            Undo model edits
          </Button>
        </>
      }
    >
      {n === 1 ? "1 model edit is staged." : `${n} model edits are staged.`} The radio changes when you apply them.
    </Banner>
  );
}
