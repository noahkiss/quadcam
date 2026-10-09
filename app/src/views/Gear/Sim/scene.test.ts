// The picture itself needs a GPU, so the e2e spec smoke-tests it. These cover the camera's
// mounting, which is plain maths.
import { describe, expect, it } from "vitest";
import { Group, PerspectiveCamera, Vector3 } from "three";
import { quatToThree, vecToThree } from "../../../lib/simview";
import { mountFpv } from "./scene";

function look(att: [number, number, number, number], uptilt: number): Vector3 {
  const quad = new Group();
  quad.quaternion.set(...quatToThree(att));
  const cam = new PerspectiveCamera();
  mountFpv(cam, uptilt);
  quad.add(cam);
  quad.updateMatrixWorld(true);
  return cam.getWorldDirection(new Vector3());
}

describe("the FPV camera mount", () => {
  it("looks along the quad's forward axis, tilted up by the uptilt", () => {
    const d = look([1, 0, 0, 0], 20);
    const u = (20 * Math.PI) / 180;
    const want = vecToThree([Math.cos(u), 0, Math.sin(u)]);
    expect(d.x).toBeCloseTo(want[0]);
    expect(d.y).toBeCloseTo(want[1]);
    expect(d.z).toBeCloseTo(want[2]);
  });

  it("turns with the quad: a quarter turn left looks along world +y", () => {
    const h = Math.SQRT1_2;
    const d = look([h, 0, 0, h], 0);
    const want = vecToThree([0, 1, 0]);
    expect(d.x).toBeCloseTo(want[0]);
    expect(d.z).toBeCloseTo(want[2]);
  });

  it("pitches with the quad: nose down looks down", () => {
    // Nose down 45° (a turn about body y, to the left).
    const a = Math.PI / 8;
    const d = look([Math.cos(a), 0, Math.sin(a), 0], 0);
    expect(d.y).toBeLessThan(-0.6);
  });
});
