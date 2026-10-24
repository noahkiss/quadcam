// The Bench (design 7.7): the changes staged for each device, in the order the next
// session applies them. The summary line, then a card per device, plugged-in devices first
// and open, away ones folded to their staged count: its status, the plug-in prompt, the
// applied Try changes waiting for Keep, and its changes as a table (`Changes/ChangeTable`)
// with the status control and Review; then the history. Copy settings opens the copy
// dialog; Copy as Markdown (in the … menu) puts the queue on the clipboard. Nothing here
// writes a device: Review opens the apply sheet.
import { useState } from "react";
import { useStore } from "../../../store";
import { Button } from "../../../components/Button";
import { Icon } from "../../../components/Icon";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
import { StatusDot } from "../../../components/dash/StatusDot";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { toast } from "../../../components/toastStore";
import { benchGroups, benchMarkdown, type BenchGroup } from "../../../lib/bench";
import { KIND_ICON, KIND_LABEL } from "../../../lib/gear";
import { AwaitingTable, History, historyOf, StagedTable } from "../Changes/ChangeTable";
import { CopyDialog } from "./CopyDialog";
import styles from "./Bench.module.css";

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
  const history = historyOf(all);
  const staged = groups.reduce((n, g) => n + g.staged.length, 0);
  const ready = groups.reduce((n, g) => n + g.ready, 0);
  const awaiting = groups.reduce((n, g) => n + g.awaiting.length, 0);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(benchMarkdown(groups));
      toast("Copied the Bench as Markdown.");
    } catch {
      toast("Could not copy to the clipboard.", true);
    }
  };
  return (
    <>
      <PageHeader
        id="bench-title"
        title="Bench"
        subtitle={
          groups.length > 0 && (
            <SummaryLine
              parts={[
                `${staged} staged`,
                `${ready} ready`,
                `${groups.filter((g) => g.present).length} of ${groups.length} ${groups.length === 1 ? "device" : "devices"} plugged in`,
                awaiting > 0 && { text: `${awaiting} waiting for Keep`, tone: "warn" },
              ]}
            />
          )
        }
        fold={[{ label: "Copy Settings…", variant: "primary", disabled: devices.filter((d) => d.kind === "fc").length < 2, run: () => setCopying(true) }]}
        more={[{ label: "Copy as Markdown", disabled: !groups.length, run: () => void copy() }]}
        moreLabel="Bench actions"
      />
      {groups.length === 0 ? (
        <EmptyState icon="sliders">Nothing staged.</EmptyState>
      ) : (
        [...groups.filter((g) => g.present), ...groups.filter((g) => !g.present)].map((g) => <Group key={g.device} g={g} />)
      )}
      <History rows={history} names={Object.fromEntries(devices.map((d) => [d.id, d.name || KIND_LABEL[d.kind]]))} />
      {copying && <CopyDialog onClose={() => setCopying(false)} />}
    </>
  );
}

/** One device: its state, what the next session applies, the plug-in prompt, the changes
 *  waiting for Keep and the staged table. A device that is away folds to one row with its
 *  staged count until opened. */
function Group({ g }: { g: BenchGroup }) {
  const open = useStore((s) => s.openApply);
  const [shown, setShown] = useState(g.present);
  const state = g.mounted ? "Plugged in" : g.unmounted ? "Unmounted, still in" : "Not connected";
  const folded = !g.present && !shown;
  return (
    <SectionCard
      label={g.name}
      collapse={g.present ? undefined : { open: shown, onToggle: setShown }}
      title={
        <span className={styles.name}>
          <Icon name={KIND_ICON[g.kind]} size={18} />
          {g.name}
        </span>
      }
      status={
        <>
          <StatusDot tone={g.mounted ? "ok" : g.unmounted ? "warn" : "idle"}>{state}</StatusDot>
          {folded && <span>{g.staged.length + g.awaiting.length} staged</span>}
          {folded && g.next && <span>Next session: {g.next.title}</span>}
        </>
      }
      actions={
        <Button size="sm" variant="primary" disabled={g.ready === 0 || !g.present} onClick={() => void open(g.device)}>
          {g.ready > 1 ? `Review ${g.ready} Changes…` : "Review…"}
        </Button>
      }
    >
      <div className={styles.body}>
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
        {g.awaiting.length > 0 && (
          <div className={styles.awaiting} role="group" aria-label="Waiting for Keep">
            <span className={styles.awaitingHead}>Applied, fly it</span>
            <AwaitingTable rows={g.awaiting} label={`Applied changes for ${g.name}`} />
          </div>
        )}
        {g.staged.length > 0 && <StagedTable rows={g.staged} label={`Staged changes for ${g.name}`} present={g.present} />}
      </div>
    </SectionCard>
  );
}
