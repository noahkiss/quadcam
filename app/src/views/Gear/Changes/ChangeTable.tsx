// The one change table of the Bench and a device's Changes segment: staged changes with
// their status control and Review, and a … menu with who made the change, when, and Discard;
// applied Try changes with Keep and Revert; and the history. Nothing here writes a device:
// Review opens the apply sheet.
import { useStore } from "../../../store";
import type { StagedChange } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Select } from "../../../components/Field";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { Disclosure } from "../../../components/dash/Disclosure";
import { MoreMenu } from "../../../components/dash/MoreMenu";
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
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "8rem",
      cell: (c) => (
        <span className={styles.actions}>
          <Button size="sm" variant="ghost" disabled={!present || c.status === "draft" || c.status === "read_first"} onClick={() => void open(c.device, c.id)}>
            Review…
          </Button>
          <MoreMenu
            label={`More actions for ${c.title}`}
            items={[
              { label: [`By ${c.editor === "agent" ? "an agent" : "you"}`, fmtWhen(c.history?.at(-1)?.at)].filter(Boolean).join(", "), disabled: true, run: () => {} },
              null,
              { label: "Discard", icon: "trash-bin-trash", danger: true, run: () => void discard(c.id) },
            ]}
          />
        </span>
      ),
    },
  ];
  return (
    <div className={styles.rows}>
      <DataTable label={label} columns={columns} rows={rows} rowKey={(c) => c.id} />
    </div>
  );
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
  return (
    <div className={styles.rows}>
      <DataTable label={label} columns={columns} rows={rows} rowKey={(c) => c.id} />
    </div>
  );
}

/** Applied, failed, reverted and discarded changes, newest first. `names` adds a Device column. */
export function HistoryTable({ rows, names }: { rows: StagedChange[]; names?: Record<string, string> }) {
  const columns: Column<StagedChange>[] = [title, edit];
  if (names) columns.push({ key: "device", header: "Device", optional: true, cell: (c) => names[c.device] || c.device });
  columns.push(state, when);
  return <DataTable label="Applied changes" columns={columns} rows={rows} rowKey={(c) => c.id} compact />;
}

/** The History disclosure: closed at first, with its count, then the history table. */
export function History({ rows, names }: { rows: StagedChange[]; names?: Record<string, string> }) {
  return (
    <section aria-label="History" className={styles.history}>
      <Disclosure summary="History" count={rows.length}>
        {rows.length ? <HistoryTable rows={rows} names={names} /> : <p className={styles.muted}>Nothing applied yet.</p>}
      </Disclosure>
    </section>
  );
}

/** The history of `all`, newest first. */
export const historyOf = (all: StagedChange[]) => all.filter((c) => !WAITING.includes(c.status) && c.status !== "applied").slice().reverse();
