// The Controls page: the radio in USB Joystick mode, live. Its sticks in the person's stick
// mode, its channel outputs and buttons, and the switch map marking what each control does
// as it moves. The radio is read natively (`gear_radio_watch`): the web view's Gamepad API
// does not see it.
import { Sticks, StickModePicker } from "../../../components/gear/Sticks";
import { Banner } from "../../../components/Banner";
import { BUTTONS, live as liveOf, sticksFrom, DEFAULT_STICKS } from "../../../lib/controls";
import { useStore } from "../../../store";
import { sel } from "../../../store/settings";
import { useRadio, useSwitchMap } from "./hooks";
import { MapSourceBar, SwitchMapView } from "./SwitchMapView";
import styles from "./Switches.module.css";

export function ControlsPage() {
  const radio = useRadio();
  const mode = useStore(sel.stickMode);
  const save = useStore((s) => s.saveSetting);
  const m = useSwitchMap();
  const frame = radio?.frame ?? null;
  const sticks = frame ? sticksFrom(frame.channels, m.map?.sticks.length ? m.map.sticks : DEFAULT_STICKS) : null;
  const live = m.map && frame ? liveOf(m.map, "radio", frame.channels) : null;

  return (
    <div className={styles.controls}>
      <h2 className={styles.title}>Controls</h2>
      <p className={styles.status} role="status" aria-label="Radio">
        {radio?.connected ? (radio.product ?? "Radio") : radio ? "No radio in USB Joystick mode. Plug the radio in and choose USB Joystick on it." : "Looking for the radio…"}
      </p>
      <section className={styles.panel} aria-label="Sticks">
        <div className={styles.bar}>
          <StickModePicker mode={mode} onChange={(v) => void save("stickMode", v)} />
        </div>
        <Sticks values={sticks} mode={mode} />
      </section>
      <section className={styles.panel} aria-label="Channels">
        <h3 className={styles.h3}>Channels</h3>
        <ul className={styles.channels}>
          {(frame?.channels ?? Array<number>(8).fill(1500)).map((us, i) => (
            <li key={i}>
              <span className={styles.ch}>CH{i + 1}</span>
              <span className={styles.meter} role="meter" aria-label={`CH${i + 1}`} aria-valuemin={988} aria-valuemax={2012} aria-valuenow={us} aria-valuetext={frame ? `${us} µs` : "no input"}>
                <span className={styles.fill} style={{ width: `${Math.max(0, Math.min(100, ((us - 988) / 1024) * 100))}%` }} />
              </span>
              <span className={styles.num}>{frame ? us : ""}</span>
            </li>
          ))}
        </ul>
        <ul className={styles.buttons} aria-label="Buttons">
          {Array.from({ length: BUTTONS }, (_, i) => {
            const pressed = !!frame && ((frame.buttons >>> i) & 1) === 1;
            return (
              <li key={i} data-on={pressed ? "" : undefined} aria-label={`Button ${i + 1}${pressed ? ", on" : ""}`}>
                {i + 1}
              </li>
            );
          })}
        </ul>
      </section>
      <section className={styles.panel} aria-label="Switch map">
        <h3 className={styles.h3}>Switch map</h3>
        <MapSourceBar src={m.src} map={m.map} openCard={m.openCard} openModel={m.openModel} openDump={m.openDump} />
        {m.error && (
          <Banner kind="error" icon="danger-triangle" tint="red">
            {m.error}
          </Banner>
        )}
        {m.map ? <SwitchMapView map={m.map} live={live} /> : !m.error && <p className={styles.empty}>Open the radio's card or model file to see what each control does as it moves.</p>}
      </section>
    </div>
  );
}
