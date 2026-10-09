import { describe, expect, it } from "vitest";
import type { SimFrame, SimHud, SimPose } from "../ipc/types";
import { ClockSync, FrameLoop, PerfStats, poseAt, type Drawn } from "./simhost";

const pose = (host_ns: number, x: number, over: Partial<SimPose> = {}): SimPose => ({ step: 0, t: 0, host_ns, input_ns: 0, pos: [x, 0, 0], quat: [1, 0, 0, 0], ...over });
const HUD: SimHud = { armed: true, turtle: false, airmode: false, mode: "acro", arm_block: null, vbat: 3.9, mah: 0, low_battery: false, sticks: { roll: 0, pitch: 0, yaw: 0, throttle: 0 }, throttle: 0, stuck: false, contacts: 0, dropped_steps: 0 };

describe("poseAt", () => {
  it("interpolates position between the two steps and clamps outside them", () => {
    const [a, b] = [pose(1_000, 0), pose(2_000, 10)];
    expect(poseAt(a, b, 1_500).pos).toEqual([5, 0, 0]);
    expect(poseAt(a, b, 1_250).pos[0]).toBeCloseTo(2.5);
    // Ahead of the newest step: the newest state, not a guess.
    expect(poseAt(a, b, 9_000).pos[0]).toBe(10);
    expect(poseAt(a, b, 0).pos[0]).toBe(0);
  });

  it("blends attitude on the short way round and keeps it a unit quaternion", () => {
    const h = Math.SQRT1_2;
    const a = pose(0, 0, { quat: [1, 0, 0, 0] });
    // The same turn written with the opposite sign: the blend must not pass through zero.
    const b = pose(1_000, 0, { quat: [-h, 0, 0, -h] });
    const q = poseAt(a, b, 500).quat;
    expect(Math.hypot(...q)).toBeCloseTo(1);
    expect(q[0]).toBeGreaterThan(0.8);
    expect(Math.abs(q[3])).toBeGreaterThan(0.3);
  });

  it("shows the newest step when the pair has no span", () => {
    expect(poseAt(pose(5, 1), pose(5, 2), 5).pos[0]).toBe(2);
  });
});

describe("ClockSync", () => {
  it("maps page time to the core's clock using the shortest round trip", () => {
    const c = new ClockSync();
    expect(c.ready).toBe(false);
    // The core's clock is the page's plus 5 s. A slow read lands its stamp late in the trip;
    // the quick one tells the truth.
    c.observe(100, 120, 5_000_000_000 + 119 * 1e6);
    c.observe(200, 201, 5_000_000_000 + 200.5 * 1e6);
    c.observe(300, 330, 5_000_000_000 + 329 * 1e6);
    expect(c.rtt).toBe(1);
    expect(c.toHostNs(400)).toBeCloseTo(5_000_000_000 + 400e6, -3);
  });

  it("forgets reads older than its window", () => {
    const c = new ClockSync();
    c.observe(0, 0.1, 1e9);
    for (let i = 1; i <= ClockSync.WINDOW; i++) c.observe(i, i + 4, 1e9 + (i + 2) * 1e6);
    expect(c.rtt).toBe(4);
  });
});

describe("PerfStats", () => {
  it("reports frame pacing and the input proxy", () => {
    const s = new PerfStats();
    let t = 0;
    for (let i = 0; i < 200; i++) {
      s.frame(t);
      t += i === 100 ? 25 : 8.33;
    }
    for (const ms of [4, 5, 6, 7, 30]) s.input(ms);
    const r = s.summary();
    expect(r.frames).toBe(200);
    expect(r.p50).toBeCloseTo(8.33, 1);
    expect(r.max).toBeCloseTo(25, 1);
    expect(r.late).toBe(1);
    expect(r.inputP50).toBe(6);
  });
});

/** A hand-turned animation frame and a read that answers when the test says. */
function rig(dt = 0.0005) {
  const cbs: ((ts: number) => void)[] = [];
  const reads: ((f: SimFrame) => void)[] = [];
  const drawn: Drawn[] = [];
  let now = 0;
  const loop = new FrameLoop({
    fetchFrame: () => new Promise<SimFrame>((res) => reads.push(res)),
    draw: (d) => drawn.push(d),
    now: () => now,
    raf: (cb) => cbs.push(cb),
    caf: () => cbs.splice(0),
    dt,
  });
  return {
    loop,
    drawn,
    reads,
    /** Run the next animation frame at page time `ms`. */
    frame(ms: number) {
      now = ms;
      cbs.shift()!(ms);
    },
    /** The core answers the oldest read at page time `ms`. */
    async answer(ms: number, f: SimFrame) {
      now = ms;
      reads.shift()!(f);
      await Promise.resolve();
    },
    pending: () => cbs.length,
  };
}

