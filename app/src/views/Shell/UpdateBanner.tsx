import { useStore } from "../../store";
import { updateBanner, updateProgress } from "../../store/update";
import { Banner } from "../../components/Banner";
import { Button } from "../../components/Button";
import { Link } from "../../components/ModulePrompt";
import { RestartToUpdate } from "./RestartToUpdate";

/** A newer QuadCam: Restart to update downloads it, then restarts once no job runs. */
export function UpdateBanner() {
  const u = useStore(updateBanner);
  const later = useStore((s) => s.laterUpdate);
  if (!u) return null;
  const { busy, waiting, percent } = updateProgress(u);
  return (
    <Banner
      icon={busy ? undefined : "restart"}
      tint="blue"
      action={
        <RestartToUpdate u={u}>
          <Button size="sm" variant="ghost" onClick={later} disabled={busy}>
            Later
          </Button>
        </RestartToUpdate>
      }
    >
      <span role="status">
        {u.phase === "failed" ? (
          <>{u.error}</>
        ) : busy ? (
          <>Downloading QuadCam {u.version}… {percent}</>
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
