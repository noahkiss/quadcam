// An FC page's Blackbox segment (design 7.12): the summary tiles (the flash now, from
// `gear_blackbox_summary` while the FC is plugged in; the stored pulls), pull the flash, erase
// it by hand, and the stored pulls with the logs of the one selected and the flights they are
// paired with by order.
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { BlackboxEntry, BlackboxFlash, BlackboxPullResult, LogInfo } from "../../../ipc/types";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { toast } from "../../../components/toastStore";
import { fmtAgo, fmtBytes, plural } from "../../../lib/format";
import { failureFor, fmtWhen, jobFor, jobText } from "../../../lib/backups";
import { linkHandle } from "../../../lib/gear";
import type { DeviceRef } from "../slots";
import styles from "./Blackbox.module.css";

const idOf = (d: DeviceRef) => d.device?.id || d.connected?.id || null;

/** What a pull did, as one toast. */
export function pullText(r: BlackboxPullResult): string {
  if (!r.pull) return "The blackbox flash is empty.";
  let out = `${plural(r.pull.logs.length, "log")} stored (${fmtBytes(r.read_bytes)}).`;
  if (r.erase === "done") out += " Flash erased. Safe to unplug.";
  else if (r.erase === "skipped") out += ` ${r.erase_note || "The flash was not erased."}`;
  else out += " Flash kept.";
  return out;
}

