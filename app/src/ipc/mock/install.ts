// Puts a fake Tauri runtime in the page, backed by MockCore: `__TAURI_INTERNALS__` for
// @tauri-apps/api (the new UI) and the `__TAURI__` global (`withGlobalTauri`, the legacy
// UI). Both UIs reach the same core, so one parity spec runs on either. `window.__qc` is the
// handle tests use: the call log, the core's state, and events to push.
import { MockCore, type MockOptions } from "./core";

type Callback = (payload: unknown) => void;

export interface MockHandle {
  core: MockCore;
  /** Every command the page sent, in order. */
  calls: MockCore["calls"];
  /** Delivers an event to the page as if the core emitted it. */
  emit: (event: string, payload?: unknown) => void;
  /** Simulates a drag of `paths` onto the window and a drop. */
  drop: (paths: string[]) => void;
}

declare global {
  interface Window {
    __qc?: MockHandle;
    __QC_MOCK__?: MockOptions;
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  }
}

/** Files the fixtures hold, served by the dev server; any other path gets a broken URL. */
const MEDIA_URL = "/e2e/fixtures/media/";

export function installMock(opts: MockOptions = {}): MockHandle {
  const callbacks = new Map<number, Callback>();
  let nextCallback = 1;
  // event -> listener id -> callback id
  const listeners = new Map<string, Map<number, number>>();
  let nextListener = 1;

  const emit = (event: string, payload: unknown = null) => {
    for (const [id, cb] of listeners.get(event) || []) callbacks.get(cb)?.({ event, id, payload });
  };
  // The real core emits from other threads, so events never arrive inside the call.
  const emitLater = (event: string, payload?: unknown) => setTimeout(() => emit(event, payload), 0);
  const core = new MockCore(emitLater, opts);
  const latency = opts.latency ?? 0;

  const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
    core.calls.push({ cmd, args: structuredClone(args) });
    if (latency) await new Promise((r) => setTimeout(r, latency));
    switch (cmd) {
      case "plugin:event|listen": {
        const id = nextListener++;
        const ev = String(args.event);
        if (!listeners.has(ev)) listeners.set(ev, new Map());
        listeners.get(ev)!.set(id, Number(args.handler));
        return id;
      }
      case "plugin:event|unlisten":
        listeners.get(String(args.event))?.delete(Number(args.eventId));
        return null;
      case "plugin:event|emit":
      case "plugin:event|emit_to":
        emitLater(String(args.event), args.payload);
        return null;
    }
    try {
      return structuredClone(core.handle(cmd, args));
    } catch (e) {
      throw typeof e === "string" ? e : String(e);
    }
  };

  const transformCallback = (cb: Callback, once = false) => {
    const id = nextCallback++;
    callbacks.set(id, (p) => {
      if (once) callbacks.delete(id);
      cb(p);
    });
    return id;
  };

  const convertFileSrc = (path: string) => MEDIA_URL + encodeURIComponent((path || "").split("/").pop() || "");

  window.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    unregisterCallback: (id: number) => callbacks.delete(id),
    convertFileSrc,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event: string, id: number) => void listeners.get(event)?.delete(id),
  };

  // The `withGlobalTauri` surface the legacy UI uses.
  const listen = async (event: string, handler: Callback) => {
    const id = (await invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler: transformCallback(handler) })) as number;
    return () => {
      listeners.get(event)?.delete(id);
    };
  };
  window.__TAURI__ = {
    core: { invoke, convertFileSrc, transformCallback },
    event: { listen, emit: (event: string, payload?: unknown) => invoke("plugin:event|emit", { event, payload }) },
    dialog: { open: (options: unknown = {}) => invoke("plugin:dialog|open", { options }) },
    opener: {
      revealItemInDir: (path: string) => invoke("plugin:opener|reveal_item_in_dir", { paths: [path] }),
      openPath: (path: string) => invoke("plugin:opener|open_path", { path }),
    },
    webview: {
      getCurrentWebview: () => ({
        onDragDropEvent: (handler: Callback) => listen("qc://drag-drop", handler),
      }),
    },
  };

  const handle: MockHandle = {
    core,
    calls: core.calls,
    emit,
    drop: (paths) => {
      emit("qc://drag-drop", { type: "enter", paths, position: { x: 400, y: 300 } });
      emit("tauri://drag-enter", { paths, position: { x: 400, y: 300 } });
      emit("qc://drag-drop", { type: "drop", paths, position: { x: 400, y: 300 } });
      emit("tauri://drag-drop", { paths, position: { x: 400, y: 300 } });
    },
  };
  window.__qc = handle;
  return handle;
}
