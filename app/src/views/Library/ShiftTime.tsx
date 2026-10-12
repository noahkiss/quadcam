import { useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useStore } from "../../store";
import { libClip } from "../../store/library";
import { api, errText } from "../../ipc/api";
import type { LibClip, LibShift, LibShiftReport, ShiftSource } from "../../ipc/types";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Input, SelectField, Field } from "../../components/Field";
import { DataTable, type Column } from "../../components/dash/DataTable";
import { Badge, type BadgeTone } from "../../components/dash/Badge";
import { toast } from "../../components/toastStore";
import { fmtDay } from "../../lib/format";
import { fmtOffset } from "../../lib/library";
import styles from "./ShiftTime.module.css";

const SOURCE: Record<ShiftSource, [string, BadgeTone]> = { set: ["Set", "neutral"], offset: ["Offset", "neutral"], log: ["Radio log", "ok"], skipped: ["No time", "neutral"] };

const when = (d: string | null, t: string | null) => (d ? `${fmtDay(d)}${t ? ` ${t}` : ""}` : "—");

/** Shifts the selected clips: the real date and time of one clip, the same offset for the
 *  rest, and a radio log's time for a clip a log matches. */
export function ShiftTime({ ids, onClose }: { ids: string[]; onClose: () => void }) {
  const clips = useStore(useShallow((s) => ids.map((id) => libClip(s, id)).filter((c): c is LibClip => !!c)));
  const cursor = useStore((s) => s.cursor);
  const timed = useMemo(() => clips.filter((c) => c.time), [clips]);
  const first = timed.find((c) => c.id === cursor) || timed[0];
  const [from, setFrom] = useState(first?.id || "");
  const anchor = timed.find((c) => c.id === from);
  const anchorId = anchor?.id;
  const [date, setDate] = useState(first?.date || "");
  const [time, setTime] = useState(first?.time || "");
  const [plan, setPlan] = useState<LibShiftReport | null>(null);
  const [busy, setBusy] = useState(false);

  // The plan follows every change of clip, date or time.
  useEffect(() => {
    if (!anchorId || !/^\d{1,2}:\d{2}$/.test(time) || !date) return;
    let live = true;
    api
      .timeShift({ ids, from: anchorId, date, time, apply: false })
      .then((r) => live && setPlan(r))
      .catch((e) => live && toast(errText(e), true));
    return () => {
      live = false;
    };
  }, [ids, anchorId, date, time]);

  const pick = (id: string) => {
    const c = timed.find((x) => x.id === id);
    setFrom(id);
    if (c) {
      setDate(c.date);
      setTime(c.time || "");
    }
  };
  const changed = plan?.clips.filter((c) => c.changed).length ?? 0;
  const close = async (v: string) => {
    if (v !== "shift" || !anchor) return onClose();
    setBusy(true);
    try {
      const r = await api.timeShift({ ids, from: anchor.id, date, time, apply: true });
      const n = r.clips.filter((c) => c.applied).length;
      toast(n === 1 ? "Shifted 1 clip." : `Shifted ${n} clips.`);
      onClose();
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(false);
    }
  };

  const columns: Column<LibShift>[] = [
    { key: "name", header: "Clip", rowHeader: true, cell: (r) => r.name },
    { key: "old", header: "Now", cell: (r) => <span className="mono">{when(r.old_date, r.old_time)}</span> },
    { key: "new", header: "New", cell: (r) => <span className="mono">{when(r.date, r.time)}</span> },
    {
      key: "source",
      header: "From",
      width: "7rem",
      cell: (r) => (
        <Badge tone={SOURCE[r.source][1]} title={r.reason || undefined}>
          {SOURCE[r.source][0]}
        </Badge>
      ),
    },
  ];

  return (
    <Dialog
      open
      title="Shift time"
      onClose={close}
      className={styles.dialog}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="shift" variant="primary" disabled={busy || !anchor || changed === 0}>
            {changed === 1 ? "Shift 1 clip" : `Shift ${changed} clips`}
          </Button>
        </>
      }
    >
      {!timed.length ? (
        <p className={styles.note}>None of the selected clips has a time of day.</p>
      ) : (
        <div className={styles.body}>
          <div className={styles.fields}>
            <SelectField label="Clip" value={from} onChange={(e) => pick(e.target.value)}>
              {timed.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </SelectField>
            <Field label="Real date">{(id) => <Input id={id} type="date" value={date} onChange={(e) => setDate(e.target.value)} />}</Field>
            <Field label="Real time">{(id) => <Input id={id} type="time" value={time} onChange={(e) => setTime(e.target.value)} />}</Field>
          </div>
          {plan && (
            <>
              <p className={styles.offset}>
                Offset <span className="mono">{fmtOffset(plan.offset_s)}</span>
              </p>
              <DataTable label="New times" columns={columns} rows={plan.clips} rowKey={(r) => r.id} compact />
              {plan.warnings.map((w) => (
                <p key={w} className={styles.note}>
                  {w}
                </p>
              ))}
            </>
          )}
        </div>
      )}
    </Dialog>
  );
}
