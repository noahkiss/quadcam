// Gear > Storage (design 2.2, 7.1): the gear folder's size as tiles and per device as a table,
// the folder with Show in Finder, Prune now, Export…, and Import backups… (an old backup
// folder, shown first as a dry run).
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText, pickFolder, reveal } from "../../../ipc/api";
import type { DeviceStorage, ImportBackupsReport, StorageView } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Badge, type BadgeTone } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { FactList } from "../../../components/dash/FactList";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
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

const OUTCOME_TONE: Record<ImportBackupsReport["items"][number]["outcome"], BadgeTone> = { imported: "ok", same: "neutral", logs: "accent", skipped: "warn" };

type ImportItem = ImportBackupsReport["items"][number];

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
    { key: "backups", header: "Backups", numeric: true, cell: (d) => d.snapshots },
    { key: "logs", header: "Logs", numeric: true, cell: (d) => d.logs },
    { key: "size", header: "Size", numeric: true, cell: (d) => fmtBytes(d.total_bytes) },
    { key: "latest", header: "Latest", optional: true, cell: (d) => fmtWhen(d.latest) },
    {
      key: "export",
      header: <span className="visually-hidden">Export</span>,
      width: "6rem",
      cell: (d) =>
        d.snapshots > 0 && (
          <Button size="sm" variant="ghost" onClick={() => exportDevice(d.device)} disabled={busy} aria-label={`Export ${d.name || d.device}`}>
            Export…
          </Button>
        ),
    },
  ];
  const items: Column<ImportItem>[] = [
    { key: "outcome", header: "Outcome", width: "8rem", cell: (i) => <Badge tone={OUTCOME_TONE[i.outcome]}>{OUTCOME_LABEL[i.outcome]}</Badge> },
    { key: "path", header: "Folder", rowHeader: true, cell: (i) => <span className="selectable">{tilde(i.path, home)}</span> },
    { key: "reason", header: "Reason", cell: (i) => (i.outcome === "skipped" && i.reason) || "" },
  ];

  return (
    <>
      <PageHeader
        id="storage-title"
        title="Storage"
        actions={
          <>
            <Button size="sm" icon="import" onClick={importFolder} disabled={busy}>
              Import backups…
            </Button>
            <Button size="sm" onClick={prune} disabled={busy}>
              Prune now
            </Button>
          </>
        }
      />
      <StatRow label="Summary">
        <Stat label="Total" value={view ? fmtBytes(view.total_bytes) : "—"} />
        <Stat label="Backups" value={view ? view.snapshots : "—"} />
        <Stat label="Stored files" value={view ? fmtBytes(view.blob_bytes) : "—"} sub={view && view.shared_blob_bytes > 0 ? `${fmtBytes(view.shared_blob_bytes)} shared` : undefined} />
        <Stat label="Radio logs" value={view ? fmtBytes(view.log_bytes) : "—"} />
      </StatRow>
      {view && (
        <FactList
          label="Gear folder"
          facts={[
            {
              label: "Folder",
              value: (
                <span className={styles.folder}>
                  <span className="selectable">{tilde(view.gear_dir, home)}</span>
                  <Button size="sm" variant="ghost" icon="finder" onClick={() => void reveal(view.gear_dir).catch((e) => toast(errText(e), true))}>
                    Show in Finder
                  </Button>
                </span>
              ),
            },
          ]}
        />
      )}
      {view && <DataTable label="Storage by device" columns={columns} rows={view.devices} rowKey={(d) => d.device} empty={<EmptyState icon="hdd">No backups yet.</EmptyState>} />}
      {report && (
        <SectionCard title={report.dry_run ? "Import (not run)" : "Import"} label="Import">
          <DataTable label="Import items" columns={items} rows={report.items} rowKey={(i) => i.path} compact />
        </SectionCard>
      )}
    </>
  );
}
