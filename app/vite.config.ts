/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The Tauri dev window loads this server. Pinned so another project's server never
// answers in its place; QUADCAM_DEV_PORT moves it (playwright.config.ts reads
// the same variable); `cargo tauri dev` always loads 4719 (`devUrl` in tauri.conf.json).
const PORT = Number(process.env.QUADCAM_DEV_PORT || 4719);

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: PORT, strictPort: true, host: "localhost" },
  // Pre-bundled up front, so a first page load never reloads mid-test to add one.
  optimizeDeps: {
    include: ["react", "react-dom", "react-dom/client", "react/jsx-runtime", "react/jsx-dev-runtime", "zustand", "@tanstack/react-virtual", "@tauri-apps/api/core", "@tauri-apps/api/event", "@tauri-apps/api/webview", "@tauri-apps/plugin-dialog", "@tauri-apps/plugin-opener", "three"],
  },
  preview: { port: PORT, strictPort: true, host: "localhost" },
  build: { outDir: "dist", target: "safari16", emptyOutDir: true },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    css: { modules: { classNameStrategy: "non-scoped" } },
  },
});
