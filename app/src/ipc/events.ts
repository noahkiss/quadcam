// The core's events, typed. Each returns its unlisten function.
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { AgentFormatRequest, ClipResult, ImportProgress, LibraryTask, StageProgress } from "./types";

export interface CoreEvents {
  "library-changed": null;
  "session-changed": null;
  "settings-changed": null;
  "volumes-changed": null;
  "library-task": LibraryTask;
  progress: StageProgress;
  "import-progress": ImportProgress;
  "import-result": ClipResult;
  "agent-format-request": AgentFormatRequest;
  "agent-format-closed": number;
  /** A native menu item: its id. */
  menu: string;
}

export function on<K extends keyof CoreEvents>(name: K, handler: (payload: CoreEvents[K]) => void): Promise<() => void> {
  return listen<CoreEvents[K]>(name, (e) => handler(e.payload));
}

export type DragDrop = { type: "enter" | "over"; paths?: string[] } | { type: "drop"; paths: string[] } | { type: "leave" };

/** Files dragged over and dropped on the window. */
export function onDragDrop(handler: (e: DragDrop) => void): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent((e) => handler(e.payload as DragDrop));
}
