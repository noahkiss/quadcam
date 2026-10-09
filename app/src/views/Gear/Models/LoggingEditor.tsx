import { Checkbox } from "../../../components/Field";
import { CommitInput } from "../../../components/CommitInput";
import type { ModelDetail, ModelOp } from "../../../ipc/types";
import styles from "./Models.module.css";

interface Props {
  detail: ModelDetail;
  stage: (ops: ModelOp[]) => Promise<void>;
}

/** Logging (the switch that writes the radio's log, how often, which sensors) and the RSSI alarms. */
export function LoggingEditor({ detail, stage }: Props) {
  const { logging, sensors } = detail.editors.logging;
  const rf = detail.editors.rf_alarms;
  const setLog = (swtch: string, period_ds: number) => stage([{ op: "set_logging", logging: { swtch, period_ds } }]);
  return (
    <>
      <section className={styles.section} aria-label="Logging">
        <h3>Logging</h3>
        <div className={styles.row}>
          <Checkbox label="Write a log" checked={!!logging} onChange={(e) => stage([{ op: "set_logging", logging: e.target.checked ? { swtch: "ON", period_ds: 10 } : null }])} />
          {logging && (
            <>
              <label className={styles.field}>
                Log switch
                <CommitInput aria-label="Log switch" value={logging.swtch} onCommit={(v) => setLog(v.trim(), logging.period_ds)} />
              </label>
              <label className={styles.field}>
                Log every (seconds)
                <CommitInput
                  aria-label="Log every (seconds)"
                  inputMode="decimal"
                  value={String(logging.period_ds / 10)}
                  onCommit={(v) => {
                    const s = Number(v);
                    if (!Number.isNaN(s)) setLog(logging.swtch, Math.round(s * 10));
                  }}
                />
              </label>
            </>
          )}
        </div>
        <fieldset className={styles.row}>
          <legend className={styles.legend}>Sensors in the log</legend>
          {sensors.map((s) => (
            <Checkbox key={s.label} label={s.label} checked={s.logs} onChange={(e) => stage([{ op: "set_sensor_logs", sensors: [{ label: s.label, logs: e.target.checked }] }])} />
          ))}
        </fieldset>
      </section>
      <section className={styles.section} aria-label="Alarms">
        <h3>Alarms</h3>
        {rf ? (
          <div className={styles.row}>
            <label className={styles.field}>
              RSSI warning
              <CommitInput aria-label="RSSI warning" inputMode="numeric" value={String(rf.warning)} onCommit={(v) => stage([{ op: "set_rf_alarms", warning: Number(v), critical: rf.critical }])} />
            </label>
            <label className={styles.field}>
              RSSI critical
              <CommitInput aria-label="RSSI critical" inputMode="numeric" value={String(rf.critical)} onCommit={(v) => stage([{ op: "set_rf_alarms", warning: rf.warning, critical: Number(v) }])} />
            </label>
          </div>
        ) : (
          <p className={styles.muted}>This model file has no RSSI alarm levels.</p>
        )}
      </section>
    </>
  );
}
