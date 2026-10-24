// The Connected and Devices pages: the devices plugged in now, as cards with each one's
// facts and next step, and every saved device, as a table grouped by kind.
import { useState } from "react";
import { ask, useStore } from "../../store";
import { api, errText } from "../../ipc/api";
import { Button } from "../../components/Button";
import { Badge } from "../../components/dash/Badge";
import { DataTable, type Column } from "../../components/dash/DataTable";
import { EmptyState } from "../../components/dash/EmptyState";
import { MoreMenu } from "../../components/dash/MoreMenu";
import { PageHeader } from "../../components/dash/PageHeader";
import { StatusDot, type StatusTone } from "../../components/dash/StatusDot";
import { SummaryLine } from "../../components/dash/SummaryLine";
import type { MenuItem } from "../../components/Menu";
import { Icon } from "../../components/Icon";
import { toast } from "../../components/toastStore";
import { importFrom, importFromDevice, prepCard } from "../../actions/session";
import { jobFor, jobText } from "../../lib/backups";
import { attentionText, deviceName, KIND_ICON, KIND_LABEL, NOTHING_FOUND, STATE_LABEL, type DeviceState } from "../../lib/gear";
import { fmtAgo, fmtBytes, plural } from "../../lib/format";
import type { DeviceKind } from "../../ipc/types";
import { identityOf } from "./Overview";
import { gearSlots, type DeviceRef } from "./slots";
import { SaveDevice } from "./SaveDevice";
import styles from "./Gear.module.css";

const TONE: Record<DeviceState, StatusTone> = { attention: "warn", inserted: "warn", working: "idle", safe: "ok", connected: "ok" };

const refName = (r: DeviceRef) => (r.device ? deviceName(r.device) : KIND_LABEL[r.kind]);

/** The state word for a device: what it needs, else its state, else "Not connected". */
export const stateText = (r: DeviceRef) => (r.state === "attention" && r.connected && !r.device ? attentionText(r.connected) : r.state ? STATE_LABEL[r.state] : "Not connected");

/** A plugged-in device's state as a dot and a word; a saved one that is away, in muted text. */
const stateDot = (r: DeviceRef) => (r.state ? <StatusDot tone={TONE[r.state]}>{stateText(r)}</StatusDot> : <span className={styles.away}>{stateText(r)}</span>);

const needsPerson = (r: DeviceRef) => r.state === "attention" || r.state === "inserted";

/** The firmware fact: "Betaflight 4.5.1", "EdgeTX 2.10.5". */
export function fwFact(d: DeviceRef) {
  const id = identityOf(d);
  return [id.firmware, id.version].filter(Boolean).join(" ") || null;
}

/** Forgets a saved device after a question; its backups stay. */
export async function forgetDevice(d: DeviceRef) {
  const dev = d.device!;
  const ok = await ask(`Forget ${deviceName(dev)}?`, "Its backups stay.", { ok: "Forget", danger: true });
  if (ok !== true) return;
  try {
    await api.gearDeviceForget(dev.id);
  } catch (e) {
    toast(errText(e), true);
  }
}

/** Whether a device can back up now: a radio's card mounted, or an FC on its serial port. */
export const canBackUp = (d: DeviceRef) => !!d.connected && !d.unmounted && ((d.kind === "radio" && d.connected.link.kind === "volume") || (d.kind === "fc" && d.connected.link.kind === "serial"));

/** Backs a device up now and says what it read. */
export async function backUpNow(d: DeviceRef) {
  const link = d.connected!.link;
  try {
    const r = await api.gearBackup(link.kind === "serial" ? { port: link.port } : { mount: link.kind === "volume" ? link.mount : null });
    toast(r.report.new ? `Backed up: ${plural(r.report.read, "file")} read.` : "No changes since the last backup.");
  } catch (e) {
    toast(errText(e), true);
  }
}

/** The devices plugged in now: a summary line, then a card per device, those that need the
 *  person first. */
