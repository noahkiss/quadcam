import { Button } from "../../../components/Button";
import { Checkbox, Select } from "../../../components/Field";
import { CommitInput } from "../../../components/CommitInput";
import type { ModelDetail, ModelOp } from "../../../ipc/types";
import { COUNTDOWN_BEEPS, PERSISTENT, TIMER_MODES, freeTimer, timerField } from "../../../lib/models";
import styles from "./Models.module.css";

interface Props {
  detail: ModelDetail;
  stage: (ops: ModelOp[]) => Promise<void>;
}

/** The model's timers (1-3): each control stages the change when it changes. */
export function TimersEditor({ detail, stage }: Props) {
  const timers = detail.view.timers;
  const free = freeTimer(timers);
  const set = (index: number, key: string, value: string) => stage([{ op: "set_timer", index, fields: [{ key, value }] }]);
  return (
    <section className={styles.section} aria-label="Timers">
      <h3>Timers</h3>
      {timers.length === 0 && <p className={styles.muted}>This model has no timers.</p>}
      <ul className={styles.rows}>
        {timers.map((t) => {
          const n = t.index + 1;
          const modes = TIMER_MODES.includes(t.mode) ? TIMER_MODES : [...TIMER_MODES, t.mode];
          return (
            <li key={t.index}>
              <fieldset className={styles.row}>
                <legend className={styles.legend}>Timer {n}</legend>
                <label className={styles.field}>
                  Name
                  <CommitInput aria-label={`Timer ${n} name`} maxLength={8} value={t.name} onCommit={(v) => set(t.index, "name", v)} />
                </label>
                <label className={styles.field}>
                  Mode
                  <Select aria-label={`Timer ${n} mode`} value={t.mode} onChange={(e) => set(t.index, "mode", e.target.value)}>
                    {modes.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </Select>
                </label>
                <label className={styles.field}>
                  Switch
                  <CommitInput aria-label={`Timer ${n} switch`} value={t.swtch} onCommit={(v) => set(t.index, "swtch", v)} />
                </label>
                <label className={styles.field}>
                  Countdown
                  <Select aria-label={`Timer ${n} countdown`} value={timerField(t, "countdownBeep")} onChange={(e) => set(t.index, "countdownBeep", e.target.value)}>
                    {COUNTDOWN_BEEPS.map(([v, l]) => (
                      <option key={v} value={v}>
                        {l}
                      </option>
                    ))}
                  </Select>
                </label>
                <label className={styles.field}>
                  Persistent
                  <Select aria-label={`Timer ${n} persistent`} value={t.persistent} onChange={(e) => set(t.index, "persistent", e.target.value)}>
                    {PERSISTENT.map(([v, l]) => (
                      <option key={v} value={v}>
                        {l}
                      </option>
                    ))}
                  </Select>
                </label>
                <Checkbox label="Beep every minute" checked={timerField(t, "minuteBeep") === "1"} onChange={(e) => set(t.index, "minuteBeep", e.target.checked ? "1" : "0")} />
                <Checkbox label="Count up" checked={timerField(t, "showElapsed") === "1"} onChange={(e) => set(t.index, "showElapsed", e.target.checked ? "1" : "0")} />
                <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Remove timer ${n}`} onClick={() => stage([{ op: "remove_timer", index: t.index }])} />
              </fieldset>
            </li>
          );
        })}
      </ul>
      {free != null && (
        <div className={styles.bar}>
          <Button size="sm" icon="add" onClick={() => stage([{ op: "set_timer", index: free, fields: [{ key: "mode", value: "ON" }, { key: "swtch", value: "NONE" }] }])}>
            Add timer
          </Button>
        </div>
      )}
    </section>
  );
}
