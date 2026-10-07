// Gear > Storage (design 2.2, 7.1): the gear folder's size in total and per device, Prune
// now, Export…, and Import backups… (an old backup folder, shown first as a dry run).
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText, pickFolder } from "../../../ipc/api";
import type { ImportBackupsReport, StorageView } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { toast } from "../../../components/toastStore";
import { fmtBytes, plural, tilde } from "../../../lib/format";
import { KIND_LABEL } from "../../../lib/gear";
import { fmtWhen } from "../../../lib/backups";
import styles from "./Storage.module.css";

const OUTCOME_LABEL: Record<ImportBackupsReport["items"][number]["outcome"], string> = {
  imported: "Imported",
  same: "Already kept",
  logs: "Logs",
  skipped: "Skipped",
};

export function StoragePage() {
  const home = useStore((s) => s.home);
  const devices = useStore((s) => s.devices);
  const [view, setView] = useState<StorageView | null>(null);
  const [report, setReport] = useState<ImportBackupsReport | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let gone = false;
    api.gearStorage().then(
      (v) => !gone && setView(v),
      (e) => !gone && toast(errText(e), true),
    );
    return () => {
      gone = true;
    };
  }, [devices, report]);

  const work = async (f: () => Promise<void>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(false);
    }
  };

  const prune = () =>
    work(async () => {
      const dry = await api.gearPrune(true);
      if (!dry.dropped.length && !dry.collected.blobs) {
        toast("Nothing to prune.");
        return;
      }
      const ok = await ask("Prune backups?", `${plural(dry.dropped.length, "backup")} and ${fmtBytes(dry.collected.bytes)} of stored files go. Pinned backups and backups taken before an apply stay.`, { ok: "Prune", danger: true });
      if (ok !== true) return;
      const r = await api.gearPrune(false);
      toast(`Pruned ${plural(r.dropped.length, "backup")}.`);
      setView(await api.gearStorage());
    });

  const importFolder = () =>
    work(async () => {
      const folder = await pickFolder("Import backups");
      if (!folder) return;
      const dry = await api.gearImportBackups(folder, true);
      if (!dry.imported && !dry.logs.added && !dry.logs.grown) {
        setReport(dry);
        toast("Nothing new to import.");
        return;
      }
      const ok = await ask("Import these backups?", `${plural(dry.imported, "new backup")}, ${plural(dry.logs.added, "new log")}. ${dry.skipped ? `${dry.skipped} skipped.` : ""}`.trim(), { ok: "Import" });
      if (ok !== true) {
        setReport(dry);
        return;
      }
      const r = await api.gearImportBackups(folder, false);
      setReport(r);
      toast(`Imported ${plural(r.imported, "backup")}.`);
    });

  const exportDevice = (device: string) =>
    work(async () => {
      const to = await pickFolder("Export backups to");
      if (!to) return;
      const r = await api.gearExport(to, { device });
      toast(`Exported ${plural(r.folders.length, "backup")}.`);
    });

  return (
    <div className={styles.page} aria-labelledby="storage-title">
      <h2 id="storage-title" className={styles.title}>
        Storage
      </h2>
      {view && (
        <dl className={styles.facts}>
          <div>
            <dt>Total</dt>
            <dd>{fmtBytes(view.total_bytes)}</dd>
          </div>
          <div>
            <dt>Backups</dt>
            <dd>{view.snapshots}</dd>
          </div>
          <div>
            <dt>Stored files</dt>
            <dd>
              {fmtBytes(view.blob_bytes)}
              {view.shared_blob_bytes > 0 && <span className={styles.muted}> ({fmtBytes(view.shared_blob_bytes)} shared)</span>}
            </dd>
          </div>
          <div>
            <dt>Radio logs</dt>
            <dd>{fmtBytes(view.log_bytes)}</dd>
          </div>
          <div>
            <dt>Folder</dt>
            <dd className="selectable">{tilde(view.gear_dir, home)}</dd>
          </div>
        </dl>
      )}
      <div className={styles.bar}>
        <Button onClick={prune} disabled={busy}>
          Prune now
        </Button>
        <Button icon="import" onClick={importFolder} disabled={busy}>
          Import backups…
        </Button>
      </div>
      {view && view.devices.length > 0 && (
        <table className={styles.table} aria-label="Storage by device">
          <thead>
            <tr>
              <th scope="col">Device</th>
              <th scope="col" className={styles.num}>
                Backups
              </th>
              <th scope="col" className={styles.num}>
                Logs
              </th>
              <th scope="col" className={styles.num}>
                Size
              </th>
              <th scope="col">Latest</th>
              <th scope="col">
                <span className="visually-hidden">Export</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {view.devices.map((d) => (
              <tr key={d.device}>
                <th scope="row">
                  {d.name || (d.kind ? `${KIND_LABEL[d.kind]} not in the list` : d.device)}
                  {d.pinned > 0 && <span className={styles.muted}> · {d.pinned} pinned</span>}
                </th>
                <td className={styles.num}>{d.snapshots}</td>
                <td className={styles.num}>{d.logs}</td>
                <td className={styles.num}>{fmtBytes(d.total_bytes)}</td>
                <td>{fmtWhen(d.latest)}</td>
                <td>
                  {d.snapshots > 0 && (
                    <Button size="sm" variant="ghost" onClick={() => exportDevice(d.device)} disabled={busy} aria-label={`Export ${d.name || d.device}`}>
                      Export…
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {view && !view.devices.length && <p className={styles.muted}>No backups yet.</p>}
      {report && (
        <section aria-label="Import" className={styles.report}>
          <h3>{report.dry_run ? "Import (not run)" : "Import"}</h3>
          <ul className={styles.items}>
            {report.items.map((i) => (
              <li key={i.path}>
                <span className={styles.outcome} data-outcome={i.outcome}>
                  {OUTCOME_LABEL[i.outcome]}
                </span>
                <span className="selectable">{tilde(i.path, home)}</span>
                {i.outcome === "skipped" && i.reason && <span className={styles.muted}>{i.reason}</span>}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
