import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./theme/fonts.css";
import "./theme/tokens.css";
import "./theme/base.css";
import { App } from "./App";
import { limitContextMenu } from "./native";

async function start() {
  const query = new URLSearchParams(location.search);
  // In a plain browser (no Tauri), dev builds run on the mock core: ?mock=<scenario>.
  if (import.meta.env.DEV && !window.__TAURI_INTERNALS__) {
    const { installMock } = await import("./ipc/mock/install");
    installMock({ scenario: (query.get("mock") || "library") as never });
  }
  let root = <App />;
  if (import.meta.env.DEV && query.has("osd")) {
    // The OSD segment on its own until the Gear page frame mounts it.
    const { OsdSegment } = await import("./views/Gear/Osd/OsdSegment");
    root = <OsdSegment />;
  }
  if (import.meta.env.DEV && query.has("gallery")) {
    const { Gallery } = await import("./views/Gallery/Gallery");
    root = <Gallery />;
  }
  limitContextMenu();
  createRoot(document.getElementById("root")!).render(<StrictMode>{root}</StrictMode>);
}

start();
