/// <reference types="vitest/config" />
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { createReadStream, existsSync, statSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

// The Tauri dev window loads this server. Pinned so another project's server never
// answers in its place.
const PORT = 4719;

const LEGACY_DIR = fileURLToPath(new URL("../ui", import.meta.url));
const TYPES: Record<string, string> = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
};

// Serves the legacy ui/ folder at /legacy/ in dev, so QUADCAM_UI=legacy and the parity
// specs reach both UIs on the one pinned port.
function legacyUi(): Plugin {
  return {
    name: "quadcam-legacy-ui",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/legacy", (req, res, next) => {
        const path = decodeURIComponent((req.url || "/").split("?")[0]);
        const file = join(LEGACY_DIR, path === "/" ? "index.html" : path);
        if (!file.startsWith(LEGACY_DIR) || !existsSync(file) || !statSync(file).isFile()) return next();
        res.setHeader("Content-Type", TYPES[extname(file)] || "application/octet-stream");
        res.setHeader("Cache-Control", "no-store");
        createReadStream(file).pipe(res);
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), legacyUi()],
  clearScreen: false,
  server: { port: PORT, strictPort: true, host: "localhost" },
  preview: { port: PORT, strictPort: true, host: "localhost" },
  build: { outDir: "dist", target: "safari16", emptyOutDir: true },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    css: { modules: { classNameStrategy: "non-scoped" } },
  },
});
