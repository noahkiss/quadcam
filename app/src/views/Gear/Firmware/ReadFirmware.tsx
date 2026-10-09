// Gear > Firmware > Read firmware: the read-only DFU trial. It reads the flash of a radio in
// DFU mode twice, saves a copy and compares the version the image names with the radio's
// known version. It cannot erase, write or restart the radio.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { api } from "../../../ipc/api";
import { on } from "../../../ipc/events";
import type { FirmwareRead, FirmwareStatus } from "../../../ipc/types";
import styles from "./Firmware.module.css";

interface Props {
  radios: FirmwareStatus[];
}

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
const NONE = "";

export function ReadFirmware({ radios }: Props) {
  const [pick, setPick] = useState<string | null>(null);
  const device = pick ?? radios[0]?.device ?? NONE;
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [result, setResult] = useState<FirmwareRead | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void on("firmware-read-progress", (p) => setProgress(p.total > 0 ? p.done / p.total : 0)).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  const read = async () => {
    setBusy(true);
    setProgress(0);
    setResult(null);
    setError(null);
    try {
      setResult(await api.gearFirmwareRead(device === NONE ? null : device));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className={styles.read} aria-labelledby="read-firmware-title">
      <h3 id="read-firmware-title" className={styles.subtitle}>
        Read radio firmware
      </h3>
      <p className={styles.muted}>Saves a copy of the firmware on the radio. It does not change the radio.</p>
      <ol className={styles.steps} aria-label="Steps">
        <li>Turn the radio off.</li>
        <li>Plug the radio into this Mac with the USB cable. Do not hold any buttons.</li>
        <li>Wait a few seconds.</li>
        <li>Select Read firmware.</li>
      </ol>
      <div className={styles.bar}>
        {radios.length > 0 && (
          <label className={styles.pick}>
            Radio
            <select value={device} onChange={(e) => setPick(e.target.value)} disabled={busy}>
              {radios.map((r) => (
                <option key={r.device} value={r.device}>
                  {r.name}
                </option>
              ))}
              <option value={NONE}>Other radio</option>
            </select>
          </label>
        )}
        <Button variant="secondary" disabled={busy} onClick={() => void read()}>
          {busy ? "Reading…" : "Read firmware"}
        </Button>
        {busy && (
          <progress className={styles.progress} aria-label="Reading the radio" max={100} value={Math.round(progress * 100)} />
        )}
      </div>
      {error && (
        <Banner kind="warning" icon="danger-triangle">
          {error}
        </Banner>
      )}
      {result && (
        <div role="region" aria-label="Firmware copy" className={styles.result}>
          <Banner kind={result.matches === false ? "warning" : "info"} icon={result.matches === false ? "danger-triangle" : "check-circle"}>
            {result.message}
          </Banner>
          <ul className={styles.steps} aria-label="Read steps">
            {result.steps.map((s) => (
              <li key={s.name}>
                {s.name}: done{s.detail ? ` (${s.detail})` : ""}
              </li>
            ))}
          </ul>
          <p className={`${styles.muted} mono selectable`}>
            {result.copy.id} · {Math.round(result.copy.size / 1024)} KB · SHA-256 {result.copy.sha256}
          </p>
        </div>
      )}
    </section>
  );
}