export function BlackboxSegment({ d }: { d: DeviceRef }) {
  const status = useStore((s) => s.gear);
  const devices = useStore((s) => s.devices);
  const id = idOf(d);
  const [list, setList] = useState<BlackboxEntry[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [read, setRead] = useState<BlackboxFlash | null>(null);
  const [reads, setReads] = useState(0);
  const job = jobFor(status, id);
  const link = d.connected && !d.unmounted ? d.connected.link : null;
  const port = link?.kind === "serial" ? link.port : null;
  const paused = !!port && !!status?.paused?.includes(port);
  const failure = failureFor(status, d.connected ? linkHandle(d.connected.link) : null);

  useEffect(() => {
    if (!id) return;
    let gone = false;
    api.gearBlackbox(id).then(
      (l) => !gone && setList(l),
      (e) => !gone && toast(errText(e), true),
    );
    return () => {
      gone = true;
    };
  }, [id, devices, status?.jobs?.length]);

  // The flash now: read once when the FC is plugged in, again after a job, a pull or an
  // erase. A port that is paused or busy shows "—"; the read never runs during a job.
  const working = !!job;
  useEffect(() => {
    if (!port || paused || working) return;
    let gone = false;
    api.gearBlackboxSummary(port).then(
      (f) => !gone && setRead(f),
      () => !gone && setRead(null),
    );
    return () => {
      gone = true;
    };
  }, [port, paused, working, reads]);
  const flash = port && !paused && !working && read?.port === port ? read : null;

  const sel = list?.find((e) => e.pull.id === picked) || list?.[0] || null;
  const last = list?.[0]?.pull;

  const run = async (what: () => Promise<string>) => {
    setBusy(true);
    try {
      toast(await what());
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(false);
      setReads((n) => n + 1);
    }
  };
  const pull = () =>
    run(async () => {
      const j = await api.gearBlackboxPull(port);
      if (j.result.pull) setPicked(j.result.pull.id);
      return pullText(j.result);
    });
  const erase = async () => {
    const ok = await ask("Erase the blackbox flash?", "This deletes the logs on the FC. QuadCam erases only when it has stored a pull of exactly what the flash holds.", { ok: "Erase" });
    if (ok !== true) return;
    run(async () => {
      await api.gearBlackboxErase(port);
      return "Flash erased. Safe to unplug.";
    });
  };

  if (!id) return <EmptyState icon="info">QuadCam pulls the blackbox once it knows this FC's id.</EmptyState>;
  const canErase = flash ? flash.pulled : !!list?.some((e) => !e.pull.erased);
  const columns: Column<BlackboxEntry>[] = [
    {
      key: "pulled",
      header: "Pulled",
      rowHeader: true,
      cell: (e) => (
        <button type="button" className={styles.pick} aria-current={e.pull.id === sel?.pull.id} onClick={() => setPicked(e.pull.id)}>
          {fmtWhen(e.pull.pulled_at)}
        </button>
      ),
    },
    { key: "aircraft", header: "Aircraft", cell: (e) => e.pull.aircraft || "None" },
    { key: "logs", header: "Logs", numeric: true, cell: (e) => e.pull.logs.length },
    { key: "size", header: "Size", numeric: true, cell: (e) => fmtBytes(e.pull.used) },
    { key: "flash", header: "Flash", cell: (e) => (e.pull.erased ? "Erased" : "Kept") },
  ];
  return (
    <>
      <StatRow label="Blackbox summary">
        <Stat
          label="Flash used"
          value={flash?.supported ? fmtBytes(flash.used) : "—"}
          sub={flash?.supported ? `of ${fmtBytes(flash.total)}${flash.used === 0 ? ", empty" : flash.pulled ? ", pulled" : ", not pulled"}` : flash ? "No flash chip" : port ? "Not read" : "FC not plugged in"}
          meter={flash?.supported && flash.total ? flash.used / flash.total : undefined}
          tone={flash?.supported && flash.total && flash.used / flash.total > 0.9 ? "warn" : undefined}
        />
        <Stat label="Pulls" value={list ? list.length : "—"} />
        <Stat label="Last pull" value={fmtAgo(last?.pulled_at)} sub={last ? fmtWhen(last.pulled_at) : "None"} />
        <Stat label="Logs" value={last ? last.logs.length : "—"} sub={last ? "In the last pull" : undefined} />
      </StatRow>
      {failure && failure.step === "Blackbox" && (
        <Banner kind="warning" icon="danger-triangle">
          {failure.step} failed: {failure.message}
        </Banner>
      )}
      <SectionCard
        title="Pulls"
        status={
          job && (
            <span className={styles.job} role="status">
              {job.step}: {jobText(job)}
              <Button size="sm" variant="ghost" onClick={() => api.gearStop(job.handle).catch((e) => toast(errText(e), true))} disabled={job.stopping}>
                Stop
              </Button>
            </span>
          )
        }
        actions={
          port && (
            <>
              <Button size="sm" icon="trash-bin-trash" onClick={erase} disabled={busy || !!job || !canErase}>
                Erase Flash…
              </Button>
              <Button size="sm" variant="primary" icon="import" onClick={pull} disabled={busy || !!job}>
                Pull Blackbox
              </Button>
            </>
          )
        }
      >
        {list && <DataTable label="Blackbox pulls" columns={columns} rows={list} rowKey={(e) => e.pull.id} compact empty={<EmptyState icon="import">No blackbox pulled yet.</EmptyState>} />}
      </SectionCard>
      {sel && <PullDetail key={sel.pull.id} e={sel} />}
    </>
  );
}

/** One pull: its logs, and the flights they were paired with by order. */
function PullDetail({ e }: { e: BlackboxEntry }) {
  const flightOf = new Map(e.flights.links.map((l) => [l.log, l]));
  const columns: Column<LogInfo>[] = [
    { key: "log", header: "Log", numeric: true, width: "3rem", cell: (l) => l.index },
    { key: "firmware", header: "Firmware", cell: (l) => l.firmware || "Unknown" },
    { key: "size", header: "Size", numeric: true, cell: (l) => fmtBytes(l.size) },
    {
      key: "flight",
      header: "Flight, paired by order",
      cell: (l) => {
        const f = flightOf.get(l.index);
        if (!f) return "None";
        return (
          <>
            {f.flight}
            {f.fits === false && (
              <>
                {" "}
                <Badge tone="warn">Size does not fit</Badge>
              </>
            )}
          </>
        );
      },
    },
  ];
  return (
    <SectionCard title={`Logs pulled ${fmtWhen(e.pull.pulled_at)}`} label={`Blackbox pulled ${fmtWhen(e.pull.pulled_at)}`}>
      {e.pull.erase_note && <p className={styles.note}>{e.pull.erase_note}</p>}
      <DataTable label="Logs" columns={columns} rows={e.pull.logs} rowKey={(l) => String(l.index)} compact />
      <p className={styles.note}>{e.pull.aircraft ? "Logs pair with the aircraft's radio-log flights by order, newest with newest." : "No aircraft is linked to this FC: its logs pair with no flights."}</p>
    </SectionCard>
  );
}
