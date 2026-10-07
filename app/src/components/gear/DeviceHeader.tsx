import type { ReactNode } from "react";
import type { DeviceKind } from "../../ipc/types";
import { KIND_ICON, KIND_LABEL, STATE_LABEL, type DeviceState } from "../../lib/gear";
import { Icon } from "../Icon";
import styles from "./gear.module.css";

interface Props {
  kind: DeviceKind;
  name: string;
  /** Not given for a device that is not plugged in. */
  state?: DeviceState | null;
  /** Buttons at the end of the row. */
  actions?: ReactNode;
}

/** A device page's header: its icon, name, kind and state. */
export function DeviceHeader({ kind, name, state, actions }: Props) {
  return (
    <header className={styles.header} data-component="device-header">
      <span className={styles.headIcon}>
        <Icon name={KIND_ICON[kind]} size={28} />
      </span>
      <div className={styles.headText}>
        <h2>{name}</h2>
        <span className={styles.sub}>
          {KIND_LABEL[kind]}
          {" · "}
          <span data-state={state || "away"}>{state ? STATE_LABEL[state] : "Not connected"}</span>
        </span>
      </div>
      {actions && <div className={styles.headActions}>{actions}</div>}
    </header>
  );
}
