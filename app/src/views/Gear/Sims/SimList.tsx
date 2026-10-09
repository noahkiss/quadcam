// The list of sims on this Mac against one quad profile: each sim's state, its files and
// profiles with a Sync button, and Restore backup for a sim QuadCam backed up. Used by the
// Rates segment and the Sims page.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Chip } from "../../../components/Chip";
import { api } from "../../../ipc/api";
import type { BackupSummary, RateProfile, SimRates } from "../../../ipc/types";
import { fmtWhen } from "../../../lib/backups";
import { profileLabel, simState, worstDiff } from "../../../lib/rates";
import { useStore } from "../../../store";
import styles from "../Rates/Rates.module.css";

export function SimList({ sims, error, profile, quad, heading = true }: { sims: SimRates[]; error: string | null; profile: RateProfile; quad: { paths: string[]; device: string | null }; heading?: boolean }) {
  const sync = useStore((s) => s.openSimSync);
  const restore = useStore((s) => s.openSimRestore);
  const applyOpen = useStore((s) => !!s.applySheet);
  // The newest backup QuadCam took of each sim's file, read again when a sheet closes.
  const [backups, setBackups] = useState<Record<string, BackupSummary | undefined>>({});
  const ids = sims.map((s) => s.id).join(",");
  useEffect(() => {
    if (applyOpen || !ids) return;
    let gone = false;
    Promise.all(ids.split(",").map((id) => api.gearBackups(`sim-${id}`).then((l) => [id, l[l.length - 1]] as const, () => [id, undefined] as const))).then((all) => {
      if (!gone) setBackups(Object.fromEntries(all));
    });
    return () => {
      gone = true;
    };
  }, [ids, applyOpen]);
  if (error) {
    return (
      <Banner kind="error" icon="danger-triangle" tint="red">
        {error}
      </Banner>
    );
  }
  if (!sims.length) return null;
  return (
    <section className={styles.sims} aria-label="Sims">
      {heading && <h3 className={styles.title}>Sims</h3>}
      <p className={styles.meta}>
        Compared with {profileLabel(profile)}. A sim takes Betaflight rates, so an Actual or Quick quad is fitted first; the difference is the largest gap over the stick range.
      </p>
      <ul className={styles.simList}>
        {sims.map((s) => (
          <li key={s.id} className={styles.sim}>
            <div className={styles.simHead}>
              <strong>{s.name}</strong>
              <Chip kind={s.in_sync === false ? "accent" : "plain"}>{simState(s)}</Chip>
              {s.running && <Chip>Running</Chip>}
              {backups[s.id] && (
                <>
                  <span className={styles.meta}>Backed up {fmtWhen(backups[s.id]!.taken_at)}</span>
                  <Button variant="ghost" disabled={s.running} aria-label={`Restore ${s.name} from its backup`} onClick={() => void restore({ sim: s.id, backup: null })}>
                    Restore backup
                  </Button>
                </>
              )}
            </div>
            {s.note && <p className={styles.meta}>{s.note}</p>}
            {s.files.map((f) => (
              <div key={f.path}>
                <p className={styles.path}>{f.path}</p>
                {f.error && <p className={styles.problem}>Cannot read: {f.error}</p>}
                <ul className={styles.profiles}>
                  {f.profiles.map((p) => {
                    const w = worstDiff(p.diff);
                    return (
                      <li key={p.name} className={styles.profileRow}>
                        <span className={styles.profileName}>{p.name}</span>{" "}
                        {p.supported ? (
                          <>
                            maximum {p.axes.map((a) => Math.round(a.max_deg_s)).join(" / ")} °/s
                            {p.diff && (p.diff.same ? ", same as the quad" : `, differs by up to ${Math.round(w ?? 0)} °/s`)}
                            {p.diff && !p.diff.same && p.diff.throttle_differs && ", throttle differs"}
                            {p.diff && Math.max(...p.diff.fit_error) > 0.5 && ` (fit error ${Math.round(Math.max(...p.diff.fit_error))} °/s)`}
                          </>
                        ) : (
                          <span className={styles.muted}>{p.note ?? "not read"}</span>
                        )}
                        {p.supported && (
                          <Button
                            variant="ghost"
                            disabled={s.running || p.diff?.same === true}
                            aria-label={`Sync ${profileLabel(profile)} into ${s.name} ${p.name}`}
                            onClick={() => void sync({ sims: [{ sim: s.id, file: f.path, profile: p.name }], paths: quad.paths, device: quad.device, backup: null, profile: profile.index })}
                          >
                            Sync
                          </Button>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
          </li>
        ))}
      </ul>
    </section>
  );
}
