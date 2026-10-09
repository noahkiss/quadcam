// The basic OSD over the picture (sim design 9.4): voltage, mAh and the flight timer in the
// bottom row, the mode and arm state above it, and a warning line. The quad's own layout from its
// `diff all` is a later package; this fixed one keeps the bottom two rows of the 16-row grid for
// text, and the stick display sits above them.
import type { SimHud } from "../../../ipc/types";
import { osdMah, osdTimer, osdVoltage, osdWarning } from "../../../lib/simview";
import styles from "./SimPage.module.css";

interface Props {
  hud: SimHud;
  /** Flight time (s). */
  seconds: number;
  radio: boolean;
}

export function Osd({ hud, seconds, radio }: Props) {
  const warning = radio ? osdWarning(hud) : "No radio";
  return (
    <div className={styles.osd} aria-label="OSD" data-component="osd">
      {warning && (
        <span className={styles.warning} data-osd="warning" role="status">
          {warning}
        </span>
      )}
      <span className={styles.state} data-osd="state">
        {hud.mode.toUpperCase()} {hud.armed ? (hud.turtle ? "TURTLE" : "ARMED") : "DISARMED"}
      </span>
      <span className={styles.voltage} data-osd="voltage">
        {osdVoltage(hud.vbat)}
      </span>
      <span className={styles.mah} data-osd="mah">
        {osdMah(hud.mah)}
      </span>
      <span className={styles.timer} data-osd="timer">
        {osdTimer(seconds)}
      </span>
    </div>
  );
}
