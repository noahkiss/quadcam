import type { ReactNode } from "react";
import type { UpdateStatus } from "../../ipc/types";
import { updateProgress } from "../../store/update";
import { Button } from "../../components/Button";
import { ProgressRing } from "../../components/ProgressRing";
import { restartToUpdate } from "../../actions/update";

/** The download's ring and Restart to update (Try again after a failure), for the banner and
 * Settings. `children` sit between the two (the banner's Later). */
export function RestartToUpdate({ u, children }: { u: UpdateStatus; children?: ReactNode }) {
  const { busy, waiting, pct } = updateProgress(u);
  return (
    <>
      {busy && <ProgressRing value={pct} size={20} label="Download" />}
      {children}
      <Button size="sm" variant="primary" onClick={() => void restartToUpdate()} disabled={busy || waiting}>
        {u.phase === "failed" ? "Try again" : "Restart to update"}
      </Button>
    </>
  );
}
