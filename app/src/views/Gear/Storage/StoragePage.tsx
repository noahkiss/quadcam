// Gear > Storage (design 2.2, 7.1): the gear folder's size as one usage bar and per device as
// a table (Export… and Show in Finder in each row's … menu), the folder (the subtitle) with
// Show in Finder, Prune now, and Import backups… (an old backup folder, shown first as a dry
// run; its report opens in a drawer).
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText, pickFolder, reveal } from "../../../ipc/api";
import type { DeviceStorage, ImportBackupsReport, StorageView } from "../../../ipc/types";
import { Badge, type BadgeTone } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { DetailDrawer, WithDrawer } from "../../../components/dash/DetailDrawer";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { toast } from "../../../components/toastStore";
import { fmtBytes, plural } from "../../../lib/format";
import { KIND_LABEL } from "../../../lib/gear";
import { fmtWhen } from "../../../lib/backups";
import styles from "./Storage.module.css";
import { PathText } from "../../../components/PathText";

const OUTCOME_LABEL: Record<ImportBackupsReport["items"][number]["outcome"], string> = {
  imported: "Imported",
  same: "Already kept",
  logs: "Logs",
  skipped: "Skipped",
};

const OUTCOME_TONE: Record<ImportBackupsReport["items"][number]["outcome"], BadgeTone> = { imported: "ok", same: "neutral", logs: "accent", skipped: "warn" };

type ImportItem = ImportBackupsReport["items"][number];

export function StoragePage() {
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

  const name = (d: DeviceStorage) => d.name || (d.kind ? `${KIND_LABEL[d.kind]} not in the list` : d.device);
  const columns: Column<DeviceStorage>[] = [
    {
      key: "device",
      header: "Device",
      rowHeader: true,
      cell: (d) => (
        <>
          {name(d)}
          {d.pinned > 0 && (
            <>
              {" "}
              <Badge>{d.pinned} pinned</Badge>
            </>
          )}
        </>
      ),
    },
    { key: "size", header: "Size", numeric: true, stack: "top", cell: (d) => fmtBytes(d.total_bytes) },
    { key: "backups", header: "Backups", numeric: true, cell: (d) => d.snapshots },
    { key: "logs", header: "Logs", numeric: true, cell: (d) => d.logs },
    { key: "latest", header: "Latest", optional: true, cell: (d) => fmtWhen(d.latest) },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "2.5rem",
      cell: (d) =>
        view && (
          <MoreMenu
            label={`More actions for ${name(d)}`}
            items={[
              { label: "Export…", disabled: busy || d.snapshots === 0, run: () => void exportDevice(d.device) },
              { label: "Show in Finder", icon: "finder", disabled: d.snapshots === 0, run: () => void reveal(`${view.gear_dir}/snapshots/${d.device}`).catch((e) => toast(errText(e), true)) },
            ]}
          />
        ),
    },
  ];
  const items: Column<ImportItem>[] = [
    { key: "outcome", header: "Outcome", width: "8rem", cell: (i) => <Badge tone={OUTCOME_TONE[i.outcome]}>{OUTCOME_LABEL[i.outcome]}</Badge> },
    { key: "path", header: "Folder", rowHeader: true, cell: (i) => <PathText path={i.path} /> },
    { key: "reason", header: "Reason", cell: (i) => (i.outcome === "skipped" && i.reason) || "" },
  ];

  return (
    <>
      <PageHeader
        id="storage-title"
        title="Storage"
        subtitle={view && <PathText path={view.gear_dir} />}
        fold={[
          { label: "Show in Finder", icon: "finder", disabled: !view, run: () => view && void reveal(view.gear_dir).catch((e) => toast(errText(e), true)) },
          { label: "Import backups…", icon: "import", disabled: busy, run: () => void importFolder() },
          { label: "Prune now", disabled: busy, run: () => void prune() },
        ]}
        moreLabel="Storage actions"
      />
      <WithDrawer
        drawer={
          report && (
            <DetailDrawer title={report.dry_run ? "Import (not run)" : "Import"} subtitle={`${plural(report.imported, "backup")} · ${plural(report.logs.added, "log")}`} onClose={() => setReport(null)}>
              <DataTable label="Import items" columns={items} rows={report.items} rowKey={(i) => i.path} compact />
            </DetailDrawer>
          )
        }
      >
        {view && <Usage v={view} />}
        {view && (
          <div className={styles.rows}>
            <DataTable label="Storage by device" columns={columns} rows={view.devices} rowKey={(d) => d.device} empty={<EmptyState icon="hdd">No backups yet.</EmptyState>} />
          </div>
        )}
      </WithDrawer>
    </>
  );
}

/** The gear folder's size as one bar: backups, stored files and radio logs, with a legend and
 *  the total. */
function Usage({ v }: { v: StorageView }) {
  const parts = [
    { label: "Backups", bytes: v.manifest_bytes, sub: plural(v.snapshots, "backup"), cls: styles.backups },
    { label: "Stored files", bytes: v.blob_bytes, sub: v.shared_blob_bytes > 0 ? `${fmtBytes(v.shared_blob_bytes)} shared` : plural(v.blobs, "file"), cls: styles.files },
    { label: "Radio logs", bytes: v.log_bytes, sub: null, cls: styles.logs },
  ];
  const total = Math.max(1, v.total_bytes);
  return (
    <section className={styles.usage} aria-label="Usage">
      <div className={styles.usageHead}>
        <span className={styles.total}>{fmtBytes(v.total_bytes)}</span>
        <span className={styles.muted}>in the gear folder</span>
      </div>
      <div className={styles.bar} role="img" aria-label={parts.map((p) => `${p.label} ${fmtBytes(p.bytes)}`).join(", ")}>
        {parts.map((p) => (
          <span key={p.label} className={p.cls} style={{ width: `${(p.bytes / total) * 100}%` }} />
        ))}
      </div>
      <ul className={styles.legend}>
        {parts.map((p) => (
          <li key={p.label}>
            <span className={[styles.swatch, p.cls].join(" ")} aria-hidden="true" />
            <span>{p.label}</span>
            <b>{fmtBytes(p.bytes)}</b>
            {p.sub && <span className={styles.muted}>{p.sub}</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}
