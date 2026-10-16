// The sims on this Mac against one quad profile, as one table: a row per sim (its state,
// its maximum rates, the difference from the quad and Sync when it has one profile; a … menu
// with Show in Finder per file and Restore… for a sim QuadCam backed up), and under a sim with
// several profiles, a row per profile behind its disclosure. Used by the Sim page's Rates
// section and the Rates segment.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import type { MenuItem } from "../../../components/Menu";
import { StatusDot } from "../../../components/dash/StatusDot";
import { toast } from "../../../components/toastStore";
import { api, errText, reveal } from "../../../ipc/api";
import type { BackupSummary, RateProfile, SimRateProfile, SimRates } from "../../../ipc/types";
import { fmtWhen } from "../../../lib/backups";
import { profileLabel, simState, simTone, worstDiff } from "../../../lib/rates";
import { useStore } from "../../../store";
import styles from "./Sims.module.css";

type Row =
  | { kind: "sim"; key: string; sim: SimRates; only: { file: string; profile: SimRateProfile } | null; count: number }
  | { kind: "profile"; key: string; sim: SimRates; file: string; profile: SimRateProfile; many: boolean }
  | { kind: "error"; key: string; sim: SimRates; file: string; error: string };

const base = (path: string) => path.split("/").pop() ?? path;

/** A row per sim; a sim with one readable profile carries it on its own row, a sim with
 *  several lists them under it while it is open. */
function rowsOf(sims: SimRates[], open: (id: string) => boolean): Row[] {
  return sims.flatMap((s) => {
    const errors = s.files.flatMap((f): Row[] => (f.error ? [{ kind: "error" as const, key: `${s.id}:${f.path}:error`, sim: s, file: f.path, error: f.error }] : []));
    const profiles = s.files.flatMap((f) => f.profiles.map((p) => ({ kind: "profile" as const, key: `${s.id}:${f.path}:${p.name}`, sim: s, file: f.path, profile: p, many: s.files.length > 1 })));
    const only = profiles.length === 1 ? { file: profiles[0].file, profile: profiles[0].profile } : null;
    return [{ kind: "sim" as const, key: s.id, sim: s, only, count: profiles.length }, ...errors, ...(only || !open(s.id) ? [] : profiles)];
  });
}

/** The difference of a sim profile from the quad, as short text. */
function difference(p: SimRateProfile): string {
  if (!p.diff) return "—";
  const parts = [p.diff.same ? "Same as the quad" : `Up to ${Math.round(worstDiff(p.diff) ?? 0)} °/s`];
  if (!p.diff.same && p.diff.throttle_differs) parts.push("throttle differs");
  const fit = Math.max(...p.diff.fit_error);
  if (fit > 0.5) parts.push(`fit error ${Math.round(fit)} °/s`);
  return parts.join(", ");
}

