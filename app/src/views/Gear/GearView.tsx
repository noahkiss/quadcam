import { useState } from "react";
import { useStore } from "../../store";
import { api, errText } from "../../ipc/api";
import { sel } from "../../store/settings";
import { ask } from "../../store";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { SelectField, TextField } from "../../components/Field";
import { Icon } from "../../components/Icon";
import { SegmentedControl } from "../../components/SegmentedControl";
import { DeviceHeader } from "../../components/gear/DeviceHeader";
import { toast } from "../../components/toastStore";
import { importFromDevice, prepCard } from "../../actions/session";
import { attentionText, deviceName, KIND_ICON, KIND_LABEL, NOTHING_FOUND, STATE_LABEL } from "../../lib/gear";
import { deviceRefs, pluggedIn, useNow } from "./refs";
import { segmentsFor } from "./segments";
import { GEAR_PAGES } from "./pages";
import type { DeviceRef } from "./slots";
import styles from "./Gear.module.css";

/** The Gear page the sidebar opened. */
export function GearView() {
  const page = useStore((s) => s.gearPage);
  const s = useStore();
  const now = useNow(s.unmounted.length > 0);
  const refs = deviceRefs(s, now);
  if (!page) return null;
  if (page.page === "device") {
    const d = refs.find((r) => r.key === page.key);
    return d ? <DevicePage d={d} /> : <Missing />;
  }
  if (page.page === "slot") {
    const slot = GEAR_PAGES.find((p) => p.id === page.id && (!p.preview || s.values[p.preview] === true));
    return <section className={styles.page}>{slot ? slot.render() : <Missing />}</section>;
  }
  const list = page.page === "connected" ? pluggedIn(refs) : refs.filter((r) => r.device);
  return (
    <section className={styles.page} aria-labelledby="gear-title">
      <h2 id="gear-title" className={styles.title}>
        {page.page === "connected" ? "Connected" : "Devices"}
      </h2>
      {list.length ? <DeviceList refs={list} /> : <p className={styles.empty}>{page.page === "connected" ? NOTHING_FOUND : "No devices saved."}</p>}
    </section>
  );
}

function Missing() {
  return (
    <section className={styles.page}>
      <p className={styles.empty}>This device is not connected or saved.</p>
    </section>
  );
}

