// The Sim page's picture (sim design 8, phase 1): a plain room drawn from the same boxes the
// physics collides with, the quad, and an FPV or chase camera. Flat shading, no assets. three.js
// is the minimal renderer; a native engine may replace it (S6).
import {
  AmbientLight,
  BoxGeometry,
  BufferGeometry,
  DirectionalLight,
  EdgesGeometry,
  Float32BufferAttribute,
  Group,
  HemisphereLight,
  LineBasicMaterial,
  LineSegments,
  Mesh,
  MeshLambertMaterial,
  PerspectiveCamera,
  RingGeometry,
  Scene,
  Vector3,
  WebGLRenderer,
  Color,
  DoubleSide,
} from "three";
import type { SimBox, SimStartInfo } from "../../../ipc/types";
import type { Pose } from "../../../lib/simhost";
import { quatToThree, vecToThree, verticalFov } from "../../../lib/simview";

export interface ViewSettings {
  view: "fpv" | "chase";
  uptiltDeg: number;
  /** Diagonal, as the profile gives it. */
  fovDeg: number;
  /** Picture width over height. */
  aspect: number;
}

export interface SimScene {
  setPose(p: Pose): void;
  setView(v: ViewSettings): void;
  /** CSS pixels. */
  resize(width: number, height: number): void;
  render(): void;
  dispose(): void;
}

const COLOUR = {
  shell: 0x9aa0b0,
  floor: 0x4a4f60,
  prop: 0xc8a97e,
  gate: 0xf0803c,
  line: 0x2b2e3a,
  sky: 0xdfe6f5,
  ground: 0x586070,
  quad: 0x23252e,
  rotor: 0x5fd0c8,
  pad: 0xf5d86b,
};

const isShell = (b: SimBox) => Math.max(...b.half) > 1;

function colourOf(b: SimBox): number {
  if (b.material === "gate") return COLOUR.gate;
  if (b.material === "floor" || b.material === "carpet" || b.material === "grass") return COLOUR.floor;
  return isShell(b) ? COLOUR.shell : COLOUR.prop;
}

/** The room: the physics' boxes, with their edges lined. */
function room(boxes: SimBox[]): Group {
  const g = new Group();
  const edge = new LineBasicMaterial({ color: COLOUR.line });
  for (const b of boxes) {
    const geo = new BoxGeometry(b.half[0] * 2, b.half[2] * 2, b.half[1] * 2);
    const m = new Mesh(geo, new MeshLambertMaterial({ color: colourOf(b) }));
    const lines = new LineSegments(new EdgesGeometry(geo), edge);
    const box = new Group();
    box.add(m, lines);
    box.position.set(...vecToThree(b.centre));
    // The world's yaw about z is a turn about three.js y.
    box.rotation.y = (b.yaw_deg * Math.PI) / 180;
    g.add(box);
  }
  return g;
}

/** Lines every metre on the floor, so speed and distance read. */
function grid(boxes: SimBox[]): LineSegments | null {
  const floor = boxes.find((b) => b.material === "floor" && isShell(b));
  if (!floor) return null;
  const hx = Math.floor(floor.half[0] - 0.1);
  const hy = Math.floor(floor.half[1] - 0.1);
  const v: number[] = [];
  for (let x = -hx; x <= hx; x++) v.push(x, 0.002, -hy, x, 0.002, hy);
  for (let y = -hy; y <= hy; y++) v.push(-hx, 0.002, -y, hx, 0.002, -y);
  const geo = new BufferGeometry();
  geo.setAttribute("position", new Float32BufferAttribute(v, 3));
  return new LineSegments(geo, new LineBasicMaterial({ color: COLOUR.line, transparent: true, opacity: 0.55 }));
}

/** The quad: a body and four rotor rings at the frame's corners (x forward, y left, z up). */
function quadModel(i: SimStartInfo): Group {
  const g = new Group();
  const [hx, hy, hz] = i.body_half;
  const body = new Mesh(new BoxGeometry(hx * 2, hz * 2, hy * 2), new MeshLambertMaterial({ color: COLOUR.quad }));
  g.add(body);
  const d = i.wheelbase_m / 2 / Math.SQRT2;
  const r = i.prop_radius_m;
  for (const [sx, sy] of [
    [1, -1],
    [1, 1],
    [-1, -1],
    [-1, 1],
  ]) {
    const ring = new Mesh(new RingGeometry(r * 0.82, r, 24), new MeshLambertMaterial({ color: COLOUR.rotor, side: DoubleSide }));
    ring.rotation.x = -Math.PI / 2;
    ring.position.set(...vecToThree([sx * d, sy * d, hz]));
    g.add(ring);
  }
  // The two frame diagonals.
  for (const turn of [Math.PI / 4, -Math.PI / 4]) {
    const arm = new Mesh(new BoxGeometry(i.wheelbase_m, hz * 1.2, r * 0.18), new MeshLambertMaterial({ color: COLOUR.quad }));
    arm.rotation.y = turn;
    arm.position.set(...vecToThree([0, 0, hz]));
    g.add(arm);
  }
  return g;
}

