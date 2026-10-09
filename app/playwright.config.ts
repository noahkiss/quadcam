import { defineConfig, devices } from "@playwright/test";

const PORT = Number(process.env.QUADCAM_DEV_PORT) || 4719;

// Headless WebKit (the engine of the app's WKWebView) against the Vite dev server, with
// the mock core injected.
export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/global-setup.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    ...devices["Desktop Safari"],
    baseURL: `http://localhost:${PORT}`,
    viewport: { width: 1280, height: 820 },
    headless: true,
    trace: "retain-on-failure",
  },
  projects: [{ name: "webkit", use: { browserName: "webkit" } }],
  webServer: {
    command: "pnpm dev",
    url: `http://localhost:${PORT}/`,
    reuseExistingServer: !process.env.CI,
  },
});
