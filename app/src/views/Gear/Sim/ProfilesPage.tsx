// Gear > Sim > Profiles (sim design 6.3, 6.6): fit a sim profile from an aircraft's logs, see
// each group the guard kept or dropped against the base profile, and save it behind the fit's
// digest. Below, the fitted profiles with Delete. The core writes nothing until Save.
import { useEffect, useState, type ReactNode } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { SelectField, TextField } from "../../../components/Field";
import { toast } from "../../../components/toastStore";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { Disclosure } from "../../../components/dash/Disclosure";
import { EmptyState } from "../../../components/dash/EmptyState";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { api, errText } from "../../../ipc/api";
import type { FitGroup, FitValue, SimFit, SimFitParams, SimProfileInfo, ValidationReport } from "../../../ipc/types";
import { useStore } from "../../../store";
import { sel } from "../../../store/settings";
import styles from "./Sim.module.css";

const GROUP_LABEL: Record<string, string> = { power: "Motor and pack", roll: "Roll gains", pitch: "Pitch gains", yaw: "Yaw gains" };

/** A value as the fit report prints it: four places, or an exponent for very small or large values. */
export function fmtValue(x: number | null | undefined): string {
  if (x == null || !Number.isFinite(x)) return "–";
  if (x !== 0 && (Math.abs(x) < 1e-3 || Math.abs(x) >= 1e5)) return x.toExponential(3);
  return String(Number(x.toFixed(4)));
}

/** Checks passed and run (a check the log has no window for does not count). */
export function passed(v: ValidationReport): [number, number] {
  const run = v.checks.filter((c) => c.outcome !== "not_in_log");
  return [run.filter((c) => c.outcome === "pass").length, run.length];
}

function ValueLine({ v }: { v: FitValue }) {
  return (
    <span className={styles.fitValue}>
      <span className={styles.mono}>{v.name}</span> {fmtValue(v.before)} → {fmtValue(v.value)} {v.unit}
    </span>
  );
}

/** The fitted profiles: one row each, Delete in its menu. */
function FittedList({ list, presets, onDelete }: { list: SimProfileInfo[]; presets: SimProfileInfo[]; onDelete: (p: SimProfileInfo) => void }) {
  const base = (id: string | null | undefined) => presets.find((p) => p.id === id)?.label ?? id ?? "–";
  const columns: Column<SimProfileInfo>[] = [
    { key: "label", header: "Profile", rowHeader: true, cell: (p) => p.label },
    { key: "aircraft", header: "Aircraft", cell: (p) => p.aircraft || "–" },
    { key: "base", header: "Starts from", optional: true, cell: (p) => base(p.base) },
    { key: "mass", header: "Weight", numeric: true, cell: (p) => (p.mass_g != null ? `${Math.round(p.mass_g)} g` : "–") },
    { key: "fitted", header: "Fitted", optional: true, cell: (p) => (p.fitted_at ? new Date(p.fitted_at).toLocaleDateString() : "–") },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "3rem",
      cell: (p) => <MoreMenu label={`Actions for ${p.label}`} items={[{ label: "Delete…", icon: "trash-bin-trash", danger: true, run: () => onDelete(p) }]} />,
    },
  ];
  return <DataTable label="Fitted profiles" columns={columns} rows={list} rowKey={(p) => p.id} empty={<EmptyState icon="quad">No fitted profiles yet.</EmptyState>} />;
}

