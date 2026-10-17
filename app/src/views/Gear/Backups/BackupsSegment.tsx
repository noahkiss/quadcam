// A device page's Backups segment (design 2.2, 7.1): the summary tiles, back up now, the card
// check and its repair, then a split view: the device's snapshots on the left, and for the one
// selected its changes from the one before and its files on the right, as two cards.
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { BackupContent, BackupSummary, DiffItem } from "../../../ipc/types";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { DiffView, type DiffEntry } from "../../../components/gear/DiffView";
import { toast } from "../../../components/toastStore";
import { fmtAgo, fmtBytes, plural } from "../../../lib/format";
import { checkFailed, checkFor, failureFor, fmtWhen, jobFor, jobText, TRIGGER_LABEL } from "../../../lib/backups";
import { linkHandle } from "../../../lib/gear";
import type { DeviceRef } from "../slots";
import styles from "./Backups.module.css";
import { PathText } from "../../../components/PathText";

/** A backup's time in the list: "Oct 5, 6:00 AM", with the year when it is not this year. */
const fmtShort = (t: string) => {
  const d = new Date(t);
  return d.toLocaleString(undefined, { month: "short", day: "numeric", year: d.getFullYear() === new Date().getFullYear() ? undefined : "numeric", hour: "numeric", minute: "2-digit" });
};

/** The device's id: saved, or as plugged in. */
const idOf = (d: DeviceRef) => d.device?.id || d.connected?.id || null;

/** True when QuadCam can back the device up as it is plugged in now. */
const canBackUp = (d: DeviceRef) => !!d.connected && !d.unmounted && ((d.kind === "radio" && d.connected.link.kind === "volume") || (d.kind === "fc" && d.connected.link.kind === "serial"));

export function BackupsSegment({ d }: { d: DeviceRef }) {
  const status = useStore((s) => s.gear);
  const devices = useStore((s) => s.devices);
  const id = idOf(d);
  const [list, setList] = useState<BackupSummary[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const job = jobFor(status, id);
  const check = checkFor(status, id);
  const failure = failureFor(status, d.connected ? linkHandle(d.connected.link) : null);
  const isCard = d.connected?.link.kind === "volume";

  useEffect(() => {
    if (!id) return;
    let gone = false;
    api.gearBackups(id).then(
      (l) => {
        if (!gone) setList(l);
      },
      (e) => {
        if (!gone) toast(errText(e), true);
      },
    );
    return () => {
      gone = true;
    };
  }, [id, devices, status?.jobs?.length]);

  const sel = list?.find((b) => b.id === picked) || list?.[0] || null;

  const run = async (what: () => Promise<string>) => {
    setBusy(true);
    try {
      toast(await what());
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(false);
    }
  };
  const backUp = () =>
    run(async () => {
      const link = d.connected!.link;
      const r = await api.gearBackup(link.kind === "serial" ? { port: link.port } : { mount: link.kind === "volume" ? link.mount : null });
      setPicked(r.report.backup.id);
      return r.report.new ? `Backed up: ${plural(r.report.read, "file")} read.` : "No changes since the last backup.";
    });
  const checkCard = () =>
    run(async () => {
      const c = await api.gearCardCheck(id!);
      return c.summary;
    });
  const cleanCard = async () => {
    const found = await api.gearCardClean(id!).catch((e) => {
      toast(errText(e), true);
      return null;
    });
    if (!found) return;
    if (!found.files.length) return void toast("The card holds no ._ files.");
    const ok = await ask(`Remove ${plural(found.files.length, "._ file")}?`, "macOS leaves these hidden files on a card it writes to. The radio does not use them. QuadCam removes only files that are AppleDouble data, then unmounts the card.", { ok: "Remove" });
    if (ok !== true) return;
    run(async () => {
      const r = await api.gearCardClean(id!, true);
      return `Removed ${plural(r.removed, "._ file")}.`;
    });
  };
  const repair = async () => {
    if (!check) return;
    const ok = await ask("Repair this card?", "QuadCam backs the card up first when it can read it, then repairs its file system. A repair cannot be stopped once it starts.", { ok: "Repair" });
    if (ok !== true) return;
    run(async () => {
      const r = await api.gearCardRepair(check.id);
      return r.verify.state === "ok" ? "Repaired. The card checks out." : r.verify.summary;
    });
  };
  const pin = async (b: BackupSummary, pinned: boolean) => {
    try {
      const n = await api.gearBackupPin(b.id, pinned);
      setList((l) => l?.map((x) => (x.id === n.id ? n : x)) || l);
    } catch (e) {
      toast(errText(e), true);
    }
  };

  if (!id) return <EmptyState icon="history">QuadCam backs this device up once it knows its id.</EmptyState>;
  const newest = list?.[0];
  const columns: Column<BackupSummary>[] = [
    {
      key: "taken",
      header: "Taken",
      rowHeader: true,
      cell: (b) => (
        <button type="button" className={styles.pick} aria-current={b.id === sel?.id} onClick={() => setPicked(b.id)}>
          {fmtShort(b.taken_at)}
        </button>
      ),
    },
    { key: "kind", header: "Kind", cell: (b) => <span className={styles.nowrap}>{TRIGGER_LABEL[b.trigger]}</span> },
    { key: "files", header: "Files", numeric: true, cell: (b) => b.files },
    { key: "size", header: "Size", numeric: true, cell: (b) => fmtBytes(b.bytes) },
    {
      key: "pinned",
      header: "Pin",
      width: "2.5rem",
      cell: (b) => <input type="checkbox" className={styles.check} checked={b.pinned} aria-label={`Pin the backup of ${fmtWhen(b.taken_at)}`} onChange={(e) => pin(b, e.target.checked)} />,
    },
  ];
  return (
    <>
      <StatRow label="Backups summary">
        <Stat label="Backups" value={list ? list.length : "—"} />
        <Stat label="Pinned" value={list ? list.filter((b) => b.pinned).length : "—"} />
        <Stat label="Last backup" value={fmtAgo(newest?.taken_at)} sub={newest ? fmtWhen(newest.taken_at) : "None"} />
        <Stat label="Size" value={list ? fmtBytes(list.reduce((n, b) => n + b.bytes, 0)) : "—"} sub="All backups" />
      </StatRow>
      {failure && failure.step !== "Card check" && (
        <Banner kind="warning" icon="danger-triangle">
          {failure.step} failed: {failure.message}
        </Banner>
      )}
      {checkFailed(check) && (
        <Banner
          kind="warning"
          icon="danger-triangle"
          action={
            !job && (
              <Button size="sm" onClick={repair} disabled={busy}>
                Repair…
              </Button>
            )
          }
        >
          Card check failed: {check!.summary}
        </Banner>
      )}
      <div className={styles.split}>
        <SectionCard
          title="Backups"
          label="Backup list"
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
            <>
              {d.kind === "radio" && isCard && d.device && !d.unmounted && (
                <Button size="sm" icon="trash-bin-trash" onClick={cleanCard} disabled={busy || !!job}>
                  Clean ._ files
                </Button>
              )}
              {isCard && d.device && !d.unmounted && (
                <Button size="sm" icon="shield" onClick={checkCard} disabled={busy || !!job}>
                  Check card
                </Button>
              )}
              {canBackUp(d) && (
                <Button size="sm" variant="primary" icon="history" onClick={backUp} disabled={busy || !!job}>
                  Back up now
                </Button>
              )}
            </>
          }
        >
          {list && <DataTable label="Backups" columns={columns} rows={list} rowKey={(b) => b.id} compact empty={<EmptyState icon="history">No backups yet.</EmptyState>} />}
        </SectionCard>
        {sel && <BackupDetail key={sel.id} b={sel} first={list![list!.length - 1].id === sel.id} />}
      </div>
    </>
  );
}

