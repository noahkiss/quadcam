// Gear > Firmware (design 2.2, 6.5): each saved device's installed firmware against the
// newest release (updates first), Flash… for an EdgeTX radio QuadCam has proven, the
// ExpressLRS section, Read firmware… (a sheet), and the saved firmware copies with Restore…
// behind a disclosure. A check reads the network only when Check for updates is selected,
// or when firmwareCheck is daily.
import { useEffect, useState } from "react";
import { useStore } from "../../../store";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { StatusDot, type StatusTone } from "../../../components/dash/StatusDot";
import type { FirmwareStatus } from "../../../ipc/types";
import { fmtWhen } from "../../../lib/backups";
import { fmtAgo, plural } from "../../../lib/format";
import { ElrsSection } from "./ElrsSection";
import { FirmwareCopies } from "./FirmwareCopies";
import { ReadFirmware } from "./ReadFirmware";
import styles from "./Firmware.module.css";

const STATE_LABEL: Record<FirmwareStatus["state"], string> = {
  up_to_date: "Up to date",
  update: "Update available",
  unknown: "Unknown",
};
const STATE_TONE: Record<FirmwareStatus["state"], StatusTone> = { up_to_date: "ok", update: "warn", unknown: "idle" };

export function FirmwarePage() {
  const view = useStore((s) => s.firmware);
  const load = useStore((s) => s.loadFirmware);
  const openFlash = useStore((s) => s.openFlash);
  const [busy, setBusy] = useState(false);
  const [reading, setReading] = useState(false);

  useEffect(() => {
    void load(null);
  }, [load]);

  const check = async () => {
    setBusy(true);
    await load(true);
    setBusy(false);
  };
  const errors = view?.latest.errors ?? [];
  const checked = view?.latest.checked_at ?? null;
  const devices = [...(view?.devices ?? [])].sort((a, b) => Number(b.state === "update") - Number(a.state === "update"));
  const updates = devices.filter((d) => d.state === "update").length;

  const columns: Column<FirmwareStatus>[] = [
    {
      key: "device",
      header: "Device",
      rowHeader: true,
      cell: (d) => (
        <>
          {d.name}
          {d.board && <span className={styles.muted}> · {d.board}</span>}
        </>
      ),
    },
    { key: "product", header: "Firmware", cell: (d) => d.product, width: "7rem" },
    {
      key: "version",
      header: "Installed → newest",
      numeric: true,
      cell: (d) => (
        <span className="selectable">
          {d.installed ?? "—"}
          {checked && d.latest && d.latest !== d.installed && <span className={d.state === "update" ? styles.newer : undefined}> → {d.latest}</span>}
        </span>
      ),
    },
    {
      key: "status",
      header: "Status",
      cell: (d) => (
        <>
          <StatusDot tone={checked ? STATE_TONE[d.state] : "idle"}>{checked ? STATE_LABEL[d.state] : "Not checked"}</StatusDot>
          {d.note && <span className={styles.note}>{d.note}</span>}
        </>
      ),
    },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "9rem",
      cell: (d) =>
        d.flashable &&
        d.latest && (
          <div className={styles.actions}>
            <Button size="sm" variant="secondary" onClick={() => void openFlash({ device: d.device, version: d.latest, splash: null })}>
              Flash {d.latest}…
            </Button>
          </div>
        ),
    },
  ];

  return (
    <>
      <PageHeader
        id="firmware-title"
        title="Firmware"
        subtitle={
          <span role="status">
            <SummaryLine
              parts={[
                checked ? { text: <span title={fmtWhen(checked)}>Checked {fmtAgo(checked) === "Now" ? "just now" : `${fmtAgo(checked)} ago`}</span> } : "Not checked yet",
                checked && { text: updates ? plural(updates, "update") : "No updates", tone: updates ? "warn" : undefined },
                view?.mode === "daily" && "Checks daily",
              ]}
            />
          </span>
        }
        fold={[
          { label: "Read firmware…", run: () => setReading(true) },
          { label: busy ? "Checking…" : "Check for updates", icon: "refresh", disabled: busy, run: () => void check() },
        ]}
        moreLabel="Firmware actions"
      />
      {errors.map((e) => (
        <Banner key={e} kind="warning" icon="danger-triangle">
          {e}
        </Banner>
      ))}
      {view && (
        <DataTable
          label="Firmware"
          columns={columns}
          rows={devices}
          rowKey={(d) => d.device}
          empty={<EmptyState icon="upload-minimalistic">No saved device runs firmware QuadCam checks.</EmptyState>}
        />
      )}
      <ElrsSection />
      {reading && <ReadFirmware radios={devices.filter((d) => d.kind === "radio")} onClose={() => setReading(false)} />}
      <FirmwareCopies radios={devices.filter((d) => d.kind === "radio")} />
    </>
  );
}
