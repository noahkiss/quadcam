import { api } from "../../../ipc/api";
import type { CheckRow, RowState } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { Disclosure } from "../../../components/dash/Disclosure";
import { PageHeader } from "../../../components/dash/PageHeader";
import { StatusDot, type StatusTone } from "../../../components/dash/StatusDot";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { ROW_LABEL } from "../../../lib/flights";
import { useStore } from "../../../store";
import { useGearData } from "./useGearData";
import styles from "./Flights.module.css";

const TONE: Record<RowState, StatusTone> = { pass: "ok", warn: "warn", unknown: "idle" };

/** Gear > Pack up: the read-only checks before a flying session. Checks that warn or cannot
 *  tell come first, each with its fix; the ones that passed fold under a disclosure. */
export function PackUpPage() {
  const openGear = useStore((s) => s.openGear);
  const { data, error, reload } = useGearData(() => api.gearPreflight());
  const packs = useGearData(() => api.gearPacks());
  const rows = data?.rows ?? [];
  const live = (packs.data?.packs ?? []).filter((p) => !p.pack.retired);
  const charged = live.filter((p) => p.state === "charged").length;
  const ready = rows.filter((r) => r.state === "pass").length;
  const todo = rows.filter((r) => r.state !== "pass");
  const passed = rows.filter((r) => r.state === "pass");
  const fix = (r: CheckRow) =>
    r.id === "packs" ? (
      <Button size="sm" className={styles.inCell} onClick={() => openGear({ page: "slot", id: "packs" })}>
        Open Packs
      </Button>
    ) : (
      <Button size="sm" className={styles.inCell} onClick={reload}>
        Check now
      </Button>
    );
  const columns: Column<CheckRow>[] = [
    { key: "check", header: "Check", rowHeader: true, width: "10rem", cell: (r) => r.label },
    { key: "state", header: "State", width: "7rem", stack: "top", cell: (r) => <StatusDot tone={TONE[r.state]}>{ROW_LABEL[r.state]}</StatusDot> },
    { key: "detail", header: "Detail", cell: (r) => <span className="selectable">{r.detail}</span> },
  ];
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Pack up"
        subtitle={
          data && (
            <SummaryLine
              parts={[
                { text: `${ready} of ${rows.length} ready`, tone: ready === rows.length ? "ok" : "warn" },
                packs.data && { text: `${charged} of ${live.length} packs charged`, tone: live.length && charged < live.length ? "warn" : undefined },
              ]}
            />
          )
        }
        actions={
          <Button icon="refresh" onClick={reload}>
            Check again
          </Button>
        }
      />
      {error && <p className={styles.muted}>{error}</p>}
      {data && todo.length > 0 && <DataTable label="Checks" columns={[...columns, { key: "actions", header: <span className="visually-hidden">Fix</span>, width: "8rem", cell: fix }]} rows={todo} rowKey={(r) => r.id} />}
      {data && passed.length > 0 && (
        <section aria-label="Passed checks">
          <Disclosure summary={`${passed.length} passed`} defaultOpen={todo.length === 0}>
            <DataTable label="Passed checks" columns={columns} rows={passed} rowKey={(r) => r.id} />
          </Disclosure>
        </section>
      )}
    </>
  );
}
