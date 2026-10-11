import { useStore } from "../../../store";
import { ask } from "../../../store";
import { libClip } from "../../../store/library";
import { api, errText } from "../../../ipc/api";
import type { Crash } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Checkbox } from "../../../components/Field";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { mmss } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import styles from "./Flights.module.css";

const NO_AIRCRAFT = "No aircraft";

/** Gear > Repairs: every crash in one table, grouped by aircraft, with what broke and the
 *  parts used. */
export function RepairsPage() {
  const s = useStore();
  const openDetail = useStore((x) => x.openDetail);
  const { data, error } = useGearData(() => api.gearCrashes());
  const list = data ?? [];
  const open = list.filter((c) => !c.repaired).length;
  const last = list.reduce<Crash | null>((a, c) => (!a || c.day > a.day ? c : a), null);
  // Aircraft by name, "No aircraft" last; each aircraft's crashes newest first.
  const rows = [...list].sort((a, b) => {
    const ka = a.aircraft || "￿";
    const kb = b.aircraft || "￿";
    return ka === kb ? b.day.localeCompare(a.day) : ka.localeCompare(kb);
  });
  const save = async (c: Crash, repaired: boolean) => {
    try {
      await api.gearCrashSave({ id: c.id, repaired });
    } catch (e) {
      toast(errText(e), true);
    }
  };
  const del = async (c: Crash) => {
    const ok = await ask("Delete this crash?", c.broke || "", { ok: "Delete", danger: true });
    if (ok !== true) return;
    try {
      await api.gearCrashDelete(c.id);
    } catch (e) {
      toast(errText(e), true);
    }
  };
  const columns: Column<Crash>[] = [
    { key: "day", header: "Day", rowHeader: true, width: "7rem", cell: (c) => fmtDay(c.day) },
    {
      key: "clip",
      header: "Clip",
      optional: true,
      cell: (c) =>
        c.clip ? (
          <button type="button" className={styles.rowButton} onClick={() => openDetail(c.clip!, c.time_s ?? null, "flight")}>
            {libClip(s, c.clip)?.name || c.clip}
          </button>
        ) : (
          "–"
        ),
    },
    { key: "at", header: "At", numeric: true, width: "4rem", cell: (c) => (c.time_s != null ? mmss(c.time_s) : "–") },
    { key: "broke", header: "Broke", cell: (c) => c.broke || "–" },
    { key: "parts", header: "Parts", cell: (c) => (c.parts?.length ? c.parts.join(", ") : "–") },
    {
      key: "repaired",
      header: "Repaired",
      width: "5.5rem",
      cell: (c) => <Checkbox label={<span className="visually-hidden">Repaired</span>} checked={!!c.repaired} onChange={(e) => save(c, e.target.checked)} />,
    },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "6rem",
      cell: (c) => (
        <Button size="sm" variant="danger-ghost" className={styles.inCell} onClick={() => del(c)}>
          Delete…
        </Button>
      ),
    },
  ];
  return (
    <>
      <PageHeader id="gear-title" title="Repairs" />
      <StatRow label="Summary">
        <Stat label="Open" value={open} tone={open ? "warn" : undefined} />
        <Stat label="Repaired" value={list.length - open} />
        <Stat label="Last crash" value={last ? fmtDay(last.day) : "—"} sub={last ? last.aircraft || NO_AIRCRAFT : undefined} />
      </StatRow>
      {error && <p className={styles.muted}>{error}</p>}
      {data && (
        <DataTable
          label="Crashes"
          columns={columns}
          rows={rows}
          rowKey={(c) => c.id}
          group={(c) => c.aircraft || NO_AIRCRAFT}
          empty={<EmptyState icon="danger-triangle">No crashes logged. Log one from a clip's Flight tab.</EmptyState>}
        />
      )}
    </>
  );
}
