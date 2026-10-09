// The picture: a canvas, the OSD and the stick display, driven by the host loop. It re-renders
// each frame the HUD changes, so the page's header and popover live outside it.
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Sticks } from "../../../components/gear/Sticks";
import { api, errText } from "../../../ipc/api";
import type { SimHud, SimStartInfo } from "../../../ipc/types";
import { asMode } from "../../../lib/controls";
import { FrameLoop, type PerfSummary } from "../../../lib/simhost";
import { aspectRatio, FlightTimer, stickValues } from "../../../lib/simview";
import { Osd } from "./Osd";
import { createScene, type SimScene } from "./scene";
import styles from "./SimPage.module.css";

/** The page's choices with the profile's values behind them. */
export interface SimView {
  view: "fpv" | "chase";
  aspect: string;
  uptiltDeg: number;
  fovDeg: number;
  stickDisplay: boolean;
  osd: boolean;
}

interface Props {
  info: SimStartInfo;
  ui: SimView;
  /** The loop stopped: the core says why. */
  onStopped: (why: string) => void;
  onStats: (s: PerfSummary) => void;
}

export function Stage({ info, ui, onStopped, onStats }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const picture = useRef<HTMLDivElement>(null);
  const scene = useRef<SimScene | null>(null);
  const [hud, setHud] = useState<SimHud | null>(null);
  const [seconds, setSeconds] = useState(0);
  const [radio, setRadio] = useState(true);
  // Written straight to the DOM: the message only appears if the context fails to open.
  const glMessage = useRef<HTMLParagraphElement>(null);
  const aspect = aspectRatio(ui.aspect);

  // Callbacks and the view change without restarting the loop.
  const latest = useRef({ onStopped, onStats });
  useEffect(() => {
    latest.current = { onStopped, onStats };
  });

  // The scene and the loop live as long as this sim.
  useEffect(() => {
    let sc: SimScene | null = null;
    try {
      sc = createScene(canvas.current!, info);
    } catch (e) {
      glMessage.current!.textContent = errText(e);
      glMessage.current!.hidden = false;
    }
    scene.current = sc;
    const fit = () => {
      const r = picture.current!.getBoundingClientRect();
      sc?.resize(r.width, r.height);
      sc?.render();
    };
    const ro = new ResizeObserver(fit);
    ro.observe(picture.current!);
    fit();

    const timer = new FlightTimer();
    let last = "";
    const loop = new FrameLoop({
      fetchFrame: api.simFrame,
      draw: ({ pose, hud: h, frame }) => {
        sc?.setPose(pose);
        sc?.render();
        // The HUD changes less often than the frame; only a change re-renders the overlay.
        const key = JSON.stringify([h, frame.radio, Math.floor(frame.cur.t)]);
        if (key !== last) {
          last = key;
          setHud(h);
          setRadio(frame.radio);
        }
        const s = timer.update(h.armed, frame.cur.t);
        setSeconds((p) => (Math.floor(p) === Math.floor(s) ? p : s));
      },
      now: () => performance.now(),
      raf: (cb) => requestAnimationFrame(cb),
      caf: (id) => cancelAnimationFrame(id),
      dt: info.dt,
      onError: (e) => latest.current.onStopped(errText(e)),
    });
    loop.start();
    const stats = setInterval(() => latest.current.onStats(loop.stats.summary()), 1000);
    return () => {
      clearInterval(stats);
      loop.stop();
      ro.disconnect();
      sc?.dispose();
      scene.current = null;
    };
  }, [info]);

  useEffect(() => {
    scene.current?.setView({ view: ui.view, uptiltDeg: ui.uptiltDeg, fovDeg: ui.fovDeg, aspect });
  }, [ui.view, ui.uptiltDeg, ui.fovDeg, aspect]);

  return (
    <div className={styles.stage}>
      <div ref={picture} className={styles.picture} style={{ "--ar": aspect } as CSSProperties}>
        <canvas ref={canvas} className={styles.canvas} aria-label="Sim view" data-view={ui.view} />
        <p ref={glMessage} className={styles.empty} role="alert" hidden />
        {ui.osd && hud && <Osd hud={hud} seconds={seconds} radio={radio} />}
        {ui.stickDisplay && (
          <div className={styles.sticks} role="group" aria-label="Stick display">
            <Sticks values={hud ? stickValues(hud.sticks) : null} mode={asMode(info.stick_mode)} compact />
          </div>
        )}
      </div>
    </div>
  );
}
