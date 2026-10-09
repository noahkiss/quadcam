// A device page's Backups segment (design 2.2, 7.1): back up now, the card check and its
// repair, the device's snapshots, and for the one selected its changes from the one before
// and its files.
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { BackupContent, BackupSummary, DiffItem } from "../../../ipc/types";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { DiffView, type DiffEntry } from "../../../components/gear/DiffView";
import { toast } from "../../../components/toastStore";
import { fmtBytes, plural } from "../../../lib/format";
import { checkFailed, checkFor, failureFor, fmtWhen, jobFor, jobText, TRIGGER_LABEL } from "../../../lib/backups";
import { linkHandle } from "../../../lib/gear";
import type { DeviceRef } from "../slots";
import styles from "./Backups.module.css";

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

  if (!id) return <p className={styles.empty}>QuadCam backs this device up once it knows its id.</p>;
  return (
    <div className={styles.segment}>
      <div className={styles.bar}>
        {canBackUp(d) && (
          <Button icon="history" onClick={backUp} disabled={busy || !!job}>
            Back up now
          </Button>
        )}
        {isCard && d.device && !d.unmounted && (
          <Button icon="shield" onClick={checkCard} disabled={busy || !!job}>
            Check card
          </Button>
        )}
        {d.kind === "radio" && isCard && d.device && !d.unmounted && (
          <Button icon="trash-bin-trash" onClick={cleanCard} disabled={busy || !!job}>
            Clean ._ files
          </Button>
        )}
        {job && (
          <span className={styles.job} role="status">
            {job.step}: {jobText(job)}
            <Button size="sm" variant="ghost" onClick={() => api.gearStop(job.handle).catch((e) => toast(errText(e), true))} disabled={job.stopping}>
              Stop
            </Button>
          </span>
        )}
      </div>
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
      {list && !list.length && <p className={styles.empty}>No backups yet.</p>}
      {list && list.length > 0 && (
        <table className={styles.table} aria-label="Backups">
          <thead>
            <tr>
              <th scope="col">Taken</th>
              <th scope="col">Kind</th>
              <th scope="col" className={styles.num}>
                Files
              </th>
              <th scope="col" className={styles.num}>
                Size
              </th>
              <th scope="col">Pinned</th>
            </tr>
          </thead>
          <tbody>
            {list.map((b) => (
              <tr key={b.id} className={b.id === sel?.id ? styles.selected : undefined}>
                <td>
                  <button type="button" className={styles.pick} aria-current={b.id === sel?.id} onClick={() => setPicked(b.id)}>
                    {fmtWhen(b.taken_at)}
                  </button>
                </td>
                <td>{TRIGGER_LABEL[b.trigger]}</td>
                <td className={styles.num}>{b.files}</td>
                <td className={styles.num}>{fmtBytes(b.bytes)}</td>
                <td>
                  <input type="checkbox" checked={b.pinned} aria-label={`Pin the backup of ${fmtWhen(b.taken_at)}`} onChange={(e) => pin(b, e.target.checked)} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {sel && <BackupDetail key={sel.id} b={sel} first={list![list!.length - 1].id === sel.id} />}
    </div>
  );
}

/** One backup: its changes from the one before, or its files and one file's text. */
function BackupDetail({ b, first }: { b: BackupSummary; first: boolean }) {
  const [view, setView] = useState<"changes" | "files">(first ? "files" : "changes");
  const [diff, setDiff] = useState<DiffItem[] | null>(null);
  const [content, setContent] = useState<BackupContent | null>(null);
  const [file, setFile] = useState<BackupContent | null>(null);

  useEffect(() => {
    let gone = false;
    const fail = (e: unknown) => !gone && toast(errText(e), true);
    if (view === "changes" && !first) api.gearBackupDiff(b.id).then((x) => !gone && setDiff(x), fail);
    if (view === "files") api.gearBackupRead(b.id, null).then((x) => !gone && setContent(x), fail);
    return () => {
      gone = true;
    };
  }, [b.id, view, first]);

  const open = (path: string) => api.gearBackupRead(b.id, path).then(setFile, (e) => toast(errText(e), true));

  return (
    <section className={styles.detail} aria-label={`Backup of ${fmtWhen(b.taken_at)}`}>
      <SegmentedControl
        label="Backup view"
        value={view}
        onChange={(v) => setView(v as "changes" | "files")}
        segments={[
          { value: "changes", label: "Changes" },
          { value: "files", label: "Files" },
        ]}
      />
      {view === "changes" &&
        (first ? <p className={styles.empty}>The first backup of this device.</p> : diff && (diff.length ? <DiffView items={diff as DiffEntry[]} /> : <p className={styles.empty}>No changes.</p>))}
      {view === "files" && content && (
        <div className={styles.files}>
          <ul aria-label="Files" className={styles.fileList}>
            {content.backup.files.map((f) => (
              <li key={f.path}>
                <button type="button" className={styles.pick} aria-current={file?.path === f.path} onClick={() => open(f.path)}>
                  <span className="mono">{f.path}</span>
                  <span className={styles.size}>{fmtBytes(f.size)}</span>
                </button>
              </li>
            ))}
          </ul>
          {file &&
            (file.text != null ? (
              <pre className={`${styles.text} mono selectable`} aria-label={file.path || ""}>
                {file.text}
              </pre>
            ) : (
              <p className={styles.empty}>{file.path} is not a text file.</p>
            ))}
        </div>
      )}
    </section>
  );
}
