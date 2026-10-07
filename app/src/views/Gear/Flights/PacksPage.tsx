import { useState } from "react";
import { useStore } from "../../../store";
import { ask } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { Chemistry, Pack, PackType, PackView, PacksView } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Checkbox, SelectField, TextField } from "../../../components/Field";
import { Dialog } from "../../../components/Dialog";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { CHARGE_LABEL, CHEMISTRY_LABEL, hhmm, mmss, num } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import g from "../Gear.module.css";
import styles from "./Flights.module.css";

const run = async (f: () => Promise<unknown>) => {
  try {
    await f();
  } catch (e) {
    toast(errText(e), true);
  }
};

/** Gear > Packs: the packs with their history, and the charging sheet. */
export function PacksPage() {
  const segment = useStore((s) => s.gearSegment);
  const setSegment = useStore((s) => s.setGearSegment);
  const seg = segment === "charging" ? "charging" : "packs";
  const { data, error } = useGearData(() => api.gearPacks());
  return (
    <>
      <h2 className={g.title}>Packs</h2>
      <SegmentedControl
        label="Sections"
        value={seg}
        onChange={setSegment}
        segments={[
          { value: "packs", label: "Packs" },
          { value: "charging", label: "Charging" },
        ]}
      />
      {error && <p className={styles.muted}>{error}</p>}
      {data && (seg === "charging" ? <Charging v={data} /> : <Packs v={data} />)}
    </>
  );
}

