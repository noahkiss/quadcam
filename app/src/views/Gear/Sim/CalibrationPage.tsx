// The sim's radio calibration (sim design 7.3): which radio the joystick is, what QuadCam
// suggests from the aircraft's radio model and quad modes, the guided steps (move, let go,
// arm switch, reset control), and the review with each axis' channel, ends, deadzone and
// Reverse. The core runs the session (`gear_sim_calibrate`) on every radio report and sends
// `sim-calibration-event` at most once per frame.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Sticks, StickModePicker } from "../../../components/gear/Sticks";
import { api, errText } from "../../../ipc/api";
import { on } from "../../../ipc/events";
import type { AxisCal, CalibrateParams, CalibrateView, Calibration, CaptureTarget, SimCalibration, SimDefaults, SuggestedControl } from "../../../ipc/types";
import type { StickMode } from "../../../lib/controls";
import { controlText, endName, highEnd, lowEnd, STICK_FNS, stepText, sticksOf, TARGET_TEXT, type StickFn } from "../../../lib/simcal";
import { useStore } from "../../../store";
import { sel } from "../../../store/settings";
import styles from "./Sim.module.css";

const FN_TEXT: Record<StickFn, string> = { roll: "Roll", pitch: "Pitch", throttle: "Throttle", yaw: "Yaw" };
const TARGETS: CaptureTarget[] = ["arm", "reset", "turtle", "angle", "horizon", "airmode"];

const calibrate = (p: Partial<CalibrateParams> & Pick<CalibrateParams, "action">) =>
  api.gearSimCalibrate({ calibration: null, quick: false, arm_known: false, review: false, target: null, ...p });

