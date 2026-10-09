// What the Sim page shows, as plain functions: the world-to-three.js frame change, the camera's
// field of view, the OSD's text and the flight timer, and the stick display's values.
import type { SimHud } from "../ipc/types";
import type { StickValues } from "./controls";
import type { Quat, Vec3 } from "./simhost";

// The sim's frame has x and y horizontal and z up (body: x forward, y left, z up). three.js has
// y up and looks down -z. The change is a turn of -90° about x, so a rotation keeps its sense.

/** A world point in three.js axes. */
export const vecToThree = ([x, y, z]: Vec3): Vec3 => [x, z, -y];

/** A world attitude (w, x, y, z) as three.js wants it (x, y, z, w). */
export const quatToThree = ([w, x, y, z]: Quat): [number, number, number, number] => [x, z, -y, w];

/** The widest vertical field of view the page draws (deg). A rectilinear view cannot show a
 *  whoop camera's 150°-plus; the fisheye look is a later package. */
export const MAX_VFOV_DEG = 100;

/** The vertical field of view (deg) for a camera spec'd by its diagonal field of view, on a
 *  picture `aspect` wide (width over height). */
export function verticalFov(diagonalDeg: number, aspect: number): number {
  const half = Math.tan((diagonalDeg * Math.PI) / 360);
  const v = 2 * Math.atan(half / Math.hypot(aspect, 1)) * (180 / Math.PI);
  return Math.min(MAX_VFOV_DEG, v);
}

/** `16:9` as 16 / 9. */
export function aspectRatio(aspect: string): number {
  const [w, h] = aspect.split(":").map(Number);
  return w > 0 && h > 0 ? w / h : 16 / 9;
}

/** The stick display wants throttle in -1..1 like the other axes. */
export const stickValues = (s: SimHud["sticks"]): StickValues => ({ roll: s.roll, pitch: s.pitch, yaw: s.yaw, throttle: s.throttle * 2 - 1 });

/** Pack voltage as the OSD prints it. */
export const osdVoltage = (v: number) => `${v.toFixed(2)}V`;

/** Flight time, minutes and seconds. */
export function osdTimer(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

export const osdMah = (mah: number) => `${Math.round(mah)}mAh`;

/** The flight timer: runs while armed, keeps its reading when disarmed, restarts at the next arm. */
export class FlightTimer {
  private since: number | null = null;
  private last = 0;
  private wasArmed = false;

  /** `t` is the sim time (s) of the frame. Returns the reading (s). */
  update(armed: boolean, t: number): number {
    if (armed && !this.wasArmed) this.since = t;
    if (armed && this.since !== null) this.last = Math.max(0, t - this.since);
    this.wasArmed = armed;
    return this.last;
  }

  reset(): void {
    this.since = null;
    this.last = 0;
    this.wasArmed = false;
  }
}

/** The OSD's warning line: what blocks arming, a low pack, or a stuck quad. Null when all is well. */
export function osdWarning(h: SimHud): string | null {
  if (!h.armed && h.arm_block) return h.arm_block;
  if (h.low_battery) return "Low battery";
  if (h.stuck) return h.armed ? "Upside down: turtle or reset" : "Upside down: arm in turtle or reset";
  return null;
}
