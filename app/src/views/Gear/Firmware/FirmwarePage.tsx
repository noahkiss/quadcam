// Gear > Firmware (design 2.2, 6.5): each saved device's installed firmware against the
// newest release (updates first), Flash… for an EdgeTX radio QuadCam has proven, the
// ExpressLRS section, Read firmware… (a sheet), and the saved firmware copies with Restore…
// behind a disclosure. A check reads the network only when Check for updates is selected,
// or when firmwareCheck is daily. An FC in its maker's channel shows the maker's build and
// newest file, with official Betaflight as an information line.
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

/** The maker an FC's firmware comes from, when it is in the maker's channel. */
export const vendorOf = (d: FirmwareStatus) => (d.channel === "vendor" ? (d.vendor ?? null) : null);

/** The installed version, with the maker and build day for a maker's build: `2025.12.5-alpha (BetaFPV, Jun 25 2026)`. */
export function installedLabel(d: FirmwareStatus) {
  const vendor = vendorOf(d);
  if (!d.installed) return "—";
  if (!vendor) return d.installed;
  return `${d.installed} (${[vendor, d.installed_date].filter(Boolean).join(", ")})`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** `2026-06-25` as Betaflight prints a build day: `Jun 25 2026`. */
export function buildDay(iso: string | null | undefined) {
  const m = iso?.match(/^(\d{4})-(\d{2})-(\d{2})/);
  return m ? `${MONTHS[Number(m[2]) - 1]} ${Number(m[3])} ${m[1]}` : null;
}

/** The newest version, with the maker's build day for a maker's file. */
export function newestLabel(d: FirmwareStatus) {
  const vendor = vendorOf(d);
  const day = buildDay(d.vendor_file?.build?.date);
  return vendor && day ? `${d.latest} (${vendor}, ${day})` : (d.latest ?? "—");
}

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
          {installedLabel(d)}
          {checked && d.latest && (d.latest !== d.installed || d.state === "update") && <span className={d.state === "update" ? styles.newer : undefined}> → {newestLabel(d)}</span>}
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
          {d.channel_note && <span className={styles.note}>{d.channel_note}</span>}
          {checked && vendorOf(d) && d.official && (
            <span className={styles.note}>
              Official Betaflight {d.official} is not {vendorOf(d)}'s build for this quad.
            </span>
          )}
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
