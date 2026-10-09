// Copy settings between quads: pick the FC to copy from and the FC that gets the settings,
// the parts to copy (rates, PID profiles, OSD, modes, ...) and any settings by name. The
// core's plan shows the compatibility checks and the diff; Stage queues one change on the
// target. Nothing is written to a quad until that change is applied from the apply sheet.
import { useEffect, useState } from "react";
import { api, errText } from "../../../ipc/api";
import type { CopyPart, CopyPlan } from "../../../ipc/types";
import { useStore } from "../../../store";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Checkbox, SelectField, TextField } from "../../../components/Field";
import { ChecksList } from "../../../components/gear/ChecksList";
import { DiffView } from "../../../components/gear/DiffView";
import { deviceName } from "../../../lib/gear";
import styles from "./Bench.module.css";

const PARTS: { id: CopyPart; label: string }[] = [
  { id: "rates", label: "Rates" },
  { id: "pid", label: "PID profiles" },
  { id: "osd", label: "OSD" },
  { id: "modes", label: "Modes" },
  { id: "adjustments", label: "Adjustments" },
  { id: "vtx", label: "VTX" },
  { id: "features", label: "Features and beeper" },
];

export function CopyDialog({ onClose, to: toInit }: { onClose: () => void; to?: string }) {
  const fcs = useStore((s) => s.devices).filter((d) => d.kind === "fc");
  const loadGear = useStore((s) => s.loadGear);
  const [to, setTo] = useState(toInit || fcs[0]?.id || "");
  const [from, setFrom] = useState(fcs.find((d) => d.id !== (toInit || fcs[0]?.id))?.id || "");
  const [parts, setParts] = useState<CopyPart[]>([]);
  const [names, setNames] = useState("");
  const [plan, setPlan] = useState<CopyPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const settings = names
    .split(/[\s,]+/)
    .map((x) => x.trim())
    .filter(Boolean);
  const key = JSON.stringify([from, to, parts, settings]);

  // The plan follows the picks.
  useEffect(() => {
    let live = true;
    setPlan(null);
    setError(null);
    if (!from || !to || from === to || (!parts.length && !settings.length)) return;
    api
      .gearCopyPlan({ from, to, parts, settings })
      .then((p) => live && setPlan(p))
      .catch((e) => live && setError(errText(e)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const close = async (v: string) => {
    if (v !== "stage") return onClose();
    try {
      await api.gearCopyStage({ from, to, parts, settings });
      await loadGear();
      onClose();
    } catch (e) {
      setError(errText(e));
    }
  };
  const ready = !!plan && plan.checks.every((c) => c.ok) && plan.edits.length > 0;
  return (
    <Dialog
      open
      title="Copy settings"
      onClose={close}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="stage" variant="primary" disabled={!ready}>
            Stage
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <SelectField label="Copy from" value={from} onChange={(e) => setFrom(e.target.value)}>
          <option value="">Pick an FC</option>
          {fcs.map((d) => (
            <option key={d.id} value={d.id} disabled={d.id === to || !d.last_backup}>
              {deviceName(d)}
              {d.last_backup ? "" : " (no backup)"}
            </option>
          ))}
        </SelectField>
        <SelectField label="Copy to" value={to} onChange={(e) => setTo(e.target.value)}>
          {fcs.map((d) => (
            <option key={d.id} value={d.id} disabled={d.id === from || !d.last_backup}>
              {deviceName(d)}
              {d.last_backup ? "" : " (no backup)"}
            </option>
          ))}
        </SelectField>
        <fieldset className={styles.parts}>
          <legend>What to copy</legend>
          {PARTS.map((p) => (
            <Checkbox key={p.id} label={p.label} checked={parts.includes(p.id)} onChange={(e) => setParts(e.target.checked ? [...parts, p.id] : parts.filter((x) => x !== p.id))} />
          ))}
        </fieldset>
        <TextField label="Settings by name" mono placeholder="osd_cap_alarm, p_roll" value={names} onChange={(e) => setNames(e.target.value)} hint="Separate names with commas or spaces." />
        {error && (
          <p role="alert" className={styles.error}>
            {error}
          </p>
        )}
        {plan && (
          <>
            <section aria-label="Checks">
              <ChecksList checks={plan.checks} />
            </section>
            {plan.diff.length > 0 && (
              <section aria-label="Changes">
                <DiffView items={[{ kind: "lines", label: `Changes for ${deviceName(fcs.find((d) => d.id === to)!)}`, lines: plan.diff }]} />
              </section>
            )}
            {plan.same > 0 && <p className={styles.muted}>{plan.same === 1 ? "1 setting is already the same." : `${plan.same} settings are already the same.`}</p>}
            {plan.notes.map((n) => (
              <p key={n} className={styles.muted}>
                {n}
              </p>
            ))}
            {plan.skipped.length > 0 && (
              <ul aria-label="Left out" className={styles.skipped}>
                {plan.skipped.map((s) => (
                  <li key={s}>{s}</li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
}
