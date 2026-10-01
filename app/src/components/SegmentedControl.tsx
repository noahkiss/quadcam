import { Icon, type IconName } from "./Icon";
import styles from "./SegmentedControl.module.css";

export interface Segment<T extends string> {
  value: T;
  label: string;
  icon?: IconName;
  /** Show only the icon; the label becomes the accessible name. */
  iconOnly?: boolean;
}

interface Props<T extends string> {
  label: string;
  value: T;
  segments: Segment<T>[];
  onChange: (value: T) => void;
  size?: "md" | "sm";
}

/** A row of toggle buttons where one is on. */
export function SegmentedControl<T extends string>({ label, value, segments, onChange, size = "md" }: Props<T>) {
  return (
    <div role="group" aria-label={label} className={[styles.track, styles[size]].join(" ")}>
      {segments.map((s) => (
        <button
          key={s.value}
          type="button"
          aria-pressed={s.value === value}
          aria-label={s.iconOnly ? s.label : undefined}
          title={s.iconOnly ? s.label : undefined}
          className={styles.segment}
          onClick={() => onChange(s.value)}
        >
          {s.icon && <Icon name={s.icon} size={15} />}
          {!s.iconOnly && s.label}
        </button>
      ))}
    </div>
  );
}
