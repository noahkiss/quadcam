import { useState } from "react";
import { useStore } from "../../../store";
import { ask } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { ChargeState, Chemistry, Pack, PackType, PackTypeView, PackView, PacksView } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Checkbox, SelectField, TextField } from "../../../components/Field";
import { Dialog } from "../../../components/Dialog";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
import { DetailDrawer, WithDrawer } from "../../../components/dash/DetailDrawer";
import { Disclosure } from "../../../components/dash/Disclosure";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { StatusDot, type StatusTone } from "../../../components/dash/StatusDot";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { CHARGE_LABEL, CHEMISTRY_LABEL, hhmm, mmss, num } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import styles from "./Flights.module.css";

const run = async (f: () => Promise<unknown>) => {
  try {
    await f();
  } catch (e) {
    toast(errText(e), true);
  }
};

const TONE: Record<ChargeState, StatusTone> = { charged: "ok", flown: "warn", unknown: "idle" };

/** Gear > Packs: the packs (history in a drawer, retired ones folded), and the charging
 *  sheet. */
export function PacksPage() {
  const segment = useStore((s) => s.gearSegment);
  const setSegment = useStore((s) => s.setGearSegment);
  const seg = segment === "charging" ? "charging" : "packs";
  const { data, error } = useGearData(() => api.gearPacks());
  const [pack, setPack] = useState<Pack | null>(null);
  const [type, setType] = useState<PackType | null>(null);
  const live = (data?.packs ?? []).filter((p) => !p.pack.retired);
  const charged = live.filter((p) => p.state === "charged").length;
  const weak = live.filter((p) => p.weak).length;
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Packs"
        subtitle={
          data && (
            <SummaryLine
              parts={[
                `${live.length} ${live.length === 1 ? "pack" : "packs"}`,
                { text: `${charged} of ${live.length} charged`, tone: live.length && charged < live.length ? "warn" : undefined },
                weak > 0 && { text: `${weak} weak`, tone: "warn" },
                `${live.reduce((n, p) => n + p.cycles, 0)} cycles`,
              ]}
            />
          )
        }
        fold={[
          seg === "charging"
            ? { label: "Add type…", icon: "add", run: () => setType({ name: "", chemistry: "lipo", cells: 1 }) }
            : { label: "Add pack…", icon: "add", run: () => setPack({ label: "", pack_type: data?.types[0]?.pack_type.name ?? null }) },
        ]}
        moreLabel="Packs actions"
      />
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
      {data && (seg === "charging" ? <Charging v={data} onEdit={setType} /> : <Packs v={data} onEdit={setPack} />)}
      {pack && data && <PackSheet pack={pack} types={data.types.map((t) => t.pack_type.name)} onClose={() => setPack(null)} />}
      {type && <TypeSheet t={type} onClose={() => setType(null)} />}
    </>
  );
}

function Packs({ v, onEdit }: { v: PacksView; onEdit: (p: Pack) => void }) {
  const [open, setOpen] = useState<string | null>(null);
  const sel = v.packs.find((p) => p.pack.label === open) || null;
  const live = v.packs.filter((p) => !p.pack.retired);
  const retired = v.packs.filter((p) => p.pack.retired);
  const columns: Column<PackView>[] = [
    {
      key: "label",
      header: "Label",
      rowHeader: true,
      cell: (p) => (
        <button type="button" className={styles.rowButton} aria-expanded={p === sel} onClick={() => setOpen(p === sel ? null : p.pack.label)}>
          {p.pack.label}
        </button>
      ),
    },
    { key: "type", header: "Type", cell: (p) => p.pack.pack_type || "–" },
    { key: "state", header: "State", stack: "top", cell: (p) => (p.pack.retired ? <StatusDot tone="idle">Retired</StatusDot> : <StatusDot tone={TONE[p.state]}>{CHARGE_LABEL[p.state]}</StatusDot>) },
    { key: "cycles", header: "Cycles", numeric: true, cell: (p) => p.cycles },
    { key: "resting", header: "Resting", numeric: true, optional: true, cell: (p) => num(p.median_resting_v, 2, "V") },
    { key: "time", header: "Flight time", numeric: true, optional: true, cell: (p) => mmss(p.median_secs) },
    { key: "note", header: "Note", cell: (p) => (p.weak ? <span className={styles.warn}>Low: {p.weak}</span> : p.pack.note || "") },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      cell: (p) => (
        <div className={styles.actions}>
          {p.state !== "charged" && !p.pack.retired && (
            <Button size="sm" onClick={() => run(() => api.gearPackSave(p.pack, true))}>
              Mark charged
            </Button>
          )}
          <MoreMenu
            label={`More actions for ${p.pack.label}`}
            items={[
              { label: "Edit…", run: () => onEdit(p.pack) },
              { label: p.pack.retired ? "Return to use" : "Retire", run: () => run(() => api.gearPackSave({ ...p.pack, retired: !p.pack.retired })) },
            ]}
          />
        </div>
      ),
    },
  ];
  return (
    <WithDrawer drawer={sel && <History p={sel} onClose={() => setOpen(null)} />}>
      <DataTable label="Packs" columns={columns} rows={live} rowKey={(p) => p.pack.label} empty={<EmptyState icon="battery">No packs saved.</EmptyState>} />
      {retired.length > 0 && (
        <section aria-label="Retired packs">
          <Disclosure summary={`${retired.length} retired`}>
            <DataTable label="Retired packs" columns={columns} rows={retired} rowKey={(p) => p.pack.label} />
          </Disclosure>
        </section>
      )}
    </WithDrawer>
  );
}

