// The sims on this Mac against one quad profile, as one table: a row per sim (its state, its
// files, Restore backup for a sim QuadCam backed up) and a row per sim profile (its maximum
// rates, the difference from the quad, Sync). Used by the Sims page and the Rates segment.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { StatusDot } from "../../../components/dash/StatusDot";
import { api } from "../../../ipc/api";
import type { BackupSummary, RateProfile, SimRateProfile, SimRates } from "../../../ipc/types";
import { fmtWhen } from "../../../lib/backups";
import { profileLabel, simState, simTone, worstDiff } from "../../../lib/rates";
import { useStore } from "../../../store";
import styles from "./Sims.module.css";

type Row =
  | { kind: "sim"; key: string; sim: SimRates }
  | { kind: "profile"; key: string; sim: SimRates; file: string; profile: SimRateProfile; many: boolean }
  | { kind: "error"; key: string; sim: SimRates; file: string; error: string };

const base = (path: string) => path.split("/").pop() ?? path;

function rowsOf(sims: SimRates[]): Row[] {
  return sims.flatMap((s) => [
    { kind: "sim" as const, key: s.id, sim: s },
    ...s.files.flatMap((f): Row[] => [
      ...(f.error ? [{ kind: "error" as const, key: `${s.id}:${f.path}:error`, sim: s, file: f.path, error: f.error }] : []),
      ...f.profiles.map((p) => ({ kind: "profile" as const, key: `${s.id}:${f.path}:${p.name}`, sim: s, file: f.path, profile: p, many: s.files.length > 1 })),
    ]),
  ]);
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

  const columns: Column<Row>[] = [
    {
      key: "sim",
      header: "Sim and profile",
      rowHeader: true,
      width: "40%",
      cell: (r) =>
        r.kind === "sim" ? (
          <span className={styles.sim} data-sim-row="">
            <span className={[styles.simName, !r.sim.enabled && styles.muted].filter(Boolean).join(" ")}>{r.sim.name}</span>
            {r.sim.note && <span className={styles.note}>{r.sim.note}</span>}
            {!compact &&
              r.sim.files.map((f) => (
                <span key={f.path} className={[styles.path, "selectable"].join(" ")}>
                  {f.path}
                </span>
              ))}
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
      key: "max",
      header: "Max °/s (R/P/Y)",
      numeric: true,
      cell: (r) => (r.kind === "profile" && r.profile.supported ? r.profile.axes.map((a) => Math.round(a.max_deg_s)).join(" / ") : null),
    },
    {
      key: "diff",
      header: "Difference",
      cell: (r) => {
        if (r.kind === "error") return <span className={styles.problem}>Cannot read: {r.error}</span>;
        if (r.kind === "sim") return null;
        if (!r.profile.supported) return <span className={styles.muted}>{r.profile.note ?? "Not read"}</span>;
        return difference(r.profile);
      },
    },
    {
      key: "state",
      header: "State",
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
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      cell: (r) => {
        if (r.kind === "sim") {
          return backups[r.sim.id] ? (
            <Button size="sm" disabled={r.sim.running} aria-label={`Restore ${r.sim.name} from its backup`} onClick={() => void restore({ sim: r.sim.id, backup: null })}>
              Restore…
            </Button>
          ) : null;
        }
        if (r.kind !== "profile" || !r.profile.supported) return null;
        const same = r.profile.diff?.same === true;
        return (
          <Button
            size="sm"
            variant={same ? "secondary" : "primary"}
            disabled={r.sim.running || same}
            aria-label={`Sync ${label} into ${r.sim.name} ${r.profile.name}`}
            onClick={() => void sync({ sims: [{ sim: r.sim.id, file: r.file, profile: r.profile.name }], paths: quad.paths, device: quad.device, backup: null, profile: profile.index })}
          >
            Sync…
          </Button>
        );
      },
    },
  ];

  return (
    <div className={styles.wrap}>
      <DataTable label="Sims" columns={columns} rows={rowsOf(sims)} rowKey={(r) => r.key} compact={compact} />
      <p className={styles.footnote}>An Actual or Quick quad is fitted to Betaflight rates first. Difference is the largest gap over the stick range.</p>
    </div>
  );
}