export function createScene(canvas: HTMLCanvasElement, info: SimStartInfo): SimScene {
  // A context can fail to open (no GPU, a blocked WebGL): the page says so and keeps its
  // controls, instead of throwing out of the effect.
  let renderer: WebGLRenderer;
  try {
    renderer = new WebGLRenderer({ canvas, antialias: true, powerPreference: "high-performance" });
  } catch {
    throw new Error("The picture needs WebGL, and this view cannot open it.");
  }
  renderer.setClearColor(new Color(COLOUR.sky));

  const scene = new Scene();
  scene.add(new HemisphereLight(COLOUR.sky, COLOUR.ground, 1.6), new AmbientLight(0xffffff, 0.4));
  const sun = new DirectionalLight(0xffffff, 1.2);
  sun.position.set(2, 5, 1);
  scene.add(sun);
  scene.add(room(info.boxes));
  const lines = grid(info.boxes);
  if (lines) scene.add(lines);
  // The start pad.
  const pad = new Mesh(new RingGeometry(0.12, 0.16, 32), new MeshLambertMaterial({ color: COLOUR.pad, side: DoubleSide }));
  pad.rotation.x = -Math.PI / 2;
  pad.position.set(...vecToThree([info.start[0], info.start[1], 0.004]));
  scene.add(pad);

  const quad = quadModel(info);
  scene.add(quad);
  const fpv = new PerspectiveCamera(80, 16 / 9, 0.01, 60);
  fpv.position.set(...vecToThree(info.camera.position));
  mountFpv(fpv, info.camera.uptilt_deg);
  quad.add(fpv);
  const chase = new PerspectiveCamera(70, 16 / 9, 0.02, 60);
  scene.add(chase);

  let view: ViewSettings = { view: "fpv", uptiltDeg: info.camera.uptilt_deg, fovDeg: info.camera.fov_deg, aspect: 16 / 9 };
  let pose: Pose = { pos: [...info.start], quat: [1, 0, 0, 0] };
  const place = () => {
    quad.position.set(...vecToThree(pose.pos));
    quad.quaternion.set(...quatToThree(pose.quat));
    quad.visible = view.view === "chase";
    if (view.view === "chase") {
      // Behind and above the heading, looking at the quad.
      const fwd = new Vector3(1, 0, 0).applyQuaternion(quad.quaternion);
      const heading = Math.atan2(-fwd.z, fwd.x);
      chase.position.copy(quad.position).add(new Vector3(-Math.cos(heading) * 0.5, 0.22, Math.sin(heading) * 0.5));
      chase.lookAt(quad.position);
    }
  };
  const camera = () => (view.view === "chase" ? chase : fpv);
  const lens = () => {
    for (const c of [fpv, chase]) {
      c.aspect = view.aspect;
      c.fov = c === fpv ? verticalFov(view.fovDeg, view.aspect) : 70;
      c.updateProjectionMatrix();
    }
    mountFpv(fpv, view.uptiltDeg);
  };
  lens();
  place();

  return {
    setPose(p) {
      pose = p;
      place();
    },
    setView(v) {
      view = v;
      lens();
      place();
    },
    resize(w, h) {
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(Math.max(1, w), Math.max(1, h), false);
    },
    render() {
      renderer.render(scene, camera());
    },
    dispose() {
      renderer.dispose();
      scene.traverse((o) => {
        if (o instanceof Mesh || o instanceof LineSegments) {
          o.geometry.dispose();
          const m = o.material;
          (Array.isArray(m) ? m : [m]).forEach((x) => x.dispose());
        }
      });
    },
  };
}

/** Points a camera that is a child of the quad along the quad's forward axis, tilted up by
 *  `uptiltDeg`. A child of the quad sits in three.js axes (y up, forward is +x), and a camera
 *  looks down its own -z. */
export function mountFpv(cam: PerspectiveCamera, uptiltDeg: number): void {
  cam.rotation.order = "YXZ";
  cam.rotation.y = -Math.PI / 2;
  cam.rotation.x = (uptiltDeg * Math.PI) / 180;
}
