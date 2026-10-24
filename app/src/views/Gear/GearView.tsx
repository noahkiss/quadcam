import { useState } from "react";
import { useStore } from "../../store";
import { Button } from "../../components/Button";
import { EmptyState } from "../../components/dash/EmptyState";
import { MoreMenu } from "../../components/dash/MoreMenu";
import { SegmentedControl } from "../../components/SegmentedControl";
import { DeviceHeader } from "../../components/gear/DeviceHeader";
import { importFromDevice, prepCard } from "../../actions/session";
import { deviceName, KIND_LABEL } from "../../lib/gear";
import { deviceRefs, pluggedIn, useNow, useRefsInput } from "./refs";
import { segmentsFor } from "./segments";
import { GEAR_PAGES } from "./pages";
import { ConnectedPage, DevicesPage, forgetDevice, fwFact } from "./DeviceLists";
import { SaveDevice } from "./SaveDevice";
import type { DeviceRef } from "./slots";
import styles from "./Gear.module.css";

/** The Gear page the sidebar opened. */
export function GearView() {
  const page = useStore((s) => s.gearPage);
  const values = useStore((s) => s.values);
  const input = useRefsInput();
  const now = useNow(input.unmounted.length > 0);
  const refs = deviceRefs(input, now);
  if (!page) return null;
  if (page.page === "device") {
    const d = refs.find((r) => r.key === page.key);
    return d ? <DevicePage d={d} /> : <Missing />;
  }
  if (page.page === "slot") {
    const slot = GEAR_PAGES.find((p) => p.id === page.id && (!p.preview || values[p.preview] === true));
    return slot ? <section className={styles.page}>{slot.render()}</section> : <Missing />;
  }
  return (
    <section className={styles.page} aria-labelledby="gear-title">
      {page.page === "connected" ? <ConnectedPage refs={pluggedIn(refs)} /> : <DevicesPage refs={refs.filter((r) => r.device)} />}
    </section>
  );
}

function Missing() {
  return (
    <section className={styles.page}>
      <EmptyState icon="danger-triangle">This device is not connected or saved.</EmptyState>
    </section>
  );
}

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
        facts={[fwFact(d), browsing && `Mounted until ${new Date(browsing.until).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`]}
        actions={
          <>
            {d.state === "inserted" && (
              <Button size="sm" onClick={() => dismiss(d.connected!)}>
                Dismiss reminder
              </Button>
            )}
            {isCard && d.unmounted && d.connected?.id && (
              <Button size="sm" onClick={() => void mountCard(d.connected!.id!)}>
                Mount
              </Button>
            )}
            {isClipCard && d.unmounted && d.connected?.id && (
              <Button size="sm" onClick={() => void importFromDevice(d.connected!.id!, name)}>
                Import clips…
              </Button>
            )}
            {isClipCard && d.connected && (d.unmounted ? !!d.connected.id : d.connected.link.kind === "volume") && (
              <Button
                size="sm"
               
                onClick={() => void prepCard(d.unmounted ? { device: d.connected!.id! } : { mount: d.connected!.link.kind === "volume" ? d.connected!.link.mount : undefined })}
              >
                Prepare card…
              </Button>
            )}
            {isCard && browsing && (
              <Button size="sm" onClick={() => void unmountCard(browsing.device)}>
                Done
              </Button>
            )}
            {canSave && (
              <Button size="sm" variant={d.device ? "secondary" : "primary"} onClick={() => setEditing(true)}>
                {d.device ? "Edit…" : "Save…"}
              </Button>
            )}
            {d.device && <MoreMenu items={[{ label: "Forget…", icon: "trash-bin-trash", danger: true, run: () => void forgetDevice(d) }]} />}
          </>
        }
      />
      {segs.length > 1 && <SegmentedControl label="Sections" value={cur.id} onChange={setSegment} segments={segs.map((x) => ({ value: x.id, label: x.label }))} />}
      <div className={styles.segment}>{cur.render(d)}</div>
      {editing && <SaveDevice d={d} onClose={() => setEditing(false)} />}
    </section>
  );
}