/** One backup: its changes from the one before, and its files with one file's text. */
function BackupDetail({ b, first }: { b: BackupSummary; first: boolean }) {
  const [diff, setDiff] = useState<DiffItem[] | null>(null);
  const [content, setContent] = useState<BackupContent | null>(null);
  const [file, setFile] = useState<BackupContent | null>(null);

  useEffect(() => {
    let gone = false;
    const fail = (e: unknown) => !gone && toast(errText(e), true);
    if (!first) api.gearBackupDiff(b.id).then((x) => !gone && setDiff(x), fail);
    api.gearBackupRead(b.id, null).then((x) => !gone && setContent(x), fail);
    return () => {
      gone = true;
    };
  }, [b.id, first]);

  const open = (path: string) => api.gearBackupRead(b.id, path).then(setFile, (e) => toast(errText(e), true));

  return (
    <section className={styles.detail} aria-label={`Backup of ${fmtWhen(b.taken_at)}`}>
      <SectionCard title="Changes from the one before">
        {first ? (
          <EmptyState icon="history">The first backup of this device.</EmptyState>
        ) : (
          diff && (diff.length ? <DiffView items={diff as DiffEntry[]} /> : <EmptyState icon="check-circle">No changes.</EmptyState>)
        )}
      </SectionCard>
      <SectionCard title="Contents">
        {content && (
          <ul aria-label="Backup files" className={styles.fileList}>
            {content.backup.files.map((f) => (
              <li key={f.path}>
                <button type="button" className={styles.file} aria-current={file?.path === f.path} onClick={() => open(f.path)}>
                  <PathText path={f.path} selectable={false} />
                  <span className={styles.size}>{fmtBytes(f.size)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {file &&
          (file.text != null ? (
            <pre className={`${styles.text} mono selectable`} aria-label={file.path || ""}>
              {file.text}
            </pre>
          ) : (
            <p className={styles.empty}>
              <PathText path={file.path} /> is not a text file.
            </p>
          ))}
      </SectionCard>
    </section>
  );
}
