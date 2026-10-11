import { api } from "../../../ipc/api";
import type { CheckRow, RowState } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { PageHeader } from "../../../components/dash/PageHeader";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { StatusDot, type StatusTone } from "../../../components/dash/StatusDot";
import { ROW_LABEL } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import styles from "./Flights.module.css";

const TONE: Record<RowState, StatusTone> = { pass: "ok", warn: "warn", unknown: "idle" };
const STAT_TONE: Record<RowState, "ok" | "warn" | undefined> = { pass: "ok", warn: "warn", unknown: undefined };

const columns: Column<CheckRow>[] = [
  { key: "check", header: "Check", rowHeader: true, width: "10rem", cell: (r) => r.label },
  { key: "state", header: "State", width: "7rem", cell: (r) => <StatusDot tone={TONE[r.state]}>{ROW_LABEL[r.state]}</StatusDot> },
  { key: "detail", header: "Detail", cell: (r) => <span className="selectable">{r.detail}</span> },
];

/** Gear > Pack up: the read-only checks before a flying session. */
export function PackUpPage() {
  const { data, error, reload } = useGearData(() => api.gearPreflight());
  const packs = useGearData(() => api.gearPacks());
  const rows = data?.rows ?? [];
  const row = (id: string) => rows.find((r) => r.id === id);
  const live = (packs.data?.packs ?? []).filter((p) => !p.pack.retired);
  const charged = live.filter((p) => p.state === "charged").length;
  const tile = (id: string, label: string) => {
    const r = row(id);
    return <Stat label={label} value={r ? ROW_LABEL[r.state] : "—"} tone={r ? STAT_TONE[r.state] : undefined} />;
  };
  const ready = rows.filter((r) => r.state === "pass").length;
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Pack up"
        actions={
          <Button icon="refresh" onClick={reload}>
            Check again
          </Button>
        }
      />
      <StatRow label="Summary">
        <Stat label="Ready" value={data ? `${ready} of ${rows.length}` : "—"} tone={data ? (ready === rows.length ? "ok" : "warn") : undefined} />
        <Stat label="Packs charged" value={packs.data ? `${charged} of ${live.length}` : "—"} tone={live.length && charged < live.length ? "warn" : undefined} />
        {tile("cards_space", "Card space")}
        {tile("backups", "Backups")}
      </StatRow>
      {error && <p className={styles.muted}>{error}</p>}
      {data && <DataTable label="Checks" columns={columns} rows={rows} rowKey={(r) => r.id} />}
    </>
  );
}
