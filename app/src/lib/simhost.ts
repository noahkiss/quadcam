// The Sim page's host loop (sim design 2.3, 2.4): once per animation frame it reads the newest
// two physics steps from the core, aligns the page clock to the core's, and hands the renderer
// the pose at "now minus one step". The physics runs on its own thread in the core at 2 kHz and
// never waits for a frame; the page never waits for the physics.
//
// Pure logic with injected time and I/O, so the unit tests drive it frame by frame. The three.js
// side is `views/Gear/Sim/scene.ts`.
import type { SimFrame, SimHud, SimPose } from "../ipc/types";

export type Vec3 = [number, number, number];
/** w, x, y, z. */
export type Quat = [number, number, number, number];

export interface Pose {
  pos: Vec3;
  quat: Quat;
}

/** Position and attitude at host time `tNs`, between a pair of steps: linear position, a
 *  normalised blend of the quaternions on the short way round. Clamped to the pair, so a
 *  stalled core shows its last state instead of guessing ahead. Same as the core's
 *  `SnapshotPair::pose_at`. */
export function poseAt(prev: SimPose, cur: SimPose, tNs: number): Pose {
  const span = cur.host_ns - prev.host_ns;
  const k = span <= 0 ? 1 : Math.min(1, Math.max(0, (tNs - prev.host_ns) / span));
  const pos = prev.pos.map((a, i) => a + (cur.pos[i] - a) * k) as Vec3;
  const dot = prev.quat.reduce((s, a, i) => s + a * cur.quat[i], 0);
  const sign = dot < 0 ? -1 : 1;
  const q = prev.quat.map((a, i) => a * (1 - k) + sign * cur.quat[i] * k);
  const n = Math.hypot(...q) || 1;
  return { pos, quat: q.map((x) => x / n) as Quat };
}

const finite = (p: SimPose) => [...p.pos, ...p.quat].every(Number.isFinite);

/** Maps the page clock (`performance.now()`, ms) to the core's host clock (ns). Each frame read
 *  tells the core's time at some moment between the request and the answer. The read with the
 *  shortest round trip pins that moment best, so the offset comes from the best of the last
 *  `WINDOW` reads. */
export class ClockSync {
  static readonly WINDOW = 120;
  private reads: { rtt: number; offset: number }[] = [];

  /** `sentMs` and `receivedMs` bracket the request; `hostNs` is the core's clock when it read. */
  observe(sentMs: number, receivedMs: number, hostNs: number): void {
    const rtt = Math.max(0, receivedMs - sentMs);
    this.reads.push({ rtt, offset: hostNs - ((sentMs + receivedMs) / 2) * 1e6 });
    if (this.reads.length > ClockSync.WINDOW) this.reads.shift();
  }

  get ready(): boolean {
    return this.reads.length > 0;
  }

  /** The shortest round trip in the window (ms). */
  get rtt(): number {
    return this.reads.length ? Math.min(...this.reads.map((r) => r.rtt)) : 0;
  }

  /** The core's clock (ns) at page time `ms`. */
  toHostNs(ms: number): number {
    if (!this.reads.length) return 0;
    let best = this.reads[0];
    for (const r of this.reads) if (r.rtt <= best.rtt) best = r;
    return ms * 1e6 + best.offset;
  }
}

const quantile = (sorted: number[], q: number) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.ceil(q * sorted.length) - 1)] : 0);

export interface PerfSummary {
  frames: number;
  /** Frame to frame (ms). */
  p50: number;
  p99: number;
  max: number;
  /** Intervals over 1.5 times the median. */
  late: number;
  /** Animation frames with no new read because the last one had not answered. */
  missed: number;
  /** Radio sample's arrival to the frame drawn (ms), the proxy in sim design 3.3. */
  inputP50: number;
  inputP95: number;
}

/** Frame pacing and the input-to-present proxy over the last `KEEP` frames. */
export class PerfStats {
  static readonly KEEP = 1200;
  private intervals: number[] = [];
  private inputs: number[] = [];
  private lastTs: number | null = null;
  frames = 0;
  missed = 0;
  /** Reads with a pose that was not a number. */
  bad = 0;

  frame(ts: number): void {
    if (this.lastTs !== null) {
      this.intervals.push(ts - this.lastTs);
      if (this.intervals.length > PerfStats.KEEP) this.intervals.shift();
    }
    this.lastTs = ts;
    this.frames++;
  }

  input(ms: number): void {
    this.inputs.push(ms);
    if (this.inputs.length > PerfStats.KEEP) this.inputs.shift();
  }

  summary(): PerfSummary {
    const iv = [...this.intervals].sort((a, b) => a - b);
    const inp = [...this.inputs].sort((a, b) => a - b);
    const p50 = quantile(iv, 0.5);
    return {
      frames: this.frames,
      p50,
      p99: quantile(iv, 0.99),
      max: iv.length ? iv[iv.length - 1] : 0,
      late: iv.filter((x) => x > p50 * 1.5).length,
      missed: this.missed,
      inputP50: quantile(inp, 0.5),
      inputP95: quantile(inp, 0.95),
    };
  }
}

/** What the renderer draws each frame. */
export interface Drawn {
  pose: Pose;
  hud: SimHud;
  frame: SimFrame;
  /** The radio sample to this draw (ms), when the core has seen a sample. */
  inputMs: number | null;
}

export interface LoopDeps {
  fetchFrame: () => Promise<SimFrame>;
  draw: (d: Drawn) => void;
  /** `performance.now()`. */
  now: () => number;
  raf: (cb: (ts: number) => void) => number;
  caf: (id: number) => void;
  /** The physics step (s). */
  dt: number;
  /** A read failed (the sim stopped): the loop has stopped. */
  onError?: (e: unknown) => void;
}

/** One read per animation frame, drawn when it answers. A read still waiting when the next
 *  frame comes is not repeated: that frame is counted as missed and the one after draws. */
export class FrameLoop {
  readonly clock = new ClockSync();
  readonly stats = new PerfStats();
  private id = 0;
  private running = false;
  private inflight = false;

  constructor(private d: LoopDeps) {}

  start(): void {
    if (this.running) return;
    this.running = true;
    this.id = this.d.raf(this.tick);
  }

  stop(): void {
    this.running = false;
    this.d.caf(this.id);
  }

  private tick = (ts: number): void => {
    if (!this.running) return;
    this.id = this.d.raf(this.tick);
    this.stats.frame(ts);
    if (this.inflight) {
      this.stats.missed++;
      return;
    }
    this.inflight = true;
    const sent = this.d.now();
    this.d.fetchFrame().then(
      (frame) => {
        this.inflight = false;
        if (!this.running) return;
        const got = this.d.now();
        this.clock.observe(sent, got, frame.host_ns);
        const nowNs = this.clock.toHostNs(got);
        const target = nowNs - this.d.dt * 1e9;
        const input = frame.cur.input_ns > 0 ? Math.max(0, (nowNs - frame.cur.input_ns) / 1e6) : null;
        if (input !== null) this.stats.input(input);
        // The core sends finite numbers; a null (JSON for NaN) would blank the picture.
        if (![frame.prev, frame.cur].every(finite)) {
          this.stats.bad++;
          return;
        }
        const pose = poseAt(frame.prev, frame.cur, target);
        this.d.draw({ pose, hud: frame.hud, frame, inputMs: input });
      },
      (e) => {
        this.inflight = false;
        this.stop();
        this.d.onError?.(e);
      },
    );
  };
}