export function ConnectedPage({ refs }: { refs: DeviceRef[] }) {
  useStore((s) => s.changes); // the staged counts follow the changes
  const attention = refs.filter(needsPerson).length;
  const working = refs.filter((r) => r.state === "working").length;
  const staged = refs.reduce((n, r) => n + (r.device ? gearSlots.stagedFor(r.device.id) : 0), 0);
  const sorted = [...refs].sort((a, b) => Number(needsPerson(b)) - Number(needsPerson(a)));
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Connected"
        subtitle={
          <SummaryLine
            parts={[
              `${refs.length} plugged in`,
              attention > 0 && { text: `${attention} ${attention === 1 ? "needs" : "need"} attention`, tone: "warn" },
              working > 0 && `${working} working`,
              staged > 0 && `${staged} staged`,
            ]}
          />
        }
      />
      {refs.length ? (
        <ul className={styles.cards} aria-label="Connected devices">
          {sorted.map((r) => (
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

/** One plugged-in device: its name (opens its page), state and `…` menu; a line of facts
 *  (kind, firmware, last backup, card space); its next step with one button. */
function DeviceCard({ r }: { r: DeviceRef }) {
  const open = useStore((s) => s.openGear);
  const gear = useStore((s) => s.gear);
  const cards = useStore((s) => s.cards);
  const mountCard = useStore((s) => s.mountCard);
  const [saving, setSaving] = useState(false);
  const name = refName(r);
  const c = r.connected!;
  const failed = gearSlots.attention(c);
  const staged = r.device ? gearSlots.stagedFor(r.device.id) : 0;
  const job = jobFor(gear, c.id);
  const mount = c.link.kind === "volume" && !r.unmounted ? c.link.mount : null;
  const isClipCard = r.kind === "dvr_card" || r.kind === "goggles";
  const isCard = r.kind === "radio" || isClipCard;
  const fresh = mount && isClipCard ? (cards[mount]?.new ?? 0) : 0;
  let next: { text: string; action?: { label: string; primary?: boolean; run: () => void } } | null = null;
  if (failed) next = { text: failed, action: { label: "Open", run: () => open({ page: "device", key: r.key }) } };
  else if (job) next = { text: jobText(job) };
  else if (!r.device && c.id) next = { text: "", action: { label: "Save…", primary: true, run: () => setSaving(true) } };
  else if (staged) next = { text: staged === 1 ? "1 change ready" : `${staged} changes ready`, action: { label: "Review…", primary: true, run: () => gearSlots.review(r.device!.id) } };
  else if (fresh && mount) next = { text: plural(fresh, "new clip"), action: { label: "Import…", run: () => void importFrom(mount) } };
  else if (r.unmounted && c.id && isClipCard) next = { text: "Unmounted", action: { label: "Import Clips…", run: () => void importFromDevice(c.id!, name) } };
  const space = mount ? cards[mount] : null;
  const free = space?.free != null && space.size > 0 ? `${fmtBytes(space.free)} free of ${fmtBytes(space.size)}` : r.device?.last_space && isCard ? `${fmtBytes(r.device.last_space.free)} free` : null;
  const backup = gearSlots.lastBackup(r.device);
  const ago = backup ? fmtAgo(backup) : null;
  const facts = [name !== KIND_LABEL[r.kind] && KIND_LABEL[r.kind], fwFact(r), ago && (ago === "Now" ? "backed up now" : `backed up ${ago} ago`), free].filter(Boolean).join(" · ");
  const items: (MenuItem | false)[] = [
    { label: "Open", run: () => open({ page: "device", key: r.key }) },
    isCard && r.unmounted && !!c.id && { label: "Mount", run: () => void mountCard(c.id!) },
    isClipCard && (r.unmounted ? !!c.id : c.link.kind === "volume") && { label: "Prepare Card…", run: () => void prepCard(r.unmounted ? { device: c.id! } : { mount: c.link.kind === "volume" ? c.link.mount : undefined }) },
    !!(r.device || c.id) && { label: r.device ? "Edit…" : "Save…", run: () => setSaving(true) },
  ];
  const menu: (MenuItem | null)[] = items.filter((x): x is MenuItem => !!x);
  if (r.device) menu.push(null, { label: "Forget…", icon: "trash-bin-trash", danger: true, run: () => void forgetDevice(r) });
  return (
    <article className={styles.card} aria-label={name}>
      <header className={styles.cardHead}>
        <Icon name={KIND_ICON[r.kind]} size={20} />
        <button type="button" className={styles.cardName} onClick={() => open({ page: "device", key: r.key })}>
          {name}
        </button>
        {stateDot(r)}
        <MoreMenu label={`More actions for ${name}`} items={menu} />
      </header>
      {facts && <p className={styles.cardFacts}>{facts}</p>}
      <div className={styles.cardBody}>
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

/** The Devices page's groups, in order. */
const KIND_GROUP: Record<DeviceKind, string> = { radio: "Radios", fc: "Quads", goggles: "Goggles and cards", dvr_card: "Goggles and cards", elrs_tx: "ExpressLRS", elrs_rx: "ExpressLRS" };
const GROUP_ORDER = ["Radios", "Quads", "Goggles and cards", "ExpressLRS"];

/** The aircraft a device belongs to: its own link, or a radio's models. */
const aircraftOf = (r: DeviceRef) => r.device?.aircraft || (r.device?.radio_aircraft ?? []).map((a) => a.profile).join(", ") || "";

/** Every saved device as a table grouped by kind: its aircraft, state, last backup, firmware
 *  (with an Update badge that opens Firmware) and staged changes, and a `…` menu per row. */
export function DevicesPage({ refs }: { refs: DeviceRef[] }) {
  const open = useStore((s) => s.openGear);
  const firmware = useStore((s) => s.firmware);
  useStore((s) => s.changes);
  const [editing, setEditing] = useState<DeviceRef | null>(null);
  const rows = [...refs].sort((a, b) => GROUP_ORDER.indexOf(KIND_GROUP[a.kind]) - GROUP_ORDER.indexOf(KIND_GROUP[b.kind]));
  const fwOf = (r: DeviceRef) => firmware?.devices.find((f) => f.device === r.device?.id);
  const updates = refs.filter((r) => fwOf(r)?.state === "update").length;
  const connected = refs.filter((r) => r.connected).length;
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
    { key: "aircraft", header: "Aircraft", optional: true, stack: "hide", cell: (r) => aircraftOf(r) || <span className={styles.away}>—</span> },
    { key: "state", header: "State", stack: "top", cell: stateDot },
    { key: "backup", header: "Last backup", numeric: true, cell: (r) => fmtAgo(gearSlots.lastBackup(r.device)) },
    {
      key: "firmware",
      header: "Firmware",
      numeric: true,
      cell: (r) => {
        const fw = fwOf(r);
        const installed = fw?.installed ?? r.device?.identity?.version ?? null;
        return (
          <span className={styles.fw}>
            {installed ?? "—"}
            {fw?.state === "update" && fw.latest && (
              <button type="button" className={styles.badgeButton} title={`Update to ${fw.latest}`} aria-label={`Update ${refName(r)} to ${fw.latest}`} onClick={() => open({ page: "slot", id: "firmware" })}>
                <Badge tone="warn">Update</Badge>
              </button>
            )}
          </span>
        );
      },
    },
    { key: "staged", header: "Staged", numeric: true, width: "5rem", cell: (r) => (r.device ? gearSlots.stagedFor(r.device.id) : 0) },
    {
      key: "actions",
      header: <span className={styles.hidden}>Actions</span>,
      width: "2.5rem",
      cell: (r) => (
        <MoreMenu
          label={`More actions for ${refName(r)}`}
          items={[
            { label: "Open", run: () => open({ page: "device", key: r.key }) },
            { label: "Back Up Now", disabled: !canBackUp(r), run: () => void backUpNow(r) },
            { label: "Edit…", run: () => setEditing(r) },
            null,
            { label: "Forget…", icon: "trash-bin-trash", danger: true, run: () => void forgetDevice(r) },
          ]}
        />
      ),
    },
  ];
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Devices"
        subtitle={refs.length > 0 && <SummaryLine parts={[`${refs.length} saved`, `${connected} connected`, updates > 0 && { text: `${plural(updates, "firmware update")}`, tone: "warn" }]} />}
      />
      <DataTable label="Devices" columns={columns} rows={rows} rowKey={(r) => r.key} group={(r) => KIND_GROUP[r.kind]} empty={<EmptyState icon="list">No devices saved.</EmptyState>} />
      {editing && <SaveDevice d={editing} onClose={() => setEditing(null)} />}
    </>
  );
}
