// Gear > Firmware > ExpressLRS (preview, design 6.4): read the module behind a radio or the
// receiver behind an FC, stage option changes (applied from the sheet like any change), and
// flash an official release with the esptool module. Off until the elrsPreview setting is on.
import { useCallback, useEffect, useState } from "react";
import { useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Checkbox, SelectField, TextField } from "../../../components/Field";
import { ChecksList } from "../../../components/gear/ChecksList";
import { DiffView, type DiffEntry } from "../../../components/gear/DiffView";
import { toast } from "../../../components/toastStore";
import { fmtWhen } from "../../../lib/backups";
import type { ApplyPlan, ApplyReport, ElrsDeviceView, ElrsFlashParams, ElrsView } from "../../../ipc/types";
import styles from "./Elrs.module.css";

interface FlashSheet {
  params: ElrsFlashParams;
  name: string;
  plan: ApplyPlan | null;
  report: ApplyReport | null;
  busy: boolean;
  error: string | null;
}

export function ElrsSection() {
  const preview = useStore((s) => s.values.elrsPreview === true);
  const phraseSet = useStore((s) => !!s.values.elrsBindingPhrase);
  const saveSetting = useStore((s) => s.saveSetting);
  const loadGear = useStore((s) => s.loadGear);
  const openApply = useStore((s) => s.openApply);
  const [view, setView] = useState<ElrsView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [phrase, setPhrase] = useState("");
  const [sheet, setSheet] = useState<FlashSheet | null>(null);

  const load = useCallback(async () => {
    try {
      setView(await api.gearElrs(null));
    } catch (e) {
      toast(errText(e), true);
    }
  }, []);
  useEffect(() => {
    if (preview) void load();
  }, [preview, load]);

  const read = async (host: string) => {
    setBusy(host);
    try {
      const r = await api.gearElrsRead(host);
      setNotes(r.notes);
      await load();
      await loadGear();
    } catch (e) {
      toast(errText(e), true);
    }
    setBusy(null);
  };

  const stage = async (d: ElrsDeviceView) => {
    const options = Object.entries(draft)
      .filter(([k, v]) => v !== "" && v !== d.snapshot.options.find((o) => o.key === k)?.value)
      .map(([option, value]) => ({ option, value }));
    if (!options.length) return;
    try {
      await api.gearChangeStage(d.snapshot.device, [{ kind: "elrs_options", options }]);
      setOpen(null);
      setDraft({});
      await loadGear();
      await load();
      await openApply(d.snapshot.device);
    } catch (e) {
      toast(errText(e), true);
    }
  };

  const plan = async (d: ElrsDeviceView) => {
    const params: ElrsFlashParams = { device: d.snapshot.device, version: view?.latest ?? null, sha256: null, port: null };
    setSheet({ params, name: d.name, plan: null, report: null, busy: true, error: null });
    try {
      const p = await api.gearElrsFlashPlan(params);
      setSheet((s) => (s?.params === params ? { ...s, plan: p, busy: false } : s));
    } catch (e) {
      setSheet((s) => (s?.params === params ? { ...s, busy: false, error: errText(e) } : s));
    }
  };

  const flash = async () => {
    if (!sheet?.plan || sheet.busy) return;
    const { params, plan: p } = sheet;
    setSheet({ ...sheet, busy: true, error: null });
    try {
      const report = await api.gearElrsFlashClick(params, p.digest);
      setSheet((s) => (s ? { ...s, report, busy: false } : s));
    } catch (e) {
      setSheet((s) => (s ? { ...s, busy: false, error: errText(e) } : s));
    }
    await load();
    await loadGear();
  };

  const savePhrase = async () => {
    await saveSetting("elrsBindingPhrase", phrase);
    setPhrase("");
    await load();
  };

  const ready = !!sheet?.plan && sheet.plan.checks.every((c) => c.ok);
  return (
    <section className={styles.section} aria-labelledby="elrs-title">
      <h3 id="elrs-title" className={styles.heading}>
        ExpressLRS (preview)
      </h3>
      <Checkbox label="Show the ELRS tools" checked={preview} onChange={(e) => void saveSetting("elrsPreview", e.target.checked)} />
      {preview && (
        <>
          <Banner icon="info">QuadCam has not tried these on a real radio, FC or ExpressLRS device yet.</Banner>
          {notes.map((n) => (
            <Banner key={n} kind="warning" icon="danger-triangle">
              {n}
            </Banner>
          ))}
          {(view?.warnings ?? []).map((w) => (
            <Banner key={w} kind="warning" icon="danger-triangle">
              {w}
            </Banner>
          ))}
          <div className={styles.bar}>
            {(view?.hosts ?? []).map((h) => (
              <Button key={h.device} variant="secondary" disabled={busy !== null} onClick={() => void read(h.device)}>
                {busy === h.device ? "Reading…" : h.kind === "radio" ? `Read the module in ${h.name}` : `Read the receiver in ${h.name}`}
              </Button>
            ))}
            {view && view.hosts.length === 0 && <span className={styles.muted}>Save a radio or an FC first.</span>}
          </div>
          {view && view.devices.length === 0 && <p className={styles.muted}>No ELRS device read yet.</p>}
          {view && view.devices.length > 0 && (
            <table className={styles.table} aria-label="ExpressLRS devices">
              <thead>
                <tr>
                  <th scope="col">Device</th>
                  <th scope="col">Target</th>
                  <th scope="col">Installed</th>
                  <th scope="col">Newest</th>
                  <th scope="col">Read</th>
                  <th scope="col">
                    <span className="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {view.devices.map((d) => (
                  <ElrsRow
                    key={d.snapshot.device}
                    d={d}
                    latest={view.latest}
                    open={open === d.snapshot.device}
                    draft={draft}
                    setDraft={setDraft}
                    onOpen={() => {
                      setDraft({});
                      setOpen(open === d.snapshot.device ? null : d.snapshot.device);
                    }}
                    onStage={() => void stage(d)}
                    onFlash={() => void plan(d)}
                  />
                ))}
              </tbody>
            </table>
          )}
          <div className={styles.phrase}>
            <TextField label="Binding phrase" type="password" autoComplete="off" value={phrase} placeholder={phraseSet || view?.phrase_set ? "(set)" : ""} onChange={(e) => setPhrase(e.target.value)} />
            <Button variant="secondary" disabled={!phrase.trim()} onClick={() => void savePhrase()}>
              Save phrase
            </Button>
            <SelectField label="Region" value={view?.region ?? "FCC"} onChange={(e) => void saveSetting("elrsRegion", e.target.value).then(load)}>
              <option value="FCC">FCC</option>
              <option value="LBT">LBT</option>
            </SelectField>
            <span className={styles.muted}>esptool: {view?.esptool ?? "not installed (Settings > Modules)"}</span>
          </div>
        </>
      )}
      <Dialog
        open={!!sheet}
        kind="sheet"
        blockReturn
        blockEscape={!!sheet?.busy && !sheet.report}
        title={`Flash ${sheet?.name ?? ""}`}
        onClose={() => setSheet(null)}
        actions={
          sheet?.report ? (
            <Button variant="primary" onClick={() => setSheet(null)}>
              Done
            </Button>
          ) : (
            <>
              <Button type="submit" value="cancel" variant="ghost" disabled={!!sheet?.busy && !!sheet.plan}>
                Cancel
              </Button>
              <Button variant="primary" disabled={!ready || !!sheet?.busy} onClick={() => void flash()}>
                Apply
              </Button>
            </>
          )
        }
      >
        {sheet && (
          <div className={styles.result}>
            {sheet.error && <Banner kind="error">{sheet.error}</Banner>}
            {sheet.plan && !sheet.report && (
              <>
                <DiffView items={sheet.plan.diff as DiffEntry[]} />
                <ChecksList checks={sheet.plan.checks} />
                <ul className={styles.warnings}>
                  {(sheet.plan.warnings ?? []).map((w) => (
                    <li key={w}>{w}</li>
                  ))}
                </ul>
              </>
            )}
            {sheet.busy && !sheet.report && sheet.plan && <p role="status">Flashing…</p>}
            {sheet.report && (
              <section aria-label="Result">
                <p>{sheet.report.message}</p>
                <ol className={styles.steps} aria-label="Steps">
                  {sheet.report.steps.map((s) => (
                    <li key={s.name} data-state={s.state}>
                      {s.name}
                      <span className="visually-hidden"> {s.state}</span>
                      {s.detail ? ` — ${s.detail}` : ""}
                    </li>
                  ))}
                </ol>
              </section>
            )}
          </div>
        )}
      </Dialog>
    </section>
  );
}

