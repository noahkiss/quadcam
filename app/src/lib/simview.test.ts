import { describe, expect, it } from "vitest";
import { Quaternion, Vector3 } from "three";
import type { SimHud } from "../ipc/types";
import { aspectRatio, FlightTimer, MAX_VFOV_DEG, osdMah, osdTimer, osdVoltage, osdWarning, quatToThree, stickValues, vecToThree, verticalFov } from "./simview";

const HUD: SimHud = { armed: false, turtle: false, airmode: false, mode: "acro", arm_block: null, vbat: 3.9, mah: 0, low_battery: false, sticks: { roll: 0, pitch: 0, yaw: 0, throttle: 0 }, throttle: 0, stuck: false, contacts: 0, dropped_steps: 0 };

describe("world to three.js axes", () => {
  it("moves a point from z up to y up", () => {
    expect(vecToThree([1, 2, 3])).toEqual([1, 3, -2]);
  });

  it("keeps a rotation's sense: a left turn about world z is a left turn about three.js y", () => {
    const h = Math.SQRT1_2;
    // Body forward is world +x. Turn 90° left (counter-clockwise from above): forward is world +y.
    const q = new Quaternion(...quatToThree([h, 0, 0, h]));
    const fwd = new Vector3(1, 0, 0).applyQuaternion(q);
    const want = vecToThree([0, 1, 0]);
    expect(fwd.x).toBeCloseTo(want[0]);
    expect(fwd.y).toBeCloseTo(want[1]);
    expect(fwd.z).toBeCloseTo(want[2]);
  });

  it("maps pitching the nose down to the nose going down", () => {
    // Nose down 90° about body y (left): forward x goes to world -z.
    const h = Math.SQRT1_2;
    const q = new Quaternion(...quatToThree([h, 0, h, 0]));
    const fwd = new Vector3(1, 0, 0).applyQuaternion(q);
    expect(fwd.y).toBeCloseTo(-1);
  });
});

describe("camera", () => {
  it("turns a diagonal field of view into a vertical one", () => {
    // 90° diagonal on a square picture: tan(v/2) = tan(45°) / sqrt(2).
    expect(verticalFov(90, 1)).toBeCloseTo((2 * Math.atan(1 / Math.SQRT2) * 180) / Math.PI, 6);
    expect(verticalFov(90, 16 / 9)).toBeLessThan(verticalFov(90, 4 / 3));
  });

  it("caps the field of view a rectilinear picture can show", () => {
    expect(verticalFov(155, 16 / 9)).toBe(MAX_VFOV_DEG);
  });

  it("reads the aspect text", () => {
    expect(aspectRatio("4:3")).toBeCloseTo(4 / 3);
    expect(aspectRatio("16:9")).toBeCloseTo(16 / 9);
    expect(aspectRatio("nonsense")).toBeCloseTo(16 / 9);
  });
});

describe("stick display values", () => {
  it("shows throttle from -1 at the bottom to +1 at the top, the other axes as they are", () => {
    expect(stickValues({ roll: 0.5, pitch: -0.25, yaw: 1, throttle: 0 })).toEqual({ roll: 0.5, pitch: -0.25, yaw: 1, throttle: -1 });
    expect(stickValues({ roll: 0, pitch: 0, yaw: 0, throttle: 0.5 }).throttle).toBe(0);
    expect(stickValues({ roll: 0, pitch: 0, yaw: 0, throttle: 1 }).throttle).toBe(1);
  });
});

describe("OSD text", () => {
  it("formats voltage, timer and mAh", () => {
    expect(osdVoltage(3.9)).toBe("3.90V");
    expect(osdTimer(0)).toBe("00:00");
    expect(osdTimer(75.9)).toBe("01:15");
    expect(osdMah(12.4)).toBe("12mAh");
  });

  it("times the armed flight, keeps the reading when disarmed and restarts at the next arm", () => {
    const t = new FlightTimer();
    expect(t.update(false, 1)).toBe(0);
    expect(t.update(true, 2)).toBe(0);
    expect(t.update(true, 7.5)).toBe(5.5);
    expect(t.update(false, 9)).toBe(5.5);
    expect(t.update(false, 20)).toBe(5.5);
    expect(t.update(true, 21)).toBe(0);
    expect(t.update(true, 22)).toBe(1);
    t.reset();
    expect(t.update(false, 30)).toBe(0);
  });

  it("warns about what blocks arming first, then a low pack, then a stuck quad", () => {
    expect(osdWarning(HUD)).toBeNull();
    expect(osdWarning({ ...HUD, arm_block: "Throttle is up: lower it to arm" })).toBe("Throttle is up: lower it to arm");
    // Armed: the block no longer applies.
    expect(osdWarning({ ...HUD, armed: true, arm_block: "x" })).toBeNull();
    expect(osdWarning({ ...HUD, armed: true, low_battery: true })).toBe("Low battery");
    expect(osdWarning({ ...HUD, stuck: true })).toMatch(/Upside down/);
  });
});