type PackFlight = PackView["history"][number];

const historyColumns: Column<PackFlight>[] = [
  { key: "day", header: "Day", rowHeader: true, cell: (h) => fmtDay(h.start.slice(0, 10)) },
  { key: "start", header: "Start", numeric: true, cell: (h) => hhmm(h.start) },
  { key: "length", header: "Length", numeric: true, stack: "top", cell: (h) => mmss(h.secs) },
  { key: "used", header: "Used", numeric: true, cell: (h) => num(h.mah, 0, "mAh") },
  { key: "sag", header: "Sag (p5 / min)", numeric: true, cell: (h) => `${num(h.sag_p5_v, 2)} / ${num(h.sag_min_v, 2, "V")}` },
  { key: "resting", header: "Resting", numeric: true, cell: (h) => num(h.resting_v, 2, "V") },
];

/** A pack's flights in the drawer, newest first. */
function History({ p, onClose }: { p: PackView; onClose: () => void }) {
  return (
    <DetailDrawer title={`History of ${p.pack.label}`} subtitle={[p.pack.pack_type, `${p.cycles} cycles`].filter(Boolean).join(" · ")} onClose={onClose}>
      <DataTable label={`Flights on ${p.pack.label}`} compact columns={historyColumns} rows={p.history} rowKey={(h) => h.flight} empty={<EmptyState>No flights on this pack yet.</EmptyState>} />
    </DetailDrawer>
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

function Charging({ v, onEdit }: { v: PacksView; onEdit: (t: PackType) => void }) {
  const [notes, setNotes] = useState<string | null>(null);
  const [notesOpen, setNotesOpen] = useState(!!v.notes.trim());
  const columns: Column<PackTypeView>[] = [
    { key: "type", header: "Type", rowHeader: true, cell: (t) => t.pack_type.name },
    { key: "chem", header: "Chemistry", cell: (t) => `${CHEMISTRY_LABEL[t.pack_type.chemistry || "lipo"]} ${t.pack_type.cells ?? 1}S` },
    { key: "capacity", header: "Capacity", numeric: true, cell: (t) => num(t.pack_type.capacity_mah, 0, "mAh") },
    { key: "connector", header: "Connector", optional: true, cell: (t) => t.pack_type.connector || "–" },
    { key: "full", header: "Full", numeric: true, cell: (t) => num(t.full_total_v, 2, "V") },
    { key: "storage", header: "Storage", numeric: true, cell: (t) => num(t.storage_total_v, 2, "V") },
    { key: "charge", header: "Charge", numeric: true, optional: true, cell: (t) => num(t.pack_type.charge_a, 1, "A") },
    { key: "warn", header: "Warning", numeric: true, cell: (t) => num(t.pack_type.warn_mah, 0, "mAh") },
    { key: "suggested", header: "Suggested warning", numeric: true, optional: true, cell: (t) => (t.suggested_warn_mah != null ? `${num(t.suggested_warn_mah, 0, "mAh")} for ${num(v.target_v, 2, "V")} a cell` : "–") },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      cell: (t) => (
        <Button size="sm" className={styles.inCell} onClick={() => onEdit(t.pack_type)}>
          Edit…
        </Button>
      ),
    },
  ];
  return (
    <>
      <DataTable label="Pack types" columns={columns} rows={v.types} rowKey={(t) => t.pack_type.name} empty={<EmptyState icon="battery">No pack types saved.</EmptyState>} />
      <SectionCard title="Notes" label="Charging notes" collapse={{ open: notesOpen, onToggle: setNotesOpen }}>
        <textarea
          id="charging-notes"
          aria-label="Charging notes"
          className={styles.notes}
          value={notes ?? v.notes}
          onChange={(e) => setNotes(e.target.value)}
          onBlur={() => {
            if (notes != null && notes !== v.notes) run(() => api.gearPackNotes(notes));
            setNotes(null);
          }}
        />
      </SectionCard>
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
