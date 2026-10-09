// The Bench (design 7.7): the changes staged for each device, in the order the next
// session applies them. Per device: its status (plugged in, unmounted, away), the plug-in
// prompt, each change with its status control, Review, Keep and Revert for an applied Try
// change, and the history. Copy settings opens the copy dialog; Copy as Markdown puts the
// queue on the clipboard. Nothing here writes a device: Review opens the apply sheet.
import { useState } from "react";
import { useStore } from "../../../store";
import { Button } from "../../../components/Button";
import { Icon } from "../../../components/Icon";
import { SelectField } from "../../../components/Field";
import { toast } from "../../../components/toastStore";
import { fmtWhen } from "../../../lib/backups";
import { benchGroups, benchMarkdown, STATUS_HINT, STATUS_LABEL, summary, WAITING, type BenchGroup } from "../../../lib/bench";
import { KIND_ICON, KIND_LABEL } from "../../../lib/gear";
import type { StagedChange } from "../../../ipc/types";
import { CopyDialog } from "./CopyDialog";
import styles from "./Bench.module.css";
import page from "../Gear.module.css";

const SETTABLE: StagedChange["status"][] = ["draft", "ready", "try", "read_first"];

/** The ids of the cards and FCs that are in the Mac now. */
function presentIds(connected: { id?: string | null }[], unmounted: { id?: string | null }[]) {
  const ids = (l: { id?: string | null }[]) => new Set(l.map((c) => c.id).filter((x): x is string => !!x));
  return { mounted: ids(connected), unmounted: ids(unmounted) };
}

export function BenchPage() {
  const all = useStore((s) => s.allChanges);
  const devices = useStore((s) => s.devices);
  const connected = useStore((s) => s.gear?.connected || []);
  const unmounted = useStore((s) => s.unmounted);
  const [copying, setCopying] = useState(false);
  const { mounted, unmounted: away } = presentIds(connected, unmounted);
  const groups = benchGroups(all, devices, mounted, away);
  const history = all.filter((c) => !WAITING.includes(c.status) && c.status !== "applied").slice().reverse();
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(benchMarkdown(groups));
      toast("Copied the Bench as Markdown.");
    } catch {
      toast("Could not copy to the clipboard.", true);
    }
  };
  return (
    <section className={page.page} aria-labelledby="bench-title">
      <h2 id="bench-title" className={page.title}>
        Bench
      </h2>
      <div className={styles.bar}>
        <Button size="sm" variant="primary" onClick={() => setCopying(true)} disabled={devices.filter((d) => d.kind === "fc").length < 2}>
          Copy settings…
        </Button>
        <Button size="sm" onClick={copy} disabled={!groups.length}>
          Copy as Markdown
        </Button>
      </div>
      {groups.length === 0 ? (
        <p className={page.empty}>Nothing staged. Stage changes from a device page, the CLI or an agent.</p>
      ) : (
        <ul className={styles.groups} aria-label="Devices with changes">
          {groups.map((g) => (
            <li key={g.device}>
              <Group g={g} />
            </li>
          ))}
        </ul>
      )}
      <History rows={history} names={Object.fromEntries(devices.map((d) => [d.id, d.name || KIND_LABEL[d.kind]]))} />
      {copying && <CopyDialog onClose={() => setCopying(false)} />}
    </section>
  );
}

function Group({ g }: { g: BenchGroup }) {
  const open = useStore((s) => s.openApply);
  const setStatus = useStore((s) => s.setChangeStatus);
  const discard = useStore((s) => s.discardChange);
  const keep = useStore((s) => s.keepChange);
  const revert = useStore((s) => s.revertChange);
  const state = g.mounted ? "Plugged in" : g.unmounted ? "Unmounted, still in" : "Not connected";
  return (
    <section className={styles.group} aria-label={g.name} data-state={g.present ? "present" : "away"}>
      <div className={styles.head}>
        <Icon name={KIND_ICON[g.kind]} size={20} />
        <h3 className={styles.name}>{g.name}</h3>
        <span className={styles.state} data-state={g.present ? "present" : "away"}>
          {state}
        </span>
        <Button size="sm" variant="primary" disabled={g.ready === 0 || !g.present} onClick={() => void open(g.device)}>
          {g.ready > 1 ? `Review ${g.ready} changes…` : "Review…"}
        </Button>
      </div>
      {g.next && (
        <p className={styles.muted}>
          Next session: <strong>{g.next.title}</strong>
        </p>
      )}
      {g.prompt && (
        <p className={styles.prompt} role="status">
          <Icon name="signal" tint="yellow" /> {g.prompt}
        </p>
      )}
      {g.staged.length > 0 && (
        <ul className={styles.list} aria-label={`Staged changes for ${g.name}`}>
          {g.staged.map((c) => (
            <li key={c.id} className={styles.item}>
              <div className={styles.row}>
                <span className={styles.title}>{c.title}</span>
                <SelectField
                  label={<span className="visually-hidden">Status of {c.title}</span>}
                  hint={undefined}
                  className={styles.status}
                  value={c.status}
                  title={STATUS_HINT[c.status]}
                  onChange={(e) => void setStatus(c.id, e.target.value as StagedChange["status"])}
                >
                  {SETTABLE.map((s) => (
                    <option key={s} value={s}>
                      {STATUS_LABEL[s]}
                    </option>
                  ))}
                </SelectField>
                <Button size="sm" variant="ghost" disabled={!g.present || c.status === "draft" || c.status === "read_first"} onClick={() => void open(g.device, c.id)}>
                  Review…
                </Button>
                <Button size="sm" variant="danger-ghost" onClick={() => void discard(c.id)}>
                  Discard
                </Button>
              </div>
              <code className={`${styles.code} mono selectable`}>{summary(c)}</code>
              {c.status === "read_first" && <span className={styles.muted}>Read the real value on the device, then set this to Ready.</span>}
              <span className={styles.muted}>{c.editor === "agent" ? "Staged by an agent" : "Staged by you"}</span>
            </li>
          ))}
        </ul>
      )}
      {g.awaiting.length > 0 && (
        <ul className={styles.list} aria-label={`Applied changes for ${g.name}`}>
          {g.awaiting.map((c) => (
            <li key={c.id} className={styles.item}>
              <div className={styles.row}>
                <span className={styles.title}>{c.title}</span>
                <span className={styles.hstatus}>Applied. Fly it, then decide.</span>
                <Button size="sm" variant="primary" onClick={() => void keep(c.id)}>
                  Keep
                </Button>
                <Button size="sm" variant="danger-ghost" onClick={() => void revert(c.id)}>
                  Revert…
                </Button>
              </div>
              <code className={`${styles.code} mono selectable`}>{summary(c)}</code>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function History({ rows, names }: { rows: StagedChange[]; names: Record<string, string> }) {
  const [shown, setShown] = useState(false);
  return (
    <section aria-label="History" className={styles.history}>
      <Button size="sm" variant="ghost" iconEnd={shown ? "chev-down" : "chev-right"} onClick={() => setShown(!shown)} aria-expanded={shown}>
        History
      </Button>
      {shown &&
        (rows.length ? (
          <ul className={styles.list} aria-label="Applied changes">
            {rows.map((c) => (
              <li key={c.id} className={styles.item}>
                <div className={styles.row}>
                  <span className={styles.title}>{c.title}</span>
                  <span className={styles.muted}>{names[c.device] || c.device}</span>
                  <span className={styles.hstatus} data-state={c.status}>
                    {STATUS_LABEL[c.status]}
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
