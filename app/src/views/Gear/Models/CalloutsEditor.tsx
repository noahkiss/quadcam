import { useState } from "react";
import { Button } from "../../../components/Button";
import { Input, Select } from "../../../components/Field";
import type { CalloutView, ModelDetail, ModelOp } from "../../../ipc/types";
import { calloutOp, formOf, repeatText, whenText, type CalloutForm } from "../../../lib/models";
import styles from "./Models.module.css";

interface Props {
  detail: ModelDetail;
  stage: (ops: ModelOp[]) => Promise<void>;
}

/** Spoken callouts: a sound that plays while a switch is on or a sensor is below or above a value. */
export function CalloutsEditor({ detail, stage }: Props) {
  const sensors = detail.view.sensors.map((s) => s.label);
  const [form, setForm] = useState<CalloutForm | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const callouts = detail.editors.callouts;
  const open = (c: CalloutView | null) => {
    setForm(formOf(c, sensors));
    setProblem(null);
  };
  const submit = async () => {
    if (!form) return;
    const r = calloutOp(form);
    if ("error" in r) return setProblem(r.error);
    await stage([r.op]);
    setForm(null);
  };
  const set = (patch: Partial<CalloutForm>) => form && setForm({ ...form, ...patch });
  return (
    <section className={styles.section} aria-label="Callouts">
      <h3>Callouts</h3>
      {callouts.length === 0 && <p className={styles.muted}>This model has no callouts.</p>}
      <ul className={styles.rows}>
        {callouts.map((c) => (
          <li key={c.track} className={styles.row}>
            <span className={styles.grow}>
              <strong>{c.track}</strong> {whenText(c)}, {repeatText(c.repeat)}
            </span>
            <Button size="sm" icon="pen" disabled={!c.when} aria-label={`Edit callout ${c.track}`} onClick={() => open(c)} />
            <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Remove callout ${c.track}`} onClick={() => stage([{ op: "remove_callout", track: c.track }])} />
          </li>
        ))}
      </ul>
      {form ? (
        <form
          className={styles.form}
          aria-label="Callout"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <div className={styles.row}>
            <label className={styles.field}>
              Sound
              <Input aria-label="Sound" list="callout-tracks" maxLength={8} value={form.track} onChange={(e) => set({ track: e.target.value })} />
            </label>
            <datalist id="callout-tracks">
              {detail.tracks.map((t) => (
                <option key={t} value={t} />
              ))}
            </datalist>
            <label className={styles.field}>
              Plays
              <Select aria-label="Plays" value={form.kind} onChange={(e) => set({ kind: e.target.value as CalloutForm["kind"] })}>
                <option value="below">When a sensor is below</option>
                <option value="above">When a sensor is above</option>
                <option value="switch">While a switch is on</option>
              </Select>
            </label>
            {form.kind === "switch" ? (
              <label className={styles.field}>
                Switch
                <Input aria-label="Switch" value={form.swtch} onChange={(e) => set({ swtch: e.target.value })} />
              </label>
            ) : (
              <>
                <label className={styles.field}>
                  Sensor
                  <Select aria-label="Sensor" value={form.source} onChange={(e) => set({ source: e.target.value })}>
                    {sensors.map((s) => (
                      <option key={s} value={`{${s}}`}>
                        {s}
                      </option>
                    ))}
                  </Select>
                </label>
                <label className={styles.field}>
                  Value
                  <Input aria-label="Value" inputMode="decimal" value={form.value} onChange={(e) => set({ value: e.target.value })} />
                </label>
                <label className={styles.field}>
                  Delay (seconds)
                  <Input aria-label="Delay (seconds)" inputMode="decimal" value={form.delay} onChange={(e) => set({ delay: e.target.value })} />
                </label>
              </>
            )}
            <label className={styles.field}>
              Repeat
              <Select aria-label="Repeat" value={form.repeat} onChange={(e) => set({ repeat: e.target.value as CalloutForm["repeat"] })}>
                <option value="1x">Once</option>
                <option value="!1x">Once, not at power-on</option>
                <option value="every">Every few seconds</option>
              </Select>
            </label>
            {form.repeat === "every" && (
              <label className={styles.field}>
                Seconds
                <Input aria-label="Seconds between repeats" inputMode="numeric" value={form.every} onChange={(e) => set({ every: e.target.value })} />
              </label>
            )}
          </div>
          {problem && (
            <p className={styles.error} role="alert">
              {problem}
            </p>
          )}
          <div className={styles.bar}>
            <Button type="submit" variant="primary" size="sm">
              Stage callout
            </Button>
            <Button size="sm" onClick={() => setForm(null)}>
              Cancel
            </Button>
          </div>
        </form>
      ) : (
        <div className={styles.bar}>
          <Button size="sm" icon="add" onClick={() => open(null)}>
            Add callout
          </Button>
        </div>
      )}
    </section>
  );
}
