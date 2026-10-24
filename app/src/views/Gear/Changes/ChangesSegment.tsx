// An FC's Changes segment: the staged changes waiting for the person, the history of what
// was applied, and "Edit setting…", which stages one raw `set` line. The friendly editors
// (rates, OSD) stage through the same core calls later.
import { useState } from "react";
import { useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { Edit } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { SelectField, TextField } from "../../../components/Field";
import { EmptyState } from "../../../components/dash/EmptyState";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { fmtWhen } from "../../../lib/backups";
import { fmtAgo } from "../../../lib/format";
import { summary } from "../../../lib/bench";
import { CopyDialog } from "../Bench/CopyDialog";
import type { DeviceRef } from "../slots";
import { AwaitingTable, History, historyOf, StagedTable } from "./ChangeTable";
import styles from "./Changes.module.css";

export { summary };

export function ChangesSegment({ d }: { d: DeviceRef }) {
  const id = d.device?.id || d.connected?.id || null;
  const changes = useStore((s) => s.changes);
  const all = useStore((s) => s.allChanges);
  const open = useStore((s) => s.openApply);
  const [editing, setEditing] = useState(false);
  const [copying, setCopying] = useState(false);
  const mine = changes.filter((c) => c.device === id);
  const ours = all.filter((c) => c.device === id);
  const awaiting = ours.filter((c) => c.status === "applied");
  const history = historyOf(ours);
  const ready = mine.filter((c) => c.status === "ready" || c.status === "try").length;
  const verified = history.filter((c) => c.status === "verified").length;
  const last = history.find((c) => c.status === "verified")?.history?.at(-1)?.at;
  const canStage = !!d.device;
  const fc = d.kind === "fc";
  return (
    <>
      <StatRow label="Changes summary">
        <Stat label="Staged" value={mine.length} />
        <Stat label="Ready to apply" value={ready} />
        <Stat label="Verified" value={verified} />
        <Stat label="Last applied" value={fmtAgo(last)} sub={last ? fmtWhen(last) : "Never"} />
      </StatRow>
      <SectionCard
        title="Staged"
        status={!canStage && <span className={styles.muted}>Save this device first.</span>}
        actions={
          <>
            {fc && (
              <Button size="sm" disabled={!canStage || !d.device?.last_backup} onClick={() => setCopying(true)}>
                Copy Settings…
              </Button>
            )}
            {fc && (
              <Button size="sm" disabled={!canStage} onClick={() => setEditing(true)}>
                Edit Setting…
              </Button>
            )}
            <Button size="sm" variant="primary" disabled={!ready} onClick={() => id && open(id)}>
              {ready > 1 ? `Review ${ready} Changes…` : "Review…"}
            </Button>
          </>
        }
      >
        {mine.length === 0 ? <EmptyState icon="sliders">No staged changes.</EmptyState> : <StagedTable rows={mine} label="Staged changes" present />}
      </SectionCard>
      {awaiting.length > 0 && (
        <SectionCard title="Applied, waiting for Keep or Revert">
          <AwaitingTable rows={awaiting} label="Applied changes waiting" />
        </SectionCard>
      )}
      <History rows={history} />
      {editing && id && <EditSetting device={id} onClose={() => setEditing(false)} />}
      {copying && id && <CopyDialog to={id} onClose={() => setCopying(false)} />}
    </>
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