function DeviceList({ refs }: { refs: DeviceRef[] }) {
  const open = useStore((s) => s.openGear);
  return (
    <ul className={styles.list} aria-label="Devices">
      {refs.map((r) => (
        <li key={r.key}>
          <button type="button" className={styles.row} onClick={() => open({ page: "device", key: r.key })} data-state={r.state || "away"}>
            <Icon name={KIND_ICON[r.kind]} size={20} />
            <span className={styles.rowName}>{r.device ? deviceName(r.device) : KIND_LABEL[r.kind]}</span>
            <span className={styles.rowKind}>{KIND_LABEL[r.kind]}</span>
            <span className={styles.rowState}>{rowState(r)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/** The state word for a row: what it needs, else its state, else "Not connected". */
const rowState = (r: DeviceRef) => (r.state === "attention" && r.connected && !r.device ? attentionText(r.connected) : r.state ? STATE_LABEL[r.state] : "Not connected");

function DevicePage({ d }: { d: DeviceRef }) {
  const segment = useStore((s) => s.gearSegment);
  const setSegment = useStore((s) => s.setGearSegment);
  const dismiss = useStore((s) => s.dismissReminder);
  const mountCard = useStore((s) => s.mountCard);
  const unmountCard = useStore((s) => s.unmountCard);
  const browsing = useStore((s) => s.gear?.mounted?.find((m) => m.device === d.device?.id || m.device === d.connected?.id));
  const [editing, setEditing] = useState(false);
  const segs = segmentsFor(d);
  const cur = segs.find((x) => x.id === segment) || segs[0];
  const name = d.device ? deviceName(d.device) : KIND_LABEL[d.kind];
  const canSave = !!(d.device || d.connected?.id);
  // Radio cards, DVR cards and goggles cards mount for the person to browse; only the last two hold clips.
  const isClipCard = d.kind === "dvr_card" || d.kind === "goggles";
  const isCard = d.kind === "radio" || isClipCard;
  return (
    <section className={styles.page} aria-label={name}>
      <DeviceHeader
        kind={d.kind}
        name={name}
        state={d.state}
        actions={
          <>
            {d.state === "inserted" && (
              <Button size="sm" variant="ghost" onClick={() => dismiss(d.connected!)}>
                Dismiss reminder
              </Button>
            )}
            {isCard && d.unmounted && d.connected?.id && (
              <Button size="sm" variant="ghost" onClick={() => void mountCard(d.connected!.id!)}>
                Mount
              </Button>
            )}
            {isClipCard && d.unmounted && d.connected?.id && (
              <Button size="sm" variant="ghost" onClick={() => void importFromDevice(d.connected!.id!, name)}>
                Import clips…
              </Button>
            )}
            {isClipCard && d.connected && (d.unmounted ? !!d.connected.id : d.connected.link.kind === "volume") && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void prepCard(d.unmounted ? { device: d.connected!.id! } : { mount: d.connected!.link.kind === "volume" ? d.connected!.link.mount : undefined })}
              >
                Prepare card…
              </Button>
            )}
            {isCard && browsing && (
              <>
                <span className={styles.rowState}>Mounted until {new Date(browsing.until).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</span>
                <Button size="sm" variant="ghost" onClick={() => void unmountCard(browsing.device)}>
                  Done
                </Button>
              </>
            )}
            {canSave && (
              <Button size="sm" variant={d.device ? "ghost" : "primary"} onClick={() => setEditing(true)}>
                {d.device ? "Edit…" : "Save…"}
              </Button>
            )}
            {d.device && (
              <Button size="sm" variant="danger-ghost" onClick={() => forget(d)}>
                Forget…
              </Button>
            )}
          </>
        }
      />
      {segs.length > 1 && <SegmentedControl label="Sections" value={cur.id} onChange={setSegment} segments={segs.map((x) => ({ value: x.id, label: x.label }))} />}
      <div className={styles.segment}>{cur.render(d)}</div>
      {editing && <SaveDevice d={d} onClose={() => setEditing(false)} />}
    </section>
  );
}

async function forget(d: DeviceRef) {
  const dev = d.device!;
  const ok = await ask(`Forget ${deviceName(dev)}?`, "Its backups stay.", { ok: "Forget", danger: true });
  if (ok !== true) return;
  try {
    await api.gearDeviceForget(dev.id);
  } catch (e) {
    toast(errText(e), true);
  }
}

/** Names a device and links it to an aircraft: the small sheet for a new device, and Edit.
 *  A radio flies many aircraft, so its Overview lists them instead. */
function SaveDevice({ d, onClose }: { d: DeviceRef; onClose: () => void }) {
  const profiles = useStore(sel.profiles);
  const [name, setName] = useState(d.device?.name || "");
  const [aircraft, setAircraft] = useState(d.device?.aircraft || "");
  const id = d.device?.id || d.connected?.id || "";
  const isRadio = d.kind === "radio";
  const close = async (v: string) => {
    if (v === "save") {
      try {
        await api.gearDeviceSave(id, name.trim(), isRadio ? null : aircraft);
      } catch (e) {
        toast(errText(e), true);
      }
    }
    onClose();
  };
  return (
    <Dialog
      open
      title={d.device ? "Edit device" : `Save this ${KIND_LABEL[d.kind]}`}
      onClose={close}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="save" variant="primary">
            Save
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <TextField label="Name" placeholder={`Unnamed ${KIND_LABEL[d.kind]}`} value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        {!isRadio && (
          <SelectField label="Aircraft" value={aircraft} onChange={(e) => setAircraft(e.target.value)}>
            <option value="">None</option>
            {profiles.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </SelectField>
        )}
      </div>
    </Dialog>
  );
}
