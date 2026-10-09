// The core's events, from the generated bindings. Each returns its unlisten function.
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { events } from "../bindings";
import * as N from "./normalize";
import type { AgentApplyRequest, AgentFormatRequest, CalibrateView, ClipResult, DeviceChanged, ImportProgress, LibraryTask, RadioEvent, StageProgress } from "./types";

export interface CoreEvents {
  "library-changed": null;
  "session-changed": null;
  "settings-changed": null;
  "volumes-changed": null;
  /** `gear.json`, a job's hold or a reminder changed: read `gear_status` again. */
  "gear-changed": null;
  /** What is plugged in changed. */
  "device-changed": DeviceChanged;
  "library-task": LibraryTask;
  progress: StageProgress;
  "import-progress": ImportProgress;
  "import-result": ClipResult;
  "agent-format-request": AgentFormatRequest;
  "agent-format-closed": number;
  "agent-apply-request": AgentApplyRequest;
  "agent-apply-closed": number;
  /** A native menu item: its id. */
  menu: string;
  /** The radio in USB Joystick mode, while `gearRadioWatch(true)` runs. */
  "radio-input": RadioEvent;
  /** The sim's calibration session, while `gearSimCalibrate` runs one. */
  "sim-calibration-event": CalibrateView;
}

type Listen = (cb: (payload: never) => void) => Promise<() => void>;
const wrap = <T,>(e: { listen: (cb: (ev: { payload: T }) => void) => Promise<() => void> }, map: (p: T) => unknown = (p) => p): Listen => (cb) => e.listen((ev) => (cb as (p: unknown) => void)(map(ev.payload)));

const SOURCES: Record<keyof CoreEvents, Listen> = {
  "library-changed": wrap(events.libraryChanged),
  "session-changed": wrap(events.sessionChanged),
  "settings-changed": wrap(events.settingsChanged),
  "volumes-changed": wrap(events.volumesChanged),
  "gear-changed": wrap(events.gearChanged),
  "device-changed": wrap(events.deviceChanged),
  "library-task": wrap(events.libraryTask),
  progress: wrap(events.progress),
  "import-progress": wrap(events.importProgress, N.importProgress),
  "import-result": wrap(events.importResult, N.result),
  "agent-format-request": wrap(events.agentFormatRequest),
  "agent-format-closed": wrap(events.agentFormatClosed),
  "agent-apply-request": wrap(events.agentApplyRequest),
  "agent-apply-closed": wrap(events.agentApplyClosed),
  menu: wrap(events.menu),
  "radio-input": wrap(events.radioInput, N.radioEvent),
  "sim-calibration-event": wrap(events.simCalibrationEvent),
};

export function on<K extends keyof CoreEvents>(name: K, handler: (payload: CoreEvents[K]) => void): Promise<() => void> {
  return SOURCES[name](handler as (p: never) => void);
}

export type DragDrop = { type: "enter" | "over"; paths?: string[] } | { type: "drop"; paths: string[] } | { type: "leave" };

/** Files dragged over and dropped on the window. */
export function onDragDrop(handler: (e: DragDrop) => void): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent((e) => handler(e.payload as DragDrop));
}
