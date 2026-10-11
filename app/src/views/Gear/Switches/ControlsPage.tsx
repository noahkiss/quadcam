// The Controls page: the radio in USB Joystick mode, live. Its sticks in the person's stick
// mode, its channel outputs and buttons, and the switch map marking what each control does
// as it moves. The radio is read natively (`gear_radio_watch`): the web view's Gamepad API
// does not see it.
import { Sticks, StickModePicker } from "../../../components/gear/Sticks";
import { Banner } from "../../../components/Banner";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
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

  const status = radio?.connected ? (radio.product ?? "Radio") : radio ? "No radio in USB Joystick mode" : "Looking for the radio…";
  const pressed = frame ? Array.from({ length: BUTTONS }, (_, i) => ((frame.buttons >>> i) & 1) === 1).filter(Boolean).length : 0;

  return (
    <>
      <PageHeader
        title="Controls"
        subtitle={
          <span role="status" aria-label="Radio">
            {status}
          </span>
        }
      />
      {radio && !radio.connected && (
        <Banner kind="info" icon="info" tint="blue">
          Plug the radio in and choose USB Joystick on it.
        </Banner>
      )}
      <SectionCard title="Sticks" actions={<StickModePicker mode={mode} onChange={(v) => void save("stickMode", v)} />}>
        <Sticks values={sticks} mode={mode} />
      </SectionCard>
      <SectionCard title="Channels">
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
        <details className={styles.more}>
          <summary>
            Buttons{pressed > 0 && <span className={styles.muted}> · {pressed} on</span>}
          </summary>
          <ul className={styles.buttons} aria-label="Buttons">
            {Array.from({ length: BUTTONS }, (_, i) => {
              const on = !!frame && ((frame.buttons >>> i) & 1) === 1;
              return (
                <li key={i} data-on={on ? "" : undefined} aria-label={`Button ${i + 1}${on ? ", on" : ""}`}>
                  {i + 1}
                </li>
              );
            })}
          </ul>
        </details>
      </SectionCard>
      <section className={styles.panel} aria-labelledby="controls-map">
        <h3 id="controls-map" className={styles.h3}>
          Switch map
        </h3>
        <MapSourceBar src={m.src} map={m.map} openCard={m.openCard} openModel={m.openModel} openDump={m.openDump} />
        {m.error && (
          <Banner kind="error" icon="danger-triangle" tint="red">
            {m.error}
          </Banner>
        )}
        {m.map ? <SwitchMapView map={m.map} live={live} /> : !m.error && <EmptyState icon="sd-card">No radio card or model file open.</EmptyState>}
      </section>
    </>
  );
}
