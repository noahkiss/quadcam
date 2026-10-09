// The Rates segment's editor: change a rate profile's model, rates, limits, throttle and name
// with the curves redrawing as you type. Nothing is written here: "Stage" queues the changed
// values as one change (WP5), and the apply sheet writes it. "Convert" fits the profile onto
// another rate model with the core's least-squares fit, and shows how far the new curve is
// from the old one.
import { useEffect, useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Input, Select } from "../../../components/Field";
import { api, errText } from "../../../ipc/api";
import type { RateProfile, StagedChange } from "../../../ipc/types";
import { AXIS_LABEL, LIMIT_LABEL, MODEL_BOUNDS, MODEL_FIELDS, RATES_TYPE, RATE_MODELS, deg, editTitle, profileEdits, profileLabel, wholeNumber } from "../../../lib/rates";
import { useStore } from "../../../store";
import styles from "./Rates.module.css";

interface Props {
  /** The profile as the FC's latest backup holds it. */
  profile: RateProfile;
  /** The FC's device id: the change is staged for it. */
  device: string;
  /** Called with the edited profile (curves redrawn by the core), or null when editing ends. */
  onDraft: (p: RateProfile | null) => void;
  onClose: () => void;
}

const DEBOUNCE_MS = 120;

export function RateEditor({ profile, device, onDraft, onClose }: Props) {
  const [draft, setDraft] = useState<RateProfile>(profile);
  const [fit, setFit] = useState<{ to: string; error: number[]; share: number[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [staged, setStaged] = useState<StagedChange | null>(null);
  const [busy, setBusy] = useState(false);
  const seq = useRef(0);
  const open = useStore((s) => s.openApply);
  const edits = profileEdits(profile, draft);

  // Redraw the curves from the core while the numbers change.
  useEffect(() => {
    const mine = ++seq.current;
    const t = setTimeout(() => {
      api.gearRatesPreview(draft).then(
        (r) => {
          if (mine !== seq.current) return;
          setError(null);
          onDraft(r.profile);
        },
        (e) => mine === seq.current && setError(errText(e)),
      );
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft]);

  const change = (f: (d: RateProfile) => void) => {
    setStaged(null);
    setFit(null);
    setDraft((d) => {
      const next = structuredClone(d);
      f(next);
      return next;
    });
  };

  const convert = async (to: string) => {
    if (!to || to === draft.rates_type) return;
    setBusy(true);
    try {
      const r = await api.gearRatesPreview(draft, to);
      setDraft({ ...draft, rates_type: r.profile.rates_type, axes: r.profile.axes });
      setFit({ to, error: r.fit_error, share: r.fit_share });
      setStaged(null);
      setError(null);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const stage = async () => {
    setBusy(true);
    try {
      const c = await api.gearChangeStage(device, edits, editTitle(profile, edits));
      await useStore.getState().loadGear();
      setStaged(c);
      setError(null);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const [rcLabel, srLabel, exLabel] = MODEL_FIELDS[draft.rates_type] ?? MODEL_FIELDS.betaflight;
  const [rcB, srB, exB] = MODEL_BOUNDS[draft.rates_type] ?? MODEL_BOUNDS.betaflight;
  const num = (label: string, value: number, bounds: [number, number], set: (d: RateProfile, v: number) => void) => (
    <Input
      type="number"
      inputMode="numeric"
      aria-label={label}
      min={bounds[0]}
      max={bounds[1]}
      step={1}
      value={Number.isFinite(value) ? value : ""}
      onChange={(e) => {
        const v = wholeNumber(e.target.value, bounds[0], bounds[1]);
        if (v !== null) change((d) => set(d, v));
      }}
    />
  );

  return (
    <section className={styles.editor} aria-label="Edit rate profile">
      <h3 className={styles.title}>Edit {profileLabel(profile)}</h3>
      <div className={styles.editorBar}>
        <label className={styles.compareLabel}>
          Name
          <Input aria-label="Profile name" maxLength={8} value={draft.name ?? ""} onChange={(e) => change((d) => void (d.name = e.target.value))} />
        </label>
        <label className={styles.compareLabel}>
          Rates type
          <Select aria-label="Rates type" value={draft.rates_type} onChange={(e) => change((d) => void (d.rates_type = e.target.value))}>
            {RATE_MODELS.map((m) => (
              <option key={m} value={m}>
                {RATES_TYPE[m]}
              </option>
            ))}
          </Select>
        </label>
        <label className={styles.compareLabel}>
          Convert to
          <Select aria-label="Convert to" value="" disabled={busy} onChange={(e) => void convert(e.target.value)}>
            <option value="">Pick a model…</option>
            {RATE_MODELS.filter((m) => m !== draft.rates_type).map((m) => (
              <option key={m} value={m}>
                {RATES_TYPE[m]}
              </option>
            ))}
          </Select>
        </label>
      </div>
      <p className={styles.meta}>Changing the type keeps the numbers. Convert fits the curve onto the other type instead.</p>
      {fit && (
        <Banner kind="info" icon="info">
          Fitted onto {RATES_TYPE[fit.to] ?? fit.to}. Largest gap from the old curve: {fit.error.map((e, i) => `${AXIS_LABEL[draft.axes[i].axis] ?? draft.axes[i].axis} ${deg(e)}`).join(", ")}.
        </Banner>
      )}
      <table className={styles.table} aria-label="Rates to edit">
        <thead>
          <tr>
            <th scope="col">Axis</th>
            <th scope="col">{rcLabel}</th>
            <th scope="col">{srLabel}</th>
            <th scope="col">{exLabel}</th>
            <th scope="col">Limit</th>
          </tr>
        </thead>
        <tbody>
          {draft.axes.map((a, i) => {
            const axis = AXIS_LABEL[a.axis] ?? a.axis;
            return (
              <tr key={a.axis}>
                <th scope="row">{axis}</th>
                <td>{num(`${axis} ${rcLabel}`, a.rc_rate, rcB, (d, v) => void (d.axes[i].rc_rate = v))}</td>
                <td>{num(`${axis} ${srLabel}`, a.srate, srB, (d, v) => void (d.axes[i].srate = v))}</td>
                <td>{num(`${axis} ${exLabel}`, a.expo, exB, (d, v) => void (d.axes[i].expo = v))}</td>
                <td>{num(`${axis} limit`, a.rate_limit, [200, 1998], (d, v) => void (d.axes[i].rate_limit = v))}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className={styles.editorBar}>
        <label className={styles.compareLabel}>
          Throttle mid
          {num("Throttle mid", draft.throttle.mid, [0, 100], (d, v) => void (d.throttle.mid = v))}
        </label>
        <label className={styles.compareLabel}>
          Throttle expo
          {num("Throttle expo", draft.throttle.expo, [0, 100], (d, v) => void (d.throttle.expo = v))}
        </label>
        <label className={styles.compareLabel}>
          Throttle limit
          <Select aria-label="Throttle limit" value={draft.throttle.limit} onChange={(e) => change((d) => void (d.throttle.limit = e.target.value))}>
            {Object.entries(LIMIT_LABEL).map(([k, v]) => (
              <option key={k} value={k}>
                {v}
              </option>
            ))}
          </Select>
        </label>
        <label className={styles.compareLabel}>
          Limit percent
          {num("Throttle limit percent", draft.throttle.limit_percent, [25, 100], (d, v) => void (d.throttle.limit_percent = v))}
        </label>
      </div>
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {staged && (
        <Banner kind="info" icon="check-circle">
          Staged: {staged.title}. Nothing is written until you apply it.
        </Banner>
      )}
      <div className={styles.editorBar}>
        {staged ? (
          <Button variant="primary" onClick={() => void open(device, staged.id)}>
            Review and apply…
          </Button>
        ) : (
          <Button variant="primary" disabled={!edits.length || busy || !!error} onClick={() => void stage()}>
            Stage {edits.length ? `${edits.length} ${edits.length === 1 ? "change" : "changes"}` : "changes"}
          </Button>
        )}
        <Button
          variant="ghost"
          onClick={() => {
            onDraft(null);
            onClose();
          }}
        >
          {staged ? "Done" : "Cancel"}
        </Button>
      </div>
    </section>
  );
}
