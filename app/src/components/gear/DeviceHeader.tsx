import type { ReactNode } from "react";
import type { DeviceKind } from "../../ipc/types";
import { KIND_ICON, KIND_LABEL, STATE_LABEL, type DeviceState } from "../../lib/gear";
import { PageHeader } from "../dash/PageHeader";
import styles from "./gear.module.css";

interface Props {
  kind: DeviceKind;
  name: string;
  /** Not given for a device that is not plugged in. */
  state?: DeviceState | null;
  /** More of the subtitle after the state: the firmware, the board. Empty ones are dropped. */
  facts?: (string | null | undefined)[];
  /** Buttons on the right, primary last. */
  actions?: ReactNode;
}

/** A device page's header: the PageHeader with the kind's icon, and kind · state · facts
 *  as its subtitle. */
export function DeviceHeader({ kind, name, state, facts, actions }: Props) {
  const more = (facts || []).filter((f): f is string => !!f);
  return (
    <div className={styles.header} data-component="device-header">
      <PageHeader
        icon={KIND_ICON[kind]}
        title={name}
        subtitle={
          <>
            {KIND_LABEL[kind]}
            {" · "}
            <span data-state={state || "away"}>{state ? STATE_LABEL[state] : "Not connected"}</span>
            {more.map((f) => ` · ${f}`)}
          </>
        }
        actions={actions}
      />
    </div>
  );
}
