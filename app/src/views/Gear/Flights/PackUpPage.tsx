import { api } from "../../../ipc/api";
import type { RowState } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Icon, type IconName } from "../../../components/Icon";
import { ROW_LABEL } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import g from "../Gear.module.css";
import styles from "./Flights.module.css";

const ICON: Record<RowState, [IconName, "green" | "yellow" | undefined]> = {
  pass: ["check-circle", "green"],
  warn: ["danger-triangle", "yellow"],
  unknown: ["info", undefined],
};

/** Gear > Pack up: the read-only checks before a flying session. */
export function PackUpPage() {
  const { data, error, reload } = useGearData(() => api.gearPreflight());
  return (
    <>
      <div className={styles.bar}>
        <h2 className={g.title}>Pack up</h2>
        <span className={styles.grow} />
        <Button icon="refresh" onClick={reload}>
          Check again
        </Button>
      </div>
      {error && <p className={styles.muted}>{error}</p>}
      {data && (
        <ul className={styles.rows} aria-label="Checks">
          {data.rows.map((r) => (
            <li key={r.id} data-state={r.state}>
              <Icon name={ICON[r.state][0]} tint={ICON[r.state][1]} />
              <b>
                {r.label}
                <span className="visually-hidden">: {ROW_LABEL[r.state]}</span>
              </b>
              <span className="selectable">{r.detail}</span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
