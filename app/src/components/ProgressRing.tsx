import styles from "./ProgressRing.module.css";

interface Props {
  /** 0 to 1. */
  value: number;
  size?: number;
  label: string;
}

/** A circular progress meter. */
export function ProgressRing({ value, size = 28, label }: Props) {
  const r = 15.5;
  const c = 2 * Math.PI * r;
  const v = Math.min(1, Math.max(0, value));
  return (
    <svg className={styles.ring} viewBox="0 0 36 36" width={size} height={size} role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(v * 100)}>
      <circle className={styles.bg} cx="18" cy="18" r={r} />
      <circle className={styles.fg} cx="18" cy="18" r={r} strokeDasharray={c} strokeDashoffset={c * (1 - v)} />
    </svg>
  );
}
