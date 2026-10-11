// The Connected and Devices pages: the devices plugged in now, as cards with each one's
// next step, and every saved device, as a table.
import { useState } from "react";
import { useStore } from "../../store";
import { Button } from "../../components/Button";
import { DataTable, type Column } from "../../components/dash/DataTable";
import { EmptyState } from "../../components/dash/EmptyState";
import { PageHeader } from "../../components/dash/PageHeader";
import { Stat, StatRow } from "../../components/dash/StatRow";
import { StatusDot, type StatusTone } from "../../components/dash/StatusDot";
import { Icon } from "../../components/Icon";
import { importFrom, importFromDevice } from "../../actions/session";
import { jobFor, jobText } from "../../lib/backups";
import { attentionText, deviceName, KIND_ICON, KIND_LABEL, NOTHING_FOUND, STATE_LABEL, type DeviceState } from "../../lib/gear";
import { fmtAgo, plural } from "../../lib/format";
import { gearSlots, type DeviceRef } from "./slots";
import { SaveDevice } from "./SaveDevice";
import styles from "./Gear.module.css";

const TONE: Record<DeviceState, StatusTone> = { attention: "warn", inserted: "warn", working: "idle", safe: "ok", connected: "ok" };

const refName = (r: DeviceRef) => (r.device ? deviceName(r.device) : KIND_LABEL[r.kind]);

/** The state word for a device: what it needs, else its state, else "Not connected". */
export const stateText = (r: DeviceRef) => (r.state === "attention" && r.connected && !r.device ? attentionText(r.connected) : r.state ? STATE_LABEL[r.state] : "Not connected");

const stateDot = (r: DeviceRef) => <StatusDot tone={r.state ? TONE[r.state] : "idle"}>{stateText(r)}</StatusDot>;

/** The devices plugged in now: tiles for what needs the person, then a card per device. */
export function ConnectedPage({ refs }: { refs: DeviceRef[] }) {
  useStore((s) => s.changes); // the staged counts follow the changes
  const attention = refs.filter((r) => r.state === "attention" || r.state === "inserted").length;
  const working = refs.filter((r) => r.state === "working").length;
  const staged = refs.reduce((n, r) => n + (r.device ? gearSlots.stagedFor(r.device.id) : 0), 0);
  return (
    <>
      <PageHeader id="gear-title" title="Connected" />
      <StatRow label="Summary">
        <Stat label="Plugged in" value={refs.length} />
        <Stat label="Need attention" value={attention} tone={attention ? "warn" : undefined} />
        <Stat label="Working" value={working} />
        <Stat label="Staged for these" value={staged} />
      </StatRow>
      {refs.length ? (
        <ul className={styles.cards} aria-label="Connected devices">
          {refs.map((r) => (
            <li key={r.key}>
              <DeviceCard r={r} />
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState icon="signal">{NOTHING_FOUND}</EmptyState>
      )}
    </>
  );
}

/** One plugged-in device: its name (opens its page), kind and state, and its next step. */
function DeviceCard({ r }: { r: DeviceRef }) {
  const open = useStore((s) => s.openGear);
  const gear = useStore((s) => s.gear);
  const cards = useStore((s) => s.cards);
  const [saving, setSaving] = useState(false);
  const name = refName(r);
  const c = r.connected!;
  const failed = gearSlots.attention(c);
  const staged = r.device ? gearSlots.stagedFor(r.device.id) : 0;
  const job = jobFor(gear, c.id);
  const mount = c.link.kind === "volume" && !r.unmounted ? c.link.mount : null;
  const fresh = mount && (r.kind === "dvr_card" || r.kind === "goggles") ? (cards[mount]?.new ?? 0) : 0;
  let next: { text: string; action?: { label: string; primary?: boolean; run: () => void } } | null = null;
  if (failed) next = { text: failed, action: { label: "Open", run: () => open({ page: "device", key: r.key }) } };
  else if (job) next = { text: jobText(job) };
  else if (!r.device && c.id) next = { text: "", action: { label: "Save…", primary: true, run: () => setSaving(true) } };
  else if (staged) next = { text: staged === 1 ? "1 change ready" : `${staged} changes ready`, action: { label: "Review…", primary: true, run: () => gearSlots.review(r.device!.id) } };
  else if (fresh && mount) next = { text: plural(fresh, "new clip"), action: { label: "Import…", run: () => void importFrom(mount) } };
  else if (r.unmounted && c.id && (r.kind === "dvr_card" || r.kind === "goggles")) next = { text: "Unmounted", action: { label: "Import clips…", run: () => void importFromDevice(c.id!, name) } };
  return (
    <article className={styles.card} aria-label={name}>
      <header className={styles.cardHead}>
        <Icon name={KIND_ICON[r.kind]} size={20} />
        <button type="button" className={styles.cardName} onClick={() => open({ page: "device", key: r.key })}>
          {name}
        </button>
        {stateDot(r)}
      </header>
      <div className={styles.cardBody}>
        {name !== KIND_LABEL[r.kind] && <span className={styles.cardKind}>{KIND_LABEL[r.kind]}</span>}
        <span className={styles.cardNext}>{next ? next.text : "Nothing to do"}</span>
        {next?.action && (
          <Button size="sm" variant={next.action.primary ? "primary" : "secondary"} onClick={next.action.run}>
            {next.action.label}
          </Button>
        )}
      </div>
      {saving && <SaveDevice d={r} onClose={() => setSaving(false)} />}
    </article>
  );
}

/** Every saved device as a table: its state, last backup, firmware and staged changes. */
export function DevicesPage({ refs }: { refs: DeviceRef[] }) {
  const open = useStore((s) => s.openGear);
  const firmware = useStore((s) => s.firmware);
  useStore((s) => s.changes);
  const columns: Column<DeviceRef>[] = [
    {
      key: "name",
      header: "Name",
      rowHeader: true,
      cell: (r) => (
        <button type="button" className={styles.rowLink} onClick={() => open({ page: "device", key: r.key })}>
          <Icon name={KIND_ICON[r.kind]} size={16} />
          {refName(r)}
        </button>
      ),
    },
    { key: "kind", header: "Kind", cell: (r) => KIND_LABEL[r.kind], width: "7rem" },
    { key: "state", header: "State", cell: stateDot },
    { key: "backup", header: "Last backup", numeric: true, optional: true, cell: (r) => fmtAgo(gearSlots.lastBackup(r.device)) },
    {
      key: "firmware",
      header: "Firmware",
      numeric: true,
      cell: (r) => {
        const fw = firmware?.devices.find((f) => f.device === r.device?.id);
        const installed = fw?.installed ?? r.device?.identity?.version ?? null;
        if (fw?.state === "update" && fw.latest) return <span className={styles.update}>{`${installed ?? "—"} → ${fw.latest}`}</span>;
        return installed ?? "—";
      },
    },
    { key: "staged", header: "Staged", numeric: true, width: "5rem", cell: (r) => (r.device ? gearSlots.stagedFor(r.device.id) : 0) },
  ];
  return (
    <>
      <PageHeader id="gear-title" title="Devices" />
      <DataTable label="Devices" columns={columns} rows={refs} rowKey={(r) => r.key} empty={<EmptyState icon="list">No devices saved.</EmptyState>} />
    </>
  );
}
