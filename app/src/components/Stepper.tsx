import { Fragment } from "react";
import { Icon } from "./Icon";
import styles from "./Stepper.module.css";

interface Props<T extends string> {
  label: string;
  steps: { id: T; label: string }[];
  current: T;
}

/** Numbered steps; finished ones show a check. */
export function Stepper<T extends string>({ label, steps, current }: Props<T>) {
  const at = steps.findIndex((s) => s.id === current);
  return (
    <ol className={styles.stepper} aria-label={label}>
      {steps.map((s, i) => (
        <Fragment key={s.id}>
          {i > 0 && <li className={styles.line} aria-hidden="true" />}
          <li className={i < at ? styles.done : i === at ? styles.on : undefined} aria-current={i === at ? "step" : undefined}>
            <span className={styles.dot}>{i < at ? <Icon name="check" size={12} /> : i + 1}</span>
            {s.label}
          </li>
        </Fragment>
      ))}
    </ol>
  );
}
