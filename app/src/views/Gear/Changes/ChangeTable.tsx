// The one change table of the Bench and a device's Changes segment: staged changes with
// their status control, Review and Discard; applied Try changes with Keep and Revert; and the
// history. Nothing here writes a device: Review opens the apply sheet.
import { useState } from "react";
import { useStore } from "../../../store";
import type { StagedChange } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Select } from "../../../components/Field";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { StatusDot, type StatusTone } from "../../../components/dash/StatusDot";
import { fmtWhen } from "../../../lib/backups";
import { STATUS_HINT, STATUS_LABEL, summary, WAITING } from "../../../lib/bench";
import styles from "./Changes.module.css";

const TONE: Partial<Record<StagedChange["status"], StatusTone>> = { verified: "ok", failed: "error", applied: "warn" };

const title: Column<StagedChange> = {
  key: "title",
  header: "Title",
  rowHeader: true,
  cell: (c) => (
    <>
      {c.title}
      {c.status === "read_first" && <span className={styles.note}>Read the real value on the device, then set this to Ready.</span>}
    </>
  ),
};

const edit: Column<StagedChange> = {
  key: "edit",
  header: "Edit",
  cell: (c) => {
    const s = summary(c);
    return (
      <code className={`${styles.edit} mono selectable`} title={s}>
        {s}
      </code>
    );
  },
};

const by: Column<StagedChange> = { key: "by", header: "By", optional: true, width: "4.5rem", cell: (c) => (c.editor === "agent" ? "Agent" : "You") };

const when: Column<StagedChange> = { key: "when", header: "When", optional: true, width: "11rem", cell: (c) => fmtWhen(c.history?.at(-1)?.at) };

const state: Column<StagedChange> = { key: "status", header: "Status", width: "7rem", cell: (c) => <StatusDot tone={TONE[c.status] ?? "idle"}>{STATUS_LABEL[c.status]}</StatusDot> };

/** Staged changes in queue order. `present`: the device is in the Mac, so Review can open. */
export function StagedTable({ rows, label, present }: { rows: StagedChange[]; label: string; present: boolean }) {
  const open = useStore((s) => s.openApply);
  const setStatus = useStore((s) => s.setChangeStatus);
  const discard = useStore((s) => s.discardChange);
  const columns: Column<StagedChange>[] = [
    title,
    edit,
    {
      key: "status",
      header: "Status",
      width: "8.5rem",
      cell: (c) => (
        <Select className={styles.status} aria-label={`Status of ${c.title}`} title={STATUS_HINT[c.status]} value={c.status} onChange={(e) => void setStatus(c.id, e.target.value as StagedChange["status"])}>
          {WAITING.map((s) => (
            <option key={s} value={s}>
              {STATUS_LABEL[s]}
            </option>
          ))}
        </Select>
      ),
    },
    by,
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "10rem",
      cell: (c) => (
        <span className={styles.actions}>
          <Button size="sm" variant="ghost" disabled={!present || c.status === "draft" || c.status === "read_first"} onClick={() => void open(c.device, c.id)}>
            Review…
          </Button>
          <Button size="sm" variant="danger-ghost" onClick={() => void discard(c.id)}>
            Discard
          </Button>
        </span>
      ),
    },
  ];
  return <DataTable label={label} columns={columns} rows={rows} rowKey={(c) => c.id} />;
}

/** Applied Try changes: fly, then Keep or Revert. */
export function AwaitingTable({ rows, label }: { rows: StagedChange[]; label: string }) {
  const keep = useStore((s) => s.keepChange);
  const revert = useStore((s) => s.revertChange);
  const columns: Column<StagedChange>[] = [
    title,
    edit,
    state,
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "10rem",
      cell: (c) => (
        <span className={styles.actions}>
          <Button size="sm" variant="primary" onClick={() => void keep(c.id)}>
            Keep
          </Button>
          <Button size="sm" variant="danger-ghost" onClick={() => void revert(c.id)}>
            Revert…
          </Button>
        </span>
      ),
    },
  ];
  return <DataTable label={label} columns={columns} rows={rows} rowKey={(c) => c.id} />;
}

/** Applied, failed, reverted and discarded changes, newest first. `names` adds a Device column. */
export function HistoryTable({ rows, names }: { rows: StagedChange[]; names?: Record<string, string> }) {
  const columns: Column<StagedChange>[] = [title, edit];
  if (names) columns.push({ key: "device", header: "Device", optional: true, cell: (c) => names[c.device] || c.device });
  columns.push(state, when);
  return <DataTable label="Applied changes" columns={columns} rows={rows} rowKey={(c) => c.id} compact />;
}

/** The History disclosure: closed at first, then the history table. */
export function History({ rows, names }: { rows: StagedChange[]; names?: Record<string, string> }) {
  const [shown, setShown] = useState(false);
  return (
    <section aria-label="History" className={styles.history}>
      <Button size="sm" variant="ghost" iconEnd={shown ? "chev-down" : "chev-right"} onClick={() => setShown(!shown)} aria-expanded={shown}>
        History
      </Button>
      {shown && (rows.length ? <HistoryTable rows={rows} names={names} /> : <p className={styles.muted}>Nothing applied yet.</p>)}
    </section>
  );
}

/** The history of `all`, newest first. */
export const historyOf = (all: StagedChange[]) => all.filter((c) => !WAITING.includes(c.status) && c.status !== "applied").slice().reverse();
