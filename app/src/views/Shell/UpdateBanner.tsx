import { useStore } from "../../store";
import { updateBanner } from "../../store/update";
import { Banner } from "../../components/Banner";
import { Button } from "../../components/Button";
import { Link } from "../../components/ModulePrompt";
import { ProgressRing } from "../../components/ProgressRing";
import { restartToUpdate } from "../../actions/update";

/** A newer QuadCam: Restart to update downloads it, then restarts once no job runs. */
export function UpdateBanner() {
  const u = useStore(updateBanner);
  const later = useStore((s) => s.laterUpdate);
  if (!u) return null;
  const busy = u.phase === "downloading";
  const waiting = u.phase === "ready" && !!u.waiting_for;
  const pct = u.total ? u.done / u.total : 0;
  return (
    <Banner
      icon={busy ? undefined : "restart"}
      tint="blue"
      action={
        <>
          {busy && <ProgressRing value={pct} size={20} label="Download" />}
          <Button size="sm" variant="ghost" onClick={later} disabled={busy}>
            Later
          </Button>
          <Button size="sm" variant="primary" onClick={() => void restartToUpdate()} disabled={busy || waiting}>
            {u.phase === "failed" ? "Try again" : "Restart to update"}
          </Button>
        </>
      }
    >
      <span role="status">
        {u.phase === "failed" ? (
          <>{u.error}</>
        ) : busy ? (
          <>Downloading QuadCam {u.version}… {u.total ? `${Math.round(pct * 100)}%` : ""}</>
        ) : waiting ? (
          <>
            <b>QuadCam {u.version} is ready.</b> It can restart after {u.waiting_for} ends.
          </>
        ) : (
          <>
            <b>QuadCam {u.version} is available.</b>
          </>
        )}{" "}
        {u.notes_url && <Link href={u.notes_url}>Release notes</Link>}
      </span>
    </Banner>
  );
}
