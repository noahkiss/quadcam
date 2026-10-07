import { useState } from "react";
import { api, errText } from "../../ipc/api";
import type { LibClip } from "../../ipc/types";
import { Button } from "../../components/Button";
import { Checkbox, TextField } from "../../components/Field";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toastStore";
import { mmss } from "../../lib/flights";
import { fmtT, parseT } from "../../lib/format";
import { useGearData } from "../Gear/Flights/useGearData";
import styles from "./Inspector.module.css";

/** The crashes logged on a clip, and the button to log one at the playhead. */
export function ClipCrashes({ clip: c, at }: { clip: LibClip; at: number | null }) {
  const { data } = useGearData(() => api.gearCrashes(c.id), [c.id]);
  const [logging, setLogging] = useState(false);
  return (
    <section className={styles.crashes} aria-label="Crashes">
      <div className={styles.crashHead}>
        <span className={styles.label}>Crashes</span>
        <Button size="sm" variant="ghost" icon="add" onClick={() => setLogging(true)}>
          Log crash…
        </Button>
      </div>
      {data && data.length === 0 && <p className={styles.muted}>None logged.</p>}
      {data && data.length > 0 && (
        <ul className={styles.crashList}>
          {data.map((x) => (
            <li key={x.id}>
              <b>{x.time_s != null ? mmss(x.time_s) : "–"}</b> {x.broke || "Crash"}
              {x.parts?.length ? <span className={styles.muted}> · {x.parts.join(", ")}</span> : null}
              {x.repaired ? <span className={styles.muted}> · repaired</span> : null}
            </li>
          ))}
        </ul>
      )}
      {logging && <LogCrash clip={c} at={at ?? 0} onClose={() => setLogging(false)} />}
    </section>
  );
}

function LogCrash({ clip: c, at, onClose }: { clip: LibClip; at: number; onClose: () => void }) {
  const [time, setTime] = useState(fmtT(at));
  const [broke, setBroke] = useState("");
  const [parts, setParts] = useState("");
  const [note, setNote] = useState("");
  const [repaired, setRepaired] = useState(false);
  const t = parseT(time);
  const close = async (v: string) => {
    if (v === "save") {
      try {
        await api.gearCrashSave({
          clip: c.id,
          time_s: t,
          broke,
          parts: parts
            .split(",")
            .map((p) => p.trim())
            .filter(Boolean),
          note,
          repaired,
        });
      } catch (e) {
        toast(errText(e), true);
      }
    }
    onClose();
  };
  return (
    <Dialog
      open
      title="Log crash"
      onClose={close}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="save" variant="primary" disabled={t == null}>
            Save
          </Button>
        </>
      }
    >
      <div className={styles.fields}>
        <TextField label="Time in clip" value={time} onChange={(e) => setTime(e.target.value)} />
        <TextField label="What broke" value={broke} onChange={(e) => setBroke(e.target.value)} autoFocus />
        <TextField label="Parts used" placeholder="prop, arm" value={parts} onChange={(e) => setParts(e.target.value)} />
        <TextField label="Note" value={note} onChange={(e) => setNote(e.target.value)} />
        <Checkbox label="Repaired" checked={repaired} onChange={(e) => setRepaired(e.target.checked)} />
      </div>
    </Dialog>
  );
}