function ElrsRow({
  d,
  latest,
  open,
  draft,
  setDraft,
  onOpen,
  onStage,
  onFlash,
}: {
  d: ElrsDeviceView;
  latest: string | null;
  open: boolean;
  draft: Record<string, string>;
  setDraft: (f: (x: Record<string, string>) => Record<string, string>) => void;
  onOpen: () => void;
  onStage: () => void;
  onFlash: () => void;
}) {
  const s = d.snapshot;
  return (
    <>
      <tr>
        <th scope="row">
          {d.name}
          <span className={styles.muted}> · {s.role === "tx" ? "Transmitter" : "Receiver"}</span>
          {d.staged > 0 && <span className={styles.muted}> · {d.staged} staged</span>}
        </th>
        <td className="selectable">{s.target ?? s.name}</td>
        <td className="mono selectable">{s.version ?? "Unknown"}</td>
        <td className="mono selectable">{latest ?? "Unknown"}</td>
        <td>{fmtWhen(s.read_at)}</td>
        <td className={styles.actions}>
          <Button variant="secondary" aria-expanded={open} onClick={onOpen}>
            Options
          </Button>{" "}
          <Button variant="secondary" disabled={!latest} onClick={onFlash}>
            Flash {latest ?? ""}…
          </Button>
        </td>
      </tr>
      {open && (
        <tr>
          <td colSpan={6}>
            <div className={styles.options} role="group" aria-label={`Options of ${d.name}`}>
              {s.options.length === 0 && <p className={styles.muted}>The device lists no option QuadCam sets.</p>}
              {s.options.map((o) =>
                o.choices.length > 0 ? (
                  <SelectField key={o.key} label={o.label} value={draft[o.key] ?? o.value} onChange={(e) => setDraft((x) => ({ ...x, [o.key]: e.target.value }))}>
                    {o.choices.map((c) => (
                      <option key={c} value={c}>
                        {c}
                      </option>
                    ))}
                  </SelectField>
                ) : (
                  <TextField key={o.key} label={o.label} type="number" min={o.min ?? undefined} max={o.max ?? undefined} value={draft[o.key] ?? o.value} onChange={(e) => setDraft((x) => ({ ...x, [o.key]: e.target.value }))} />
                ),
              )}
              <div className={styles.optionsBar}>
                <Button variant="primary" disabled={Object.keys(draft).length === 0} onClick={onStage}>
                  Stage changes
                </Button>
              </div>
            </div>
          </td>
        </tr>
      )}
    </>
  );
}