export function CalibrationPage() {
  const profiles = useStore(sel.profiles);
  const devices = useStore((s) => s.devices);
  const mode = useStore(sel.stickMode);
  const saveSetting = useStore((s) => s.saveSetting);
  const defaultProfile = useStore(sel.defaultProfile);
  const [aircraft, setAircraft] = useState<string>(defaultProfile);
  const [found, setFound] = useState<SimCalibration | null>(null);
  const [radio, setRadio] = useState<string | null>(null);
  const [defaults, setDefaults] = useState<SimDefaults | null>(null);
  const [view, setView] = useState<CalibrateView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  // Which radio, and its saved calibration.
  useEffect(() => {
    api.gearSimCalibration().then(
      (c) => {
        setFound(c);
        setRadio(c.resolution?.radio ?? null);
        setError(null);
      },
      (e) => setError(errText(e)),
    );
  }, []);

  useEffect(() => {
    api.gearSimDefaults(aircraft || null).then(setDefaults, (e) => setError(errText(e)));
  }, [aircraft]);

  // The live session: events while it runs, stopped when the page goes.
  useEffect(() => {
    let raf = 0;
    let latest: CalibrateView | null = null;
    const unlisten = on("sim-calibration-event", (v) => {
      latest = v;
      if (!raf)
        raf = requestAnimationFrame(() => {
          raf = 0;
          setView(latest);
        });
    });
    return () => {
      cancelAnimationFrame(raf);
      void unlisten.then((f) => f());
      calibrate({ action: "stop" }).catch(() => {});
    };
  }, []);

  // Start once the radio and the defaults are known: at Review on a saved calibration,
  // else from the defaults (the short check when the model gives every stick). Another
  // aircraft's defaults start a new calibration again.
  const savedCal = found?.calibration?.calibration ?? null;
  const started = view?.active ?? false;
  useEffect(() => {
    if (!found || !defaults || (savedCal && started)) return;
    const start = savedCal ?? { ...defaults.calibration, mode };
    act({ action: "start", calibration: start, review: !!savedCal, quick: !savedCal && defaults.map.known, arm_known: !!defaults.arm });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per radio and defaults
  }, [found, defaults]);

  function act(p: Partial<CalibrateParams> & Pick<CalibrateParams, "action">) {
    setSaved(false);
    calibrate(p).then(
      (v) => {
        setView(v);
        setError(null);
      },
      (e) => setError(errText(e)),
    );
  }

  const cal = view?.calibration ?? null;
  const set = (c: Calibration) => act({ action: "set", calibration: c });
  const setAxis = (f: StickFn, a: Partial<AxisCal>) => {
    if (!cal) return;
    const next = { ...cal, [f]: { ...cal[f], ...a } };
    // A channel another stick has: the two swap.
    const other = a.ch != null ? STICK_FNS.find((g) => g !== f && cal[g].ch === a.ch) : undefined;
    if (other) next[other] = { ...cal[other], ch: cal[f].ch };
    set(next);
  };
  const setMode = (m: StickMode) => {
    void saveSetting("stickMode", m);
    if (cal) set({ ...cal, mode: m });
  };

  async function save() {
    if (!cal || !radio || !found?.resolution) return;
    const r = found.resolution;
    try {
      await api.gearSimCalibrationSave({
        radio,
        calibration: cal,
        product: r.product,
        firmware: found.firmware ?? null,
        remember: r.radio == null || (r.provisional && radio !== r.radio),
        replaces: r.provisional && radio !== r.radio ? r.radio : null,
      });
      setSaved(true);
      setError(null);
    } catch (e) {
      setError(errText(e));
    }
  }

  const res = found?.resolution ?? null;
  const radios = devices.filter((d) => d.kind === "radio");
  const choices = res && !res.provisional && res.choices.length ? res.choices : radios.map((d) => ({ id: d.id, name: d.name || "Unnamed Radio" }));

  return (
    <div className={styles.page} data-component="sim-calibration">
      <h2 className={styles.title}>Sim radio</h2>
      <p className={styles.status} role="status" aria-label="Radio">
        {res ? `${res.product}: ${res.how}` : error ? "" : "Looking for the radio…"}
      </p>
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}

      {res && (res.radio == null || res.provisional) && (
        <label className={styles.row}>
          <span>{res.radio == null ? "Which radio is this?" : "Link to a saved radio"}</span>
          <select aria-label="Radio" value={radio ?? ""} onChange={(e) => setRadio(e.target.value || (res.radio ?? null))}>
            <option value="">{res.radio == null ? "Choose…" : "Not linked"}</option>
            {choices.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </label>
      )}

      <section className={styles.panel} aria-label="Suggestions">
        <div className={styles.bar}>
          <h3 className={styles.h3}>Sim controls</h3>
          <label className={styles.row}>
            <span>Aircraft</span>
            <select aria-label="Aircraft" value={aircraft} onChange={(e) => setAircraft(e.target.value)}>
              <option value="">None</option>
              {profiles.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        {defaults && (
          <dl className={styles.suggestions}>
            <dt>Sticks</dt>
            <dd>
              Roll CH{cal?.roll.ch ?? defaults.map.roll}, pitch CH{cal?.pitch.ch ?? defaults.map.pitch}, throttle CH{cal?.throttle.ch ?? defaults.map.throttle}, yaw CH
              {cal?.yaw.ch ?? defaults.map.yaw} <span className={styles.source}>(from {defaults.map.source})</span>
            </dd>
            {TARGETS.map((t) => (
              <Suggestion key={t} target={t} current={cal?.[t] ?? null} suggested={defaults[t] ?? null} canChange={view?.phase === "review"} onChange={() => act({ action: "capture", target: t })} />
            ))}
          </dl>
        )}
        {defaults?.notes.map((n) => (
          <p key={n} className={styles.note}>
            {n}
          </p>
        ))}
      </section>

      <section className={styles.panel} aria-label="Calibration">
        <div className={styles.bar}>
          <StickModePicker mode={mode} onChange={setMode} />
        </div>
        <Sticks values={sticksOf(view)} mode={mode} />
        {view && (
          <>
            <p className={styles.step} role="status" aria-label="Step">
              {stepText(view.phase, view.quick, view.target)}
            </p>
            {view.phase === "move" && (
              <ul className={styles.coverage} aria-label="Coverage">
                {STICK_FNS.map((f) => (
                  <li key={f}>
                    <span>{FN_TEXT[f]}</span>
                    <span className={styles.meter} role="meter" aria-label={FN_TEXT[f]} aria-valuemin={0} aria-valuemax={100} aria-valuenow={view.coverage[f]}>
                      <span className={styles.fill} style={{ width: `${view.coverage[f]}%` }} />
                    </span>
                    <span className={styles.num}>{view.coverage[f]}%</span>
                  </li>
                ))}
              </ul>
            )}
            {view.message && <p className={styles.note}>{view.message}</p>}
            <div className={styles.bar}>
              {view.phase === "move" && <Button onClick={() => act({ action: "advance" })}>Done</Button>}
              {view.phase === "let_go" && <Button onClick={() => act({ action: "advance" })}>Next</Button>}
              {view.phase === "arm" && <Button onClick={() => act({ action: "skip" })}>No arm switch</Button>}
              {view.phase === "reset" && (
                <>
                  {cal?.reset && <Button onClick={() => act({ action: "advance" })}>Keep {controlText(cal.reset)}</Button>}
                  <Button onClick={() => act({ action: "skip" })}>No reset control</Button>
                </>
              )}
              {view.phase === "capture" && (
                <>
                  <Button onClick={() => act({ action: "advance" })}>Cancel</Button>
                  <Button onClick={() => act({ action: "skip" })}>Clear</Button>
                </>
              )}
              {view.phase === "review" && (
                <>
                  <Button variant="primary" disabled={!radio} onClick={() => void save()}>
                    Save
                  </Button>
                  <Button onClick={() => act({ action: "recalibrate" })}>Recalibrate</Button>
                  {saved && <span role="status">Saved</span>}
                </>
              )}
            </div>
            {view.phase === "review" && cal && <AxisTable cal={cal} onAxis={setAxis} />}
          </>
        )}
      </section>
    </div>
  );
}

function Suggestion({ target, current, suggested, canChange, onChange }: { target: CaptureTarget; current: Calibration["arm"]; suggested: SuggestedControl | null; canChange: boolean; onChange: () => void }) {
  const same = suggested && current && JSON.stringify(suggested.control) === JSON.stringify(current);
  return (
    <>
      <dt>{TARGET_TEXT[target]}</dt>
      <dd data-target={target}>
        {same && suggested ? (
          <>
            {suggested.label} <span className={styles.source}>(from {suggested.source})</span>
          </>
        ) : (
          controlText(current)
        )}
        <Button size="sm" variant="ghost" disabled={!canChange} onClick={onChange} aria-label={`Change ${TARGET_TEXT[target]}`}>
          Change
        </Button>
      </dd>
    </>
  );
}

function AxisTable({ cal, onAxis }: { cal: Calibration; onAxis: (f: StickFn, a: Partial<AxisCal>) => void }) {
  const num = (v: string) => Math.max(0, Math.min(2048, Math.round(Number(v) || 0)));
  return (
    <table className={styles.table} aria-label="Axes">
      <thead>
        <tr>
          <th>Axis</th>
          <th>Channel</th>
          <th>Reverse</th>
          <th>Ends</th>
          <th>Deadzone</th>
        </tr>
      </thead>
      <tbody>
        {STICK_FNS.map((f) => {
          const a = cal[f];
          const rev = !!a.reverse;
          return (
            <tr key={f}>
              <th scope="row">{FN_TEXT[f]}</th>
              <td>
                <select aria-label={`${FN_TEXT[f]} channel`} value={a.ch} onChange={(e) => onAxis(f, { ch: Number(e.target.value) })}>
                  {[1, 2, 3, 4, 5, 6, 7, 8].map((c) => (
                    <option key={c} value={c}>
                      CH{c}
                    </option>
                  ))}
                </select>
              </td>
              <td>
                <input type="checkbox" aria-label={`${FN_TEXT[f]} reverse`} checked={rev} onChange={(e) => onAxis(f, { reverse: e.target.checked })} />
              </td>
              <td className={styles.ends}>
                {(["low", "high"] as const).map((end) => {
                  const edited = end === "low" ? a.edited_low != null : a.edited_high != null;
                  const name = endName(f, end, rev);
                  return (
                    <label key={end} className={styles.end}>
                      <span>
                        {name}
                        {edited && <span className={styles.edited}> (edited)</span>}
                      </span>
                      <input
                        type="number"
                        min={0}
                        max={2048}
                        aria-label={`${FN_TEXT[f]} ${name}`}
                        value={end === "low" ? lowEnd(a) : highEnd(a)}
                        onChange={(e) => onAxis(f, end === "low" ? { edited_low: num(e.target.value) } : { edited_high: num(e.target.value) })}
                      />
                    </label>
                  );
                })}
              </td>
              <td>
                {f === "throttle" ? (
                  ""
                ) : (
                  <input type="number" min={0} max={50} aria-label={`${FN_TEXT[f]} deadzone`} value={a.deadzone ?? 0} onChange={(e) => onAxis(f, { deadzone: Math.max(0, Math.min(50, Math.round(Number(e.target.value) || 0))) })} />
                )}
                {f !== "throttle" && " %"}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
