// The radio's two sticks, drawn as gimbal boxes with each axis named and what it does. The
// controls page draws the live radio with it; the built-in sim will too.
import { SegmentedControl } from "../SegmentedControl";
import { AXIS_TEXT, MODE_LAYOUT, pct, STICK_MODES, type Gimbal, type StickMode, type StickValues } from "../../lib/controls";
import styles from "./Sticks.module.css";

interface Props {
  /** Null while there is no radio: the sticks rest in the middle, dimmed. */
  values: StickValues | null;
  mode: StickMode;
  /** The sim's overlay: small boxes with one line of percentages under each. The size follows
   *  `--stick-size` on a parent. */
  compact?: boolean;
}

/** Both sticks in the given mode. */
export function Sticks({ values, mode, compact }: Props) {
  const l = MODE_LAYOUT[mode];
  return (
    <div className={[styles.sticks, compact && styles.compact].filter(Boolean).join(" ")} data-idle={values ? undefined : ""}>
      <GimbalBox side="Left" g={l.left} values={values} compact={compact} />
      <GimbalBox side="Right" g={l.right} values={values} compact={compact} />
    </div>
  );
}

function GimbalBox({ side, g, values, compact }: { side: string; g: Gimbal; values: StickValues | null; compact?: boolean }) {
  const x = values ? values[g.h] : 0;
  const y = values ? values[g.v] : 0;
  const [v, h] = [AXIS_TEXT[g.v], AXIS_TEXT[g.h]];
  const label = `${side} stick: ${v.name} ${values ? pct(y) : "no input"}, ${h.name} ${values ? pct(x) : "no input"}`;
  return (
    <figure className={styles.gimbal} aria-label={`${side} stick`}>
      <div className={styles.frame}>
        <span className={styles.vlabel} aria-hidden="true">
          {v.name}
        </span>
        <div className={styles.box} role="img" aria-label={label}>
          <span className={styles.cross} />
          <span className={styles.dot} style={{ left: `${50 + x * 50}%`, top: `${50 - y * 50}%` }} />
        </div>
      </div>
      <figcaption className={styles.caption}>
        {compact ? (
          <span className={styles.line}>
            {v.name} {values ? pct(y) : "-"} · {h.name} {values ? pct(x) : "-"}
          </span>
        ) : (
          <>
            <span className={styles.side}>{side} stick</span>
            <Axis name={v.name} motion={v.motion} value={values ? pct(y) : null} />
            <Axis name={h.name} motion={h.motion} value={values ? pct(x) : null} />
          </>
        )}
      </figcaption>
    </figure>
  );
}

function Axis({ name, motion, value }: { name: string; motion: string; value: string | null }) {
  return (
    <span className={styles.axis}>
      <strong>{name}</strong>
      <span className={styles.motion}>{motion}</span>
      {value && <span className={styles.value}>{value}</span>}
    </span>
  );
}

/** Mode 1 to 4. */
export function StickModePicker({ mode, onChange }: { mode: StickMode; onChange: (m: StickMode) => void }) {
  return (
    <SegmentedControl
      label="Stick mode"
      size="sm"
      value={String(mode)}
      onChange={(v) => onChange(Number(v) as StickMode)}
      segments={STICK_MODES.map((m) => ({ value: String(m), label: `Mode ${m}` }))}
    />
  );
}
