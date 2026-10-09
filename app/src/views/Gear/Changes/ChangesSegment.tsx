// An FC's Changes segment: the staged changes waiting for the person, the history of what
// was applied, and "Edit setting…", which stages one raw `set` line. The friendly editors
// (rates, OSD) stage through the same core calls later.
import { useState } from "react";
import { useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { Edit, StagedChange } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { SelectField, TextField } from "../../../components/Field";
import { toast } from "../../../components/toastStore";
import { fmtWhen } from "../../../lib/backups";
import type { DeviceRef } from "../slots";
import styles from "./Changes.module.css";

const STATUS: Record<StagedChange["status"], string> = {
  draft: "Draft",
  ready: "Ready",
  try: "Try",
  read_first: "Read first",
  applied: "Applied",
  verified: "Verified",
  failed: "Failed",
  reverted: "Reverted",
  discarded: "Discarded",
};

/** What a change does, in one line. */
export function summary(c: StagedChange): string {
  return c.edits
    .map((e: Edit) => {
      if (e.kind === "fc_set") return `set ${e.name} = ${e.value}`;
      if (e.kind === "fc_lines") return e.lines.filter((l) => l.trim()).join("; ");
      return e.kind.replace(/_/g, " ");
    })
    .join("; ");
}

export function ChangesSegment({ d }: { d: DeviceRef }) {
  const id = d.device?.id || d.connected?.id || null;
  const changes = useStore((s) => s.changes);
  const open = useStore((s) => s.openApply);
  const discard = useStore((s) => s.discardChange);
  const [editing, setEditing] = useState(false);
  const mine = changes.filter((c) => c.device === id);
  const canStage = !!d.device;
  return (
    <div className={styles.segment}>
      <div className={styles.bar}>
        <Button size="sm" variant="primary" disabled={!canStage} onClick={() => setEditing(true)}>
          Edit setting…
        </Button>
        <Button size="sm" disabled={!mine.some((c) => c.status === "ready")} onClick={() => id && open(id)}>
          Review…
        </Button>
        {!canStage && <span className={styles.muted}>Save this FC first.</span>}
      </div>
      {mine.length === 0 ? (
        <p className={styles.muted}>No staged changes.</p>
      ) : (
        <ul className={styles.list} aria-label="Staged changes">
          {mine.map((c) => (
            <li key={c.id}>
              <div className={styles.row}>
                <span className={styles.title}>{c.title}</span>
                <span className={styles.status} data-state={c.status}>
                  {STATUS[c.status]}
                </span>
                <Button size="sm" variant="ghost" onClick={() => open(c.device, c.id)}>
                  Review…
                </Button>
                <Button size="sm" variant="danger-ghost" onClick={() => discard(c.id)}>
                  Discard
                </Button>
              </div>
              <code className={`${styles.code} mono selectable`}>{summary(c)}</code>
              <span className={styles.muted}>{c.editor === "agent" ? "Staged by an agent" : "Staged by you"}</span>
            </li>
          ))}
        </ul>
      )}
      <History device={id} />
      {editing && id && <EditSetting device={id} onClose={() => setEditing(false)} />}
    </div>
  );
}

/** Applied, failed and discarded changes of the device, newest first. */
function History({ device }: { device: string | null }) {
  const [rows, setRows] = useState<StagedChange[] | null>(null);
  const changes = useStore((s) => s.changes);
  const [shown, setShown] = useState(false);
  const load = async () => {
    if (!device) return;
    try {
      setRows((await api.gearChanges(device, true)).filter((c) => c.status !== "ready" && c.status !== "draft").reverse());
    } catch (e) {
      toast(errText(e), true);
    }
  };
  void changes;
  return (
    <section aria-label="History">
      <Button
        size="sm"
        variant="ghost"
        iconEnd={shown ? "chev-down" : "chev-right"}
        onClick={() => {
          setShown(!shown);
          if (!shown) void load();
        }}
      >
        History
      </Button>
      {shown &&
        (rows?.length ? (
          <ul className={styles.list} aria-label="Applied changes">
            {rows.map((c) => (
              <li key={c.id}>
                <div className={styles.row}>
                  <span className={styles.title}>{c.title}</span>
                  <span className={styles.status} data-state={c.status}>
                    {STATUS[c.status]}
                  </span>
                  <span className={styles.muted}>{fmtWhen(c.history?.at(-1)?.at)}</span>
                </div>
                <code className={`${styles.code} mono selectable`}>{summary(c)}</code>
              </li>
            ))}
          </ul>
        ) : (
          <p className={styles.muted}>Nothing applied yet.</p>
        ))}
    </section>
  );
}

/** Stages one `set`. The core refuses a name the FC's latest backup does not hold. */
function EditSetting({ device, onClose }: { device: string; onClose: () => void }) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [kind, setKind] = useState("master");
  const [index, setIndex] = useState("0");
  const [error, setError] = useState<string | null>(null);
  const section = kind === "master" ? { kind: "master" as const } : { kind: kind as "profile" | "rate_profile", index: Number(index) };
  const close = async (v: string) => {
    if (v !== "stage") return onClose();
    try {
      const edit = { kind: "fc_set", section, name: name.trim(), value: value.trim() } as Edit;
      await api.gearChangeStage(device, [edit]);
      await useStore.getState().loadGear();
      onClose();
    } catch (e) {
      setError(errText(e));
    }
  };
  return (
    <Dialog
      open
      title="Edit setting"
      onClose={close}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="stage" variant="primary" disabled={!name.trim() || !value.trim()}>
            Stage
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <TextField label="Setting" mono placeholder="osd_cap_alarm" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        <TextField label="Value" mono value={value} onChange={(e) => setValue(e.target.value)} />
        <SelectField label="Applies to" value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="master">The whole FC</option>
          <option value="profile">A PID profile</option>
          <option value="rate_profile">A rate profile</option>
        </SelectField>
        {kind !== "master" && <TextField label="Profile number" inputMode="numeric" value={index} onChange={(e) => setIndex(e.target.value)} />}
        {error && <p role="alert" className={styles.error}>{error}</p>}
      </div>
    </Dialog>
  );
}
