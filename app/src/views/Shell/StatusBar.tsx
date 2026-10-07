import { useStore } from "../../store";
import { Button } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { connectedName, deviceName, GROUP_OF, GROUPS, STATE_LABEL, worst, type DeviceState } from "../../lib/gear";
import { deviceRefs, pluggedIn, useNow } from "../Gear/refs";
import type { DeviceRef } from "../Gear/slots";
import styles from "./StatusBar.module.css";

const TINT: Partial<Record<DeviceState, "yellow" | "green" | "blue">> = { attention: "yellow", inserted: "yellow", safe: "green", working: "blue" };

/** The bar along the bottom of the window: one item per kind of device plugged in, with its
 *  state. A click opens the device (or the Connected list when there are several). */
export function StatusBar() {
  const s = useStore();
  const now = useNow(s.unmounted.length > 0);
  const plugged = pluggedIn(deviceRefs(s, now));
  const items = GROUPS.map((g) => ({ g, refs: plugged.filter((r) => GROUP_OF[r.kind] === g.id) })).filter((x) => x.refs.length);
  const inserted = plugged.filter((r) => r.state === "inserted");
  return (
    <footer className={styles.bar} aria-label="Gear status" data-component="status-bar">
      {!items.length && <span className={styles.none}>No gear connected</span>}
      {items.map(({ g, refs }) => {
        const state = refs.map((r) => r.state!).reduce(worst);
        const names = refs.map(nameOf).join(", ");
        const label = `${g.label}: ${refs.length > 1 ? `${refs.length} devices, ` : ""}${STATE_LABEL[state]}`;
        const open = () => s.openGear(refs.length === 1 ? { page: "device", key: refs[0].key } : { page: "connected" });
        return (
          <button key={g.id} type="button" className={styles.item} data-state={state} aria-label={label} title={`${names}: ${STATE_LABEL[state]}`} onClick={open}>
            <Icon name={g.icon} size={16} tint={TINT[state]} />
            <span className={styles.word}>{state === "connected" ? g.label : STATE_LABEL[state]}</span>
            {refs.length > 1 && <span className={styles.count}>{refs.length}</span>}
          </button>
        );
      })}
      {inserted.map((r) => (
        <Button key={r.key} size="sm" variant="ghost" className={styles.dismiss} aria-label={`Dismiss the reminder for ${nameOf(r)}`} title="Dismiss reminder" onClick={() => s.dismissReminder(r.connected!)}>
          Dismiss
        </Button>
      ))}
    </footer>
  );
}

const nameOf = (r: DeviceRef) => (r.device ? deviceName(r.device) : connectedName(r.connected!));