export function SimTable({ sims, error, profile, quad, compact }: { sims: SimRates[]; error: string | null; profile: RateProfile; quad: { paths: string[]; device: string | null }; compact?: boolean }) {
  const sync = useStore((s) => s.openSimSync);
  const restore = useStore((s) => s.openSimRestore);
  const applyOpen = useStore((s) => !!s.applySheet);
  // The newest backup QuadCam took of each sim's file, read again when a sheet closes.
  const [backups, setBackups] = useState<Record<string, BackupSummary | undefined>>({});
  // The sims whose profiles are folded away (all open at first).
  const [folded, setFolded] = useState<Set<string>>(new Set());
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
  const label = profileLabel(profile);
  const toggle = (id: string) => setFolded((f) => (f.has(id) ? new Set([...f].filter((x) => x !== id)) : new Set([...f, id])));

  const syncButton = (sim: SimRates, file: string, p: SimRateProfile) => {
    if (!p.supported) return null;
    const same = p.diff?.same === true;
    return (
      <Button
        size="sm"
        variant={same ? "secondary" : "primary"}
        disabled={sim.running || same}
        aria-label={`Sync ${label} into ${sim.name} ${p.name}`}
        onClick={() => void sync({ sims: [{ sim: sim.id, file, profile: p.name }], paths: quad.paths, device: quad.device, backup: null, profile: profile.index })}
      >
        Sync…
      </Button>
    );
  };
  const maxOf = (p: SimRateProfile) => (p.supported ? p.axes.map((a) => Math.round(a.max_deg_s)).join(" / ") : null);
  const diffOf = (p: SimRateProfile) => (p.supported ? difference(p) : <span className={styles.muted}>{p.note ?? "Not read"}</span>);

  const columns: Column<Row>[] = [
    {
      key: "sim",
      header: "Sim",
      rowHeader: true,
      width: "34%",
      cell: (r) =>
        r.kind === "sim" ? (
          <span className={styles.sim} data-sim-row="" title={r.sim.files.map((f) => f.path).join("\n") || undefined}>
            <span className={styles.simHead}>
              {r.count > 1 && (
                <button type="button" className={styles.disclose} aria-expanded={!folded.has(r.sim.id)} aria-label={`${folded.has(r.sim.id) ? "Show" : "Hide"} the profiles of ${r.sim.name}`} onClick={() => toggle(r.sim.id)}>
                  <span className={styles.triangle} aria-hidden="true" />
                </button>
              )}
              <span className={[styles.simName, !r.sim.enabled && styles.muted].filter(Boolean).join(" ")}>{r.sim.name}</span>
            </span>
            {r.only && <span className={styles.note}>{r.only.profile.name}</span>}
            {r.count > 1 && <span className={styles.note}>{r.count} profiles</span>}
            {r.sim.note && <span className={styles.note}>{r.sim.note}</span>}
          </span>
        ) : (
          <span className={[styles.sim, styles.profile].join(" ")}>
            <span className="visually-hidden">{r.sim.name} </span>
            <span className={r.kind === "error" ? styles.muted : undefined}>{r.kind === "error" ? base(r.file) : r.profile.name}</span>
            {r.kind === "profile" && r.many && <span className={styles.note}>{base(r.file)}</span>}
          </span>
        ),
    },
    {
      key: "state",
      header: "State",
      stack: "top",
      cell: (r) => {
        if (r.kind !== "sim") return null;
        const b = backups[r.sim.id];
        return (
          <span className={styles.state}>
            <StatusDot tone={simTone(r.sim)}>{simState(r.sim)}</StatusDot>
            {r.sim.running && <Badge>Running</Badge>}
            {b && <span className={styles.note}>Backed up {fmtWhen(b.taken_at)}</span>}
          </span>
        );
      },
    },
    {
      key: "max",
      header: "Max °/s (R/P/Y)",
      numeric: true,
      cell: (r) => (r.kind === "profile" ? maxOf(r.profile) : r.kind === "sim" && r.only ? maxOf(r.only.profile) : null),
    },
    {
      key: "diff",
      header: (
        <span title="An Actual or Quick quad is fitted to Betaflight rates first. Difference is the largest gap over the stick range." className={styles.help}>
          Difference
        </span>
      ),
      cell: (r) => {
        if (r.kind === "error") return <span className={styles.problem}>Cannot read: {r.error}</span>;
        if (r.kind === "sim") return r.only ? diffOf(r.only.profile) : null;
        return diffOf(r.profile);
      },
    },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      cell: (r) => {
        if (r.kind === "profile") return syncButton(r.sim, r.file, r.profile);
        if (r.kind !== "sim") return null;
        const items: (MenuItem | null)[] = r.sim.files.map((f) => ({ label: r.sim.files.length > 1 ? `Show ${base(f.path)} in Finder` : "Show in Finder", icon: "finder" as const, run: () => void reveal(f.path).catch((e) => toast(errText(e), true)) }));
        if (backups[r.sim.id]) items.push(null, { label: "Restore…", disabled: r.sim.running, run: () => void restore({ sim: r.sim.id, backup: null }) });
        return (
          <span className={styles.actions}>
            {r.only && syncButton(r.sim, r.only.file, r.only.profile)}
            {items.length > 0 && <MoreMenu label={`More actions for ${r.sim.name}`} items={items} />}
          </span>
        );
      },
    },
  ];

  return (
    <div className={styles.wrap}>
      <DataTable label="Sims" columns={columns} rows={rowsOf(sims, (id) => !folded.has(id))} rowKey={(r) => r.key} compact={compact} />
    </div>
  );
}