/** The fit's result: each guard group with its fitted and base values, then the detail. */
function FitResult({ fit, base }: { fit: SimFit; base: string }) {
  const r = fit.report;
  const groups: Column<FitGroup>[] = [
    { key: "group", header: "Group", rowHeader: true, cell: (g) => GROUP_LABEL[g.name] ?? g.name },
    {
      key: "values",
      header: "Base → fitted",
      cell: (g) =>
        g.values.length ? (
          <span className={styles.fitValues}>
            {g.values.map((v) => (
              <ValueLine key={v.name} v={v} />
            ))}
          </span>
        ) : (
          "–"
        ),
    },
    { key: "failed", header: "Checks failed", numeric: true, optional: true, cell: (g) => `${g.failed} (base ${g.base_failed})` },
    {
      key: "kept",
      header: "Result",
      stack: "top",
      cell: (g) => (
        <span className={styles.fitValues}>
          <Badge tone={g.kept ? "ok" : "warn"}>{g.kept ? "Kept" : "Dropped"}</Badge>
          {g.note && <span className={styles.note}>{g.note}</span>}
        </span>
      ),
    },
  ];
  const values: Column<FitValue>[] = [
    { key: "name", header: "Value", rowHeader: true, cell: (v) => <span className={styles.mono}>{v.name}</span> },
    { key: "before", header: "Base", numeric: true, cell: (v) => fmtValue(v.before) },
    { key: "value", header: "Fitted", numeric: true, cell: (v) => fmtValue(v.value) },
    { key: "unit", header: "Unit", cell: (v) => v.unit },
    { key: "samples", header: "Samples", numeric: true, optional: true, cell: (v) => v.samples },
  ];
  const [p, n] = passed(fit.validation);
  const [bp, bn] = passed(fit.base_validation);
  return (
    <>
      <p className={styles.note} role="status" aria-label="Checks">
        Checks passed: {p} of {n} fitted, {bp} of {bn} with {base}.
      </p>
      <DataTable label="Fit groups" columns={groups} rows={r.groups} rowKey={(g) => g.name} empty={<p className={styles.note}>The fit scored no groups.</p>} />
      <Disclosure summary="All values" count={r.values.length + r.measured.length}>
        <DataTable label="Profile values" compact columns={values} rows={r.values} rowKey={(v) => v.name} />
        {r.measured.length > 0 && <DataTable label="Measured" compact columns={values} rows={r.measured} rowKey={(v) => v.name} />}
      </Disclosure>
      <Disclosure summary="Logs read" count={fit.sources.length}>
        <ul className={styles.fitList}>
          {fit.sources.map((s) => (
            <li key={s} className="selectable">
              {s}
            </li>
          ))}
        </ul>
      </Disclosure>
      {r.estimates.length + r.notes.length > 0 && (
        <Disclosure summary="Estimates and notes" count={r.estimates.length + r.notes.length}>
          <ul className={styles.fitList}>
            {[...r.notes, ...r.estimates].map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ul>
        </Disclosure>
      )}
    </>
  );
}

/** The Profiles section: the fit form and its result, and the fitted profiles. */
export function ProfilesPage({ tabs }: { tabs?: ReactNode }) {
  const aircraftList = useStore(sel.profiles);
  const [profiles, setProfiles] = useState<SimProfileInfo[]>([]);
  const [aircraft, setAircraft] = useState("");
  const [base, setBase] = useState("meteor75");
  const [mass, setMass] = useState("");
  const [busy, setBusy] = useState<"fit" | "save" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ params: SimFitParams; fit: SimFit } | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [deleting, setDeleting] = useState<SimProfileInfo | null>(null);

  const reload = () => api.gearSimProfiles().then(setProfiles, (e) => setError(errText(e)));
  useEffect(() => void reload(), []);

  const presets = profiles.filter((p) => p.kind === "preset");
  const fitted = profiles.filter((p) => p.kind === "fitted");
  const grams = mass.trim() === "" ? null : Number(mass);
  const massOk = grams === null || (Number.isFinite(grams) && grams > 0);
  const baseLabel = (id: string) => presets.find((p) => p.id === id)?.label ?? id;

  async function runFit() {
    const params: SimFitParams = { base, aircraft, mass_g: grams };
    setError(null);
    setResult(null);
    setBusy("fit");
    try {
      setResult({ params, fit: await api.gearSimFit(params) });
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  }

  async function save(v: string) {
    setConfirm(false);
    if (v !== "save" || !result) return;
    setBusy("save");
    try {
      const info = await api.gearSimFitSave(result.params, result.fit.digest);
      toast(`Saved ${info.label}`);
      setResult(null);
      await reload();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  }

  async function remove(v: string) {
    const p = deleting;
    setDeleting(null);
    if (v !== "delete" || !p) return;
    try {
      await api.gearSimProfileDelete(p.id);
      toast(`Deleted ${p.label}`);
      await reload();
    } catch (e) {
      setError(errText(e));
    }
  }

  const id = result?.fit.report.profile_id ?? "";
  const label = result ? (fitted.find((p) => p.id === id)?.label ?? result.params.aircraft ?? id) : "";
  return (
    <div className={styles.page} data-component="sim-profiles">
      <PageHeader title="Sim" subtitle={<SummaryLine parts={[`${fitted.length} fitted`, `${presets.length} built in`]} />} />
      {tabs}
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}

      <SectionCard
        title="Fit"
        actions={
          <Button variant="primary" onClick={() => void runFit()} disabled={!aircraft || !massOk || busy !== null}>
            {busy === "fit" ? "Fitting…" : "Fit Profile"}
          </Button>
        }
      >
        <div className={styles.fitForm}>
          <SelectField label="Aircraft" value={aircraft} onChange={(e) => setAircraft(e.target.value)}>
            <option value="">Choose…</option>
            {aircraftList.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </SelectField>
          <SelectField label="Starts from" value={base} onChange={(e) => setBase(e.target.value)}>
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </SelectField>
          <TextField label="Weight with pack (g)" inputMode="decimal" value={mass} placeholder={String(presets.find((p) => p.id === base)?.mass_g ?? "")} onChange={(e) => setMass(e.target.value)} aria-invalid={!massOk} />
        </div>
        {!aircraftList.length && <p className={styles.note}>Add an aircraft profile in Settings first.</p>}
      </SectionCard>

      {result && (
        <SectionCard
          title={`Result: ${label}`}
          actions={
            <Button variant="primary" onClick={() => setConfirm(true)} disabled={busy !== null}>
              {busy === "save" ? "Saving…" : result.fit.replaces ? "Replace…" : "Save…"}
            </Button>
          }
        >
          <FitResult fit={result.fit} base={baseLabel(result.params.base)} />
        </SectionCard>
      )}

      <SectionCard title="Fitted profiles">
        <FittedList list={fitted} presets={presets} onDelete={setDeleting} />
      </SectionCard>

      {confirm && result && (
        <Dialog
          open
          title={result.fit.replaces ? `Replace ${id}?` : `Save ${label}?`}
          onClose={(v) => void save(v)}
          actions={
            <>
              <Button type="submit" value="cancel" variant="ghost">
                Cancel
              </Button>
              <Button type="submit" value="save" variant="primary">
                {result.fit.replaces ? "Replace" : "Save"}
              </Button>
            </>
          }
        >
          <p className={styles.note}>
            {result.fit.replaces ? `Writes this fit over the saved profile ${id}.` : `Saves the sim profile ${id}.`}
          </p>
        </Dialog>
      )}
      {deleting && (
        <Dialog
          open
          title={`Delete ${deleting.label}?`}
          onClose={(v) => void remove(v)}
          actions={
            <>
              <Button type="submit" value="cancel" variant="ghost">
                Cancel
              </Button>
              <Button type="submit" value="delete" variant="danger">
                Delete
              </Button>
            </>
          }
        >
          <p className={styles.note}>Removes the fitted profile {deleting.id}. The built-in profiles stay.</p>
        </Dialog>
      )}
    </div>
  );
}