function Packs({ v }: { v: PacksView }) {
  const [open, setOpen] = useState<string | null>(null);
  const [editing, setEditing] = useState<Pack | null>(null);
  const sel = v.packs.find((p) => p.pack.label === open) || null;
  return (
    <>
      <div className={styles.bar}>
        <span className={styles.grow} />
        <Button icon="add" onClick={() => setEditing({ label: "", pack_type: v.types[0]?.pack_type.name ?? null })}>
          Add pack…
        </Button>
      </div>
      {v.packs.length === 0 ? (
        <p className={g.empty}>No packs saved.</p>
      ) : (
        <div className={styles.tableWrap}>
          <table className={styles.table} aria-label="Packs">
            <thead>
              <tr>
                <th scope="col">Label</th>
                <th scope="col">Type</th>
                <th scope="col">State</th>
                <th scope="col">Cycles</th>
                <th scope="col">Resting</th>
                <th scope="col">Flight time</th>
                <th scope="col">Note</th>
                <th scope="col">
                  <span className="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {v.packs.map((p) => (
                <tr key={p.pack.label} aria-selected={p === sel}>
                  <td>
                    <button type="button" className={styles.rowButton} aria-expanded={p === sel} onClick={() => setOpen(p === sel ? null : p.pack.label)}>
                      {p.pack.label}
                    </button>
                  </td>
                  <td>{p.pack.pack_type || "–"}</td>
                  <td>{p.pack.retired ? "Retired" : CHARGE_LABEL[p.state]}</td>
                  <td>{p.cycles}</td>
                  <td>{num(p.median_resting_v, 2, "V")}</td>
                  <td>{mmss(p.median_secs)}</td>
                  <td className={p.weak ? styles.warn : undefined}>{p.weak ? `Low: ${p.weak}` : p.pack.note || ""}</td>
                  <td>
                    {p.state !== "charged" && !p.pack.retired && (
                      <Button size="sm" variant="ghost" onClick={() => run(() => api.gearPackSave(p.pack, true))}>
                        Mark charged
                      </Button>
                    )}
                    <Button size="sm" variant="ghost" onClick={() => setEditing(p.pack)}>
                      Edit…
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {sel && <History p={sel} />}
      {editing && <PackSheet pack={editing} types={v.types.map((t) => t.pack_type.name)} onClose={() => setEditing(null)} />}
    </>
  );
}

function History({ p }: { p: PackView }) {
  return (
    <section className={styles.panel} aria-label={`History of ${p.pack.label}`}>
      <h3>History of {p.pack.label}</h3>
      {p.history.length === 0 ? (
        <p className={styles.muted}>No flights on this pack yet.</p>
      ) : (
        <table className={styles.table} aria-label={`Flights on ${p.pack.label}`}>
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col">Start</th>
              <th scope="col">Length</th>
              <th scope="col">Used</th>
              <th scope="col">Sag (p5 / min)</th>
              <th scope="col">Resting</th>
            </tr>
          </thead>
          <tbody>
            {p.history.map((h) => (
              <tr key={h.flight}>
                <td>{fmtDay(h.start.slice(0, 10))}</td>
                <td>{hhmm(h.start)}</td>
                <td>{mmss(h.secs)}</td>
                <td>{num(h.mah, 0, "mAh")}</td>
                <td>
                  {num(h.sag_p5_v, 2)} / {num(h.sag_min_v, 2, "V")}
                </td>
                <td>{num(h.resting_v, 2, "V")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function PackSheet({ pack, types, onClose }: { pack: Pack; types: string[]; onClose: () => void }) {
  const isNew = !pack.label;
  const [p, setP] = useState<Pack>(pack);
  const close = async (v: string) => {
    if (v === "save") await run(() => api.gearPackSave({ ...p, label: p.label.trim() }));
    if (v === "delete") {
      const ok = await ask(`Delete pack ${pack.label}?`, "Its flights keep the label.", { ok: "Delete", danger: true });
      if (ok === true) await run(() => api.gearPackDelete(pack.label));
    }
    onClose();
  };
  return (
    <Dialog
      open
      title={isNew ? "Add pack" : `Edit ${pack.label}`}
      onClose={close}
      actions={
        <>
          {!isNew && (
            <Button type="submit" value="delete" variant="danger-ghost">
              Delete…
            </Button>
          )}
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="save" variant="primary" disabled={!p.label.trim()}>
            Save
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <TextField label="Label" value={p.label} disabled={!isNew} onChange={(e) => setP({ ...p, label: e.target.value })} autoFocus={isNew} />
        <SelectField label="Type" value={p.pack_type || ""} onChange={(e) => setP({ ...p, pack_type: e.target.value || null })}>
          <option value="">None</option>
          {types.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </SelectField>
        <TextField label="Received" type="date" value={p.received || ""} onChange={(e) => setP({ ...p, received: e.target.value || null })} />
        <TextField label="Note" value={p.note || ""} onChange={(e) => setP({ ...p, note: e.target.value })} />
        <Checkbox label="Retired" checked={!!p.retired} onChange={(e) => setP({ ...p, retired: e.target.checked })} />
      </div>
    </Dialog>
  );
}

function Charging({ v }: { v: PacksView }) {
  const [editing, setEditing] = useState<PackType | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  return (
    <>
      <div className={styles.bar}>
        <span className={styles.grow} />
        <Button icon="add" onClick={() => setEditing({ name: "", chemistry: "lipo", cells: 1 })}>
          Add type…
        </Button>
      </div>
      {v.types.length === 0 ? (
        <p className={g.empty}>No pack types saved.</p>
      ) : (
        <div className={styles.tableWrap}>
          <table className={styles.table} aria-label="Pack types">
            <thead>
              <tr>
                <th scope="col">Type</th>
                <th scope="col">Chemistry</th>
                <th scope="col">Capacity</th>
                <th scope="col">Connector</th>
                <th scope="col">Full</th>
                <th scope="col">Storage</th>
                <th scope="col">Charge</th>
                <th scope="col">Warning</th>
                <th scope="col">Suggested warning</th>
                <th scope="col">
                  <span className="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {v.types.map((t) => {
                const pt = t.pack_type;
                return (
                  <tr key={pt.name}>
                    <td>{pt.name}</td>
                    <td>
                      {CHEMISTRY_LABEL[pt.chemistry || "lipo"]} {pt.cells ?? 1}S
                    </td>
                    <td>{num(pt.capacity_mah, 0, "mAh")}</td>
                    <td>{pt.connector || "–"}</td>
                    <td>{num(t.full_total_v, 2, "V")}</td>
                    <td>{num(t.storage_total_v, 2, "V")}</td>
                    <td>{num(pt.charge_a, 1, "A")}</td>
                    <td>{num(pt.warn_mah, 0, "mAh")}</td>
                    <td>{t.suggested_warn_mah != null ? `${num(t.suggested_warn_mah, 0, "mAh")} for ${num(v.target_v, 2, "V")} a cell` : "–"}</td>
                    <td>
                      <Button size="sm" variant="ghost" onClick={() => setEditing(pt)}>
                        Edit…
                      </Button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <label className={styles.muted} htmlFor="charging-notes">
        Notes
      </label>
      <textarea
        id="charging-notes"
        className={styles.notes}
        value={notes ?? v.notes}
        onChange={(e) => setNotes(e.target.value)}
        onBlur={() => {
          if (notes != null && notes !== v.notes) run(() => api.gearPackNotes(notes));
          setNotes(null);
        }}
      />
      {editing && <TypeSheet t={editing} onClose={() => setEditing(null)} />}
    </>
  );
}

const numOrNull = (s: string) => (s.trim() === "" || !Number.isFinite(Number(s)) ? null : Number(s));

function TypeSheet({ t: start, onClose }: { t: PackType; onClose: () => void }) {
  const isNew = !start.name;
  const [t, setT] = useState<PackType>(start);
  const field = (label: string, key: "capacity_mah" | "full_v" | "storage_v" | "charge_a" | "warn_mah", step: string) => (
    <TextField label={label} type="number" step={step} min="0" value={t[key] ?? ""} onChange={(e) => setT({ ...t, [key]: numOrNull(e.target.value) })} />
  );
  const close = async (v: string) => {
    if (v === "save") await run(() => api.gearPackTypeSave({ ...t, name: t.name.trim() }));
    if (v === "delete") {
      const ok = await ask(`Delete pack type ${start.name}?`, "", { ok: "Delete", danger: true });
      if (ok === true) await run(() => api.gearPackTypeDelete(start.name));
    }
    onClose();
  };
  return (
    <Dialog
      open
      title={isNew ? "Add pack type" : `Edit ${start.name}`}
      onClose={close}
      actions={
        <>
          {!isNew && (
            <Button type="submit" value="delete" variant="danger-ghost">
              Delete…
            </Button>
          )}
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="save" variant="primary" disabled={!t.name.trim()}>
            Save
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <TextField label="Name" value={t.name} disabled={!isNew} onChange={(e) => setT({ ...t, name: e.target.value })} autoFocus={isNew} />
        <div className={styles.pair}>
          <SelectField label="Chemistry" value={t.chemistry || "lipo"} onChange={(e) => setT({ ...t, chemistry: e.target.value as Chemistry })}>
            {(Object.keys(CHEMISTRY_LABEL) as Chemistry[]).map((c) => (
              <option key={c} value={c}>
                {CHEMISTRY_LABEL[c]}
              </option>
            ))}
          </SelectField>
          <TextField label="Cells" type="number" min="1" max="14" value={t.cells ?? 1} onChange={(e) => setT({ ...t, cells: Math.max(1, Number(e.target.value) || 1) })} />
        </div>
        <div className={styles.pair}>
          {field("Capacity (mAh)", "capacity_mah", "1")}
          <TextField label="Connector" value={t.connector || ""} onChange={(e) => setT({ ...t, connector: e.target.value || null })} />
        </div>
        <div className={styles.pair}>
          {field("Full (V a cell)", "full_v", "0.01")}
          {field("Storage (V a cell)", "storage_v", "0.01")}
        </div>
        <div className={styles.pair}>
          {field("Charge current (A)", "charge_a", "0.1")}
          {field("Warning (mAh)", "warn_mah", "1")}
        </div>
      </div>
    </Dialog>
  );
}