// The core's clock is the page's plus one hour.
const HOST0 = 3_600e9;
const host = (ms: number) => HOST0 + ms * 1e6;
const frameAt = (ms: number, x0: number, x1: number, inputMsAgo = 0): SimFrame => ({
  host_ns: host(ms),
  prev: pose(host(ms - 1.5), x0),
  cur: pose(host(ms - 1), x1, { input_ns: inputMsAgo ? host(ms - inputMsAgo) : 0 }),
  hud: HUD,
  radio: true,
});

describe("FrameLoop", () => {
  it("reads once per animation frame and draws the pose at the newest step", async () => {
    const r = rig();
    r.loop.start();
    r.frame(0);
    expect(r.reads).toHaveLength(1);
    await r.answer(1, frameAt(1, 0, 1, 12));
    expect(r.drawn).toHaveLength(1);
    // One step ahead of the newest read: clamped to it.
    expect(r.drawn[0].pose.pos[0]).toBe(1);
    // 12 ms before the read, plus the half round trip the clock estimate puts on the answer.
    expect(r.drawn[0].inputMs).toBeCloseTo(12.5, 1);
    r.frame(8.3);
    await r.answer(9.3, frameAt(9.3, 2, 3));
    expect(r.drawn).toHaveLength(2);
    expect(r.drawn[1].pose.pos[0]).toBe(3);
    expect(r.loop.clock.ready).toBe(true);
    r.loop.stop();
  });

  it("does not repeat a read that has not answered, and counts the frame missed", async () => {
    const r = rig();
    r.loop.start();
    r.frame(0);
    r.frame(8.3);
    r.frame(16.6);
    expect(r.reads).toHaveLength(1);
    expect(r.loop.stats.missed).toBe(2);
    await r.answer(17, frameAt(17, 0, 1));
    r.frame(25);
    expect(r.reads).toHaveLength(1);
    r.loop.stop();
  });

  it("draws a recorded flight smoothly: positions never go backwards and never lead the core", async () => {
    const r = rig();
    r.loop.start();
    // A 120 Hz display against a core that steps every 0.5 ms along x = 2 m/s.
    for (let i = 0; i < 120; i++) {
      const ms = i * 8.333;
      r.frame(ms);
      const newest = (ms + 1) * 2 / 1000;
      await r.answer(ms + 1, frameAt(ms + 1, newest - 0.001, newest));
    }
    const xs = r.drawn.map((d) => d.pose.pos[0]);
    expect(xs).toHaveLength(120);
    for (let i = 1; i < xs.length; i++) expect(xs[i]).toBeGreaterThanOrEqual(xs[i - 1]);
    expect(xs[119]).toBeLessThanOrEqual((119 * 8.333 + 1) * 2 / 1000 + 1e-9);
    r.loop.stop();
  });

  it("draws between two steps when the core is behind the page's clock estimate", async () => {
    // A core that stepped 4 ms ago and 8 ms ago (a slow host): the target time falls inside.
    const r = rig(0.008);
    r.loop.start();
    r.frame(0);
    const f: SimFrame = { host_ns: host(5), prev: pose(host(2), 0), cur: pose(host(10), 8), hud: HUD, radio: true };
    await r.answer(10, f);
    // The read went out at 0 and came back at 10, so the core read at 5 ms: page time 10 ms is host
    // 10 ms, and one 8 ms step earlier is 2 ms, the previous step.
    expect(r.drawn[0].pose.pos[0]).toBeCloseTo(0);
    r.loop.stop();
  });

  it("skips a frame whose pose is not a number", async () => {
    const r = rig();
    r.loop.start();
    r.frame(0);
    const f = frameAt(1, 0, 1);
    f.cur.pos = [null as unknown as number, 0, 0];
    await r.answer(1, f);
    expect(r.drawn).toHaveLength(0);
    expect(r.loop.stats.bad).toBe(1);
    r.loop.stop();
  });

  it("stops and reports when a read fails", async () => {
    const errors: unknown[] = [];
    const cbs: (() => void)[] = [];
    const loop = new FrameLoop({
      fetchFrame: () => Promise.reject("The sim is not running."),
      draw: () => {},
      now: () => 0,
      raf: (cb) => cbs.push(() => cb(0)),
      caf: () => cbs.splice(0),
      dt: 0.0005,
      onError: (e) => errors.push(e),
    });
    loop.start();
    cbs.shift()!();
    await Promise.resolve();
    await Promise.resolve();
    expect(errors).toEqual(["The sim is not running."]);
    expect(cbs).toHaveLength(0);
  });
});
