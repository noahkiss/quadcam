import { Icon } from "../Icon";
import styles from "./gear.module.css";

/** `gear::model::Check`: one guard, passed or refused with a reason. */
export interface CheckItem {
  name: string;
  ok: boolean;
  refusal?: { code: string; reason: string } | null;
}

/** Each guard of an apply plan with a pass mark or the reason it fails. */
export function ChecksList({ checks }: { checks: CheckItem[] }) {
  return (
    <ul className={styles.checks} aria-label="Checks" data-component="checks-list">
      {checks.map((c) => (
        <li key={c.name} data-state={c.ok ? "ok" : "failed"}>
          <Icon name={c.ok ? "check-circle" : "close-circle"} tint={c.ok ? "green" : "red"} />
          <span className={styles.checkName}>{c.name}</span>
          <span className="visually-hidden">{c.ok ? "Passed" : "Failed"}</span>
          {!c.ok && c.refusal && <span className={styles.reason}>{c.refusal.reason}</span>}
        </li>
      ))}
    </ul>
  );
}
