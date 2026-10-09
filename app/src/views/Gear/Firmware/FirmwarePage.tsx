// Gear > Firmware (design 2.2, 6.5): each saved device's installed firmware against the
// newest release, and Flash… for an EdgeTX radio QuadCam has proven. A check reads the
// network only when Check for updates is selected, or when firmwareCheck is daily.
import { useEffect, useState } from "react";
import { useStore } from "../../../store";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import type { FirmwareStatus } from "../../../ipc/types";
import { fmtWhen } from "../../../lib/backups";
import { ReadFirmware } from "./ReadFirmware";
import styles from "./Firmware.module.css";

const STATE_LABEL: Record<FirmwareStatus["state"], string> = {
  up_to_date: "Up to date",
  update: "Update available",
  unknown: "Unknown",
};

export function FirmwarePage() {
  const view = useStore((s) => s.firmware);
  const load = useStore((s) => s.loadFirmware);
  const openFlash = useStore((s) => s.openFlash);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void load(null);
  }, [load]);

  const check = async () => {
    setBusy(true);
    await load(true);
    setBusy(false);
  };
  const errors = view?.latest.errors ?? [];
  const checked = view?.latest.checked_at;

  return (
    <div className={styles.page} aria-labelledby="firmware-title">
      <h2 id="firmware-title" className={styles.title}>
        Firmware
      </h2>
      <div className={styles.bar}>
        <Button variant="secondary" icon="refresh" disabled={busy} onClick={() => void check()}>
          {busy ? "Checking…" : "Check for updates"}
        </Button>
        <span className={styles.muted} role="status">
          {checked ? `Checked ${fmtWhen(checked)}.` : "Not checked yet."}
        </span>
      </div>
      {errors.map((e) => (
        <Banner key={e} kind="warning" icon="danger-triangle">
          {e}
        </Banner>
      ))}
      {view && view.devices.length === 0 && <p className={styles.muted}>No saved device runs firmware QuadCam checks.</p>}
      {view && view.devices.length > 0 && (
        <table className={styles.table} aria-label="Firmware">
          <thead>
            <tr>
              <th scope="col">Device</th>
              <th scope="col">Firmware</th>
              <th scope="col">Installed</th>
              <th scope="col">Newest</th>
              <th scope="col">Status</th>
              <th scope="col">
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {view.devices.map((d) => (
              <tr key={d.device}>
                <th scope="row">
                  {d.name}
                  {d.board && <span className={styles.muted}> · {d.board}</span>}
                </th>
                <td>{d.product}</td>
                <td className="mono selectable">{d.installed ?? "Unknown"}</td>
                <td className="mono selectable">{d.latest ?? "Unknown"}</td>
                <td data-state={d.state}>
                  {STATE_LABEL[d.state]}
                  {d.note && <span className={styles.note}>{d.note}</span>}
                </td>
                <td className={styles.actions}>
                  {d.flashable && d.latest && (
                    <Button variant="secondary" onClick={() => void openFlash({ device: d.device, version: d.latest, splash: null })}>
                      Flash {d.latest}…
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <ReadFirmware radios={(view?.devices ?? []).filter((d) => d.kind === "radio")} />
    </div>
  );
}
