import { useEffect, useRef, useState } from "react";
import { useStore } from "../../store";
import { Button } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { connectedName, deviceName, GROUP_OF, GROUPS, STATE_LABEL, worst, type DeviceState } from "../../lib/gear";
import { deviceRefs, pluggedIn, useNow } from "../Gear/refs";
import type { DeviceRef } from "../Gear/slots";
import styles from "./StatusBar.module.css";

const TINT: Partial<Record<DeviceState, "yellow" | "green" | "blue">> = { attention: "yellow", inserted: "yellow", safe: "green", working: "blue" };

/** The bar along the bottom of the window: one item per kind of device plugged in, with its
 *  state. A click opens the device (or the Connected list when there are several). With
 *  nothing plugged in it stays, empty but for "No gear connected", so the window does not
 *  change height. A change of state is announced politely. */
export function StatusBar() {
  const s = useStore();
  const now = useNow(s.unmounted.length > 0);
  const plugged = pluggedIn(deviceRefs(s, now));
  const items = GROUPS.map((g) => ({ g, refs: plugged.filter((r) => GROUP_OF[r.kind] === g.id) }))
    .filter((x) => x.refs.length)
    .map((x) => ({ ...x, state: x.refs.map((r) => r.state!).reduce(worst) }));
  const inserted = plugged.filter((r) => r.state === "inserted");
  const said = useAnnounce(items.map(({ g, state }) => [g.label, state] as const));
  return (
    <footer className={styles.bar} aria-label="Gear status" data-component="status-bar">
      {!items.length && <span className={styles.none}>No gear connected</span>}
      {items.map(({ g, refs, state }) => {
        const names = refs.map(nameOf).join(", ");
        const label = `${g.label}: ${refs.length > 1 ? `${refs.length} devices, ` : ""}${STATE_LABEL[state]}`;
        const open = () => s.openGear(refs.length === 1 ? { page: "device", key: refs[0].key } : { page: "connected" });
        return (
          <button key={g.id} type="button" className={styles.item} data-state={state} aria-label={label} title={`${names}: ${STATE_LABEL[state]}`} onClick={open}>
            <Icon name={g.icon} size={16} tint={TINT[state]} />
            <span className={styles.name}>{g.label}</span>
            {refs.length > 1 && <span className={styles.count}>{refs.length}</span>}
            {state !== "connected" && <span className={styles.word}>{STATE_LABEL[state]}</span>}
          </button>
        );
      })}
      {inserted.map((r) => (
        <Button key={r.key} size="sm" variant="ghost" className={styles.dismiss} aria-label={`Dismiss the reminder for ${nameOf(r)}`} title="Dismiss reminder" onClick={() => s.dismissReminder(r.connected!)}>
          Dismiss
        </Button>
      ))}
      <span role="status" aria-live="polite" className={styles.live}>
        {said}
      </span>
    </footer>
  );
}

const nameOf = (r: DeviceRef) => (r.device ? deviceName(r.device) : connectedName(r.connected!));

/** The text for the polite live region: each group whose state changed since the last
 *  render, as "Radio: Safe to unplug", and "Quad: Removed" for a group that left. Nothing at
 *  first: what was plugged in at launch is not news. */
export function announce(before: Map<string, DeviceState> | null, now: readonly (readonly [string, DeviceState])[]): string {
  if (!before) return "";
  const out: string[] = [];
  for (const [name, state] of now) if (before.get(name) !== state) out.push(`${name}: ${STATE_LABEL[state]}`);
  const names = new Set(now.map(([n]) => n));
  for (const name of before.keys()) if (!names.has(name)) out.push(`${name}: Removed`);
  return out.join(". ");
}

function useAnnounce(items: readonly (readonly [string, DeviceState])[]) {
  const key = items.map(([n, st]) => `${n}=${st}`).join("|");
  const last = useRef<Map<string, DeviceState> | null>(null);
  const [said, setSaid] = useState("");
  useEffect(() => {
    const text = announce(last.current, items);
    last.current = new Map(items);
    if (text) setSaid(text);
    // `key` stands for `items`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return said;
}
