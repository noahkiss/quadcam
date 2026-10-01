import { useStore } from "../../store";
import { libClip } from "../../store/library";
import { Button } from "../../components/Button";

/** The open clip. Built out in plan step U5. */
export function ClipDetail() {
  const c = useStore((s) => libClip(s, s.detailId));
  const close = useStore((s) => s.closeDetail);
  if (!c) return null;
  return (
    <section aria-label="Clip" style={{ padding: "var(--s-4)" }}>
      <Button variant="ghost" icon="arrow-left" onClick={close}>
        Library
      </Button>
      <h2>{c.name}</h2>
    </section>
  );
}
