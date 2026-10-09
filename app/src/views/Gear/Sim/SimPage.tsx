// Gear > Sim (sim design 9.2): pick an aircraft and fly it in the plain room with the radio. The
// core runs the physics on its own thread and feeds it the radio (`core/sim_host.rs`); this page
// draws what the core says (`lib/simhost.ts`, `Stage.tsx`). Settings save as they change.
import { useCallback, useEffect, useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Popover } from "../../../components/Popover";
import { api, errText } from "../../../ipc/api";
import type { Calibration, SimPreset, SimStartInfo, SimUiSettings } from "../../../ipc/types";
import type { PerfSummary } from "../../../lib/simhost";
import { useStore } from "../../../store";
import { SimSettings } from "./SimSettings";
import { Stage, type SimView } from "./Stage";
import styles from "./SimPage.module.css";

const NONE: SimUiSettings = {};
const DEFAULT_PROFILE = "meteor75";

/** The saved choices over the profile's camera. */
function resolve(s: SimUiSettings, camera: SimStartInfo["camera"] | null): SimView {
  return {
    view: s.view ?? "fpv",
    aspect: s.aspect ?? camera?.aspect ?? "16:9",
    uptiltDeg: s.uptilt_deg ?? camera?.uptilt_deg ?? 15,
    fovDeg: s.fov_deg ?? camera?.fov_deg ?? 150,
    stickDisplay: s.stick_display ?? true,
    osd: s.osd ?? true,
  };
}

const ms = (x: number) => `${x.toFixed(1)} ms`;

export function SimPage() {
  const saved = useStore((s) => s.values.simSettings) ?? NONE;
  const saveSetting = useStore((s) => s.saveSetting);
  const openGear = useStore((s) => s.openGear);
  const [presets, setPresets] = useState<SimPreset[]>([]);
  const [run, setRun] = useState<SimStartInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [radioNote, setRadioNote] = useState<string | null>(null);
  const [stats, setStats] = useState<PerfSummary | null>(null);
  const [open, setOpen] = useState(false);
  const gear = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    api.simPresets().then(setPresets, (e) => setError(errText(e)));
    // Leaving the page stops the sim; the radio is free for the controls page again.
    return () => void api.simStop().catch(() => {});
  }, []);

  const profile = saved.profile ?? DEFAULT_PROFILE;
  const ui = resolve(saved, run?.camera ?? null);
  const change = useCallback(
    (patch: Partial<SimUiSettings>) => void saveSetting("simSettings", { ...saved, ...patch }),
    [saved, saveSetting],
  );

  async function fly() {
    setError(null);
    let calibration: Calibration | null = null;
    try {
      const c = await api.gearSimCalibration();
      calibration = c.calibration?.calibration ?? null;
      setRadioNote(c.resolution?.product ?? null);
    } catch (e) {
      // No radio in USB Joystick mode yet: the sim still starts, and picks it up when it is plugged in.
      setRadioNote(null);
      void e;
    }
    try {
      setStats(null);
      setRun(await api.simStart({ profile, calibration }));
    } catch (e) {
      setRun(null);
      setError(errText(e));
    }
  }

  async function stop() {
    setRun(null);
    setStats(null);
    await api.simStop().catch(() => {});
  }

  return (
    <div className={styles.page} data-component="sim">
      <div className={styles.head}>
        <h2 className={styles.title}>Sim</h2>
        {run ? (
          <>
            <Button icon="restart" onClick={() => void api.simReset().catch((e) => setError(errText(e)))}>
              Reset
            </Button>
            <Button variant="secondary" icon="pause" onClick={() => void stop()}>
              Stop
            </Button>
          </>
        ) : (
          <Button variant="primary" icon="play" onClick={() => void fly()} disabled={!presets.length}>
            Fly
          </Button>
        )}
        <span className={styles.anchor}>
          <Button ref={gear} icon="sliders" aria-label="Sim settings" aria-expanded={open} aria-haspopup="dialog" onClick={() => setOpen((o) => !o)} />
          {open && (
            <Popover label="Sim settings" anchor={gear} onClose={() => setOpen(false)}>
              <SimSettings
                presets={presets}
                profile={profile}
                ui={ui}
                profileCamera={run ? { uptiltDeg: run.camera.uptilt_deg, fovDeg: run.camera.fov_deg, aspect: run.camera.aspect } : null}
                onChange={change}
              />
            </Popover>
          )}
        </span>
      </div>

      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {run && !run.calibrated && (
        <Banner
          kind="warning"
          icon="radio"
          action={
            <Button size="sm" onClick={() => openGear({ page: "slot", id: "sim-radio" })}>
              Open Sim radio
            </Button>
          }
        >
          No saved calibration for this radio, so the arm, turtle and reset switches do nothing.
        </Banner>
      )}

      {run ? (
        <Stage key={run.host_ns} info={run} ui={ui} onStopped={(why) => (setError(`The sim stopped: ${why}`), setRun(null))} onStats={setStats} />
      ) : (
        <p className={styles.note}>
          Fly the plain room with your radio in USB Joystick mode. Arm with the switch you set in Sim radio, with the throttle down.{radioNote ? ` Radio: ${radioNote}.` : ""}
        </p>
      )}
      {run && stats && stats.frames > 1 && (
        <p className={styles.stats} role="status" aria-label="Timing">
          Frame {ms(stats.p50)}, 99th {ms(stats.p99)}, longest {ms(stats.max)} · Radio to picture {ms(stats.inputP50)}, 95th {ms(stats.inputP95)}
        </p>
      )}
    </div>
  );
}
