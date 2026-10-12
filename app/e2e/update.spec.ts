// The app's own update on the mock core: the banner, Later, the download with progress, the
// restart held back while a job runs, Check for Updates from the menu, the setting, and the
// same Restart to update under the version in Settings.
import AxeBuilder from "@axe-core/playwright";
import { test, expect, dialog, axeSchemes } from "./fixtures";

const banner = (page: import("@playwright/test").Page) => page.getByRole("status").filter({ hasText: /QuadCam 0\.99\.0/ });
const banner14 = (page: import("@playwright/test").Page) => page.getByRole("status").filter({ hasText: /QuadCam 0\.14\.0/ });
const check = (page: import("@playwright/test").Page) => page.evaluate(() => window.__qc!.emit("menu", "check-updates"));

test.beforeEach(async ({ app }) => {
  await app.open("library");
  await app.core("(c) => { c.update.feed = '0.99.0'; }");
});

test("Check for Updates shows the banner; Later hides it", async ({ app, page }) => {
  await expect(banner(page)).toBeHidden();
  await check(page);
  await expect(banner(page)).toContainText("QuadCam 0.99.0 is available.");
  await expect(page.getByRole("link", { name: "Release notes" })).toBeVisible();
  await page.getByRole("button", { name: "Later" }).click();
  await expect(banner(page)).toBeHidden();
  expect(await app.core<number>("(c) => c.update.restarts")).toBe(0);
  // Checking again shows it again.
  await check(page);
  await expect(banner(page)).toBeVisible();
});

test("Restart to update downloads with progress, then restarts", async ({ app, page }) => {
  await check(page);
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect(page.getByRole("progressbar", { name: "Download" })).toBeVisible();
  await expect(banner(page)).toContainText(/Downloading QuadCam 0\.99\.0… \d+%/);
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
  expect((await app.calls("update_restart")).length).toBe(1);
});

test("a running job holds the restart back until it ends", async ({ app, page }) => {
  await app.core("(c) => { c.update.job = 'the import or card step'; }");
  await check(page);
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect(banner(page)).toContainText("QuadCam 0.99.0 is ready. It can restart after the import or card step ends.");
  const restart = page.getByRole("button", { name: "Restart to update" });
  await expect(restart).toBeDisabled();
  expect((await app.calls("update_restart")).length).toBe(0);
  // The job ends: the core says so, and the restart is offered, not done.
  await page.evaluate(() => {
    const c = window.__qc!.core;
    c.update.job = null;
    window.__qc!.emit("update-changed", { ...c.update.status, waiting_for: null });
  });
  await expect(restart).toBeEnabled();
  expect(await app.core<number>("(c) => c.update.restarts")).toBe(0);
  await restart.click();
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
});

test("a failed download offers Try again", async ({ app, page }) => {
  await app.core("(c) => { c.update.failDownload = true; }");
  await check(page);
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect(page.getByRole("status").filter({ hasText: "The update did not download" })).toBeVisible();
  await page.getByRole("button", { name: "Try again" }).click();
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
});

test("a newer release replaces a known one in the banner, Settings and the install", async ({ app, page }) => {
  const any = (v: string) => page.getByRole("status").filter({ hasText: `QuadCam ${v}` }).first();
  await app.core("(c) => { c.update.feed = '0.14.0'; }");
  await check(page);
  await expect(any("0.14.0")).toContainText("QuadCam 0.14.0 is available.");
  // The next check finds 0.17.0: it replaces 0.14.0.
  await app.core("(c) => { c.update.feed = '0.17.0'; }");
  await check(page);
  await expect(any("0.17.0")).toContainText("QuadCam 0.17.0 is available.");
  await expect(page.getByRole("status").filter({ hasText: "QuadCam 0.14.0" })).toHaveCount(0);
  const dlg = await advanced(page);
  await expect(dlg.getByRole("status", { name: "Update status" })).toContainText("QuadCam 0.17.0 is available.");
  await dlg.getByRole("button", { name: "Done" }).click();
  // Published while 0.17.0 was known: Restart to update checks once more and installs 0.18.0.
  await app.core("(c) => { c.update.feed = '0.18.0'; }");
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
  expect(await app.core<string>("(c) => c.update.installed")).toBe("0.18.0");
});

test("a downloaded update gives way to a newer release", async ({ app, page }) => {
  await app.core("(c) => { c.update.feed = '0.14.0'; c.update.job = 'the device job'; }");
  await check(page);
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect(banner14(page)).toContainText("QuadCam 0.14.0 is ready.");
  await app.core("(c) => { c.update.feed = '0.17.0'; c.update.job = null; }");
  await check(page);
  await expect(page.getByRole("status").filter({ hasText: "QuadCam 0.17.0 is available." })).toBeVisible();
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
  expect(await app.core<string>("(c) => c.update.installed")).toBe("0.17.0");
});

test("nothing at or below the running version is offered", async ({ app, page }) => {
  const running = await app.core<string>("(c) => c.update.status.current");
  await app.core(`(c) => { c.update.feed = '${running}'; }`);
  await check(page);
  await expect(page.getByText(/is the newest version/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Restart to update" })).toHaveCount(0);
});

test("nothing newer and a failed check are quiet toasts", async ({ app, page }) => {
  await app.core("(c) => { c.update.feed = null; }");
  await check(page);
  await expect(page.getByText(/is the newest version/)).toBeVisible();
  await app.core("(c) => { c.update.feed = 'fail'; }");
  await check(page);
  await expect(page.getByText("Could not check for updates: the feed did not answer")).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("Settings > Advanced turns the automatic check off and checks on request", async ({ app, page }) => {
  await page.evaluate(() => window.__qc!.emit("menu", "settings"));
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Advanced" }).click();
  const box = dlg.getByRole("checkbox", { name: /Check for updates automatically/ });
  await expect(box).toBeChecked();
  await expect(dlg.getByRole("status", { name: "Update status" })).toContainText("Not checked yet.");
  await dlg.getByRole("button", { name: "Check for Updates" }).click();
  await expect(dlg.getByRole("status", { name: "Update status" })).toContainText("QuadCam 0.99.0 is available.");
  await box.uncheck();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => (await app.method("settings_set")).some((p) => (p.values as Record<string, unknown>)?.updateCheck === false)).toBe(true);
});

async function advanced(page: import("@playwright/test").Page) {
  await page.evaluate(() => window.__qc!.emit("menu", "settings"));
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Advanced" }).click();
  return dlg;
}

test("Settings > Advanced offers Restart to update with progress and Release notes", async ({ app, page }) => {
  const dlg = await advanced(page);
  const line = dlg.getByRole("status", { name: "Update status" });
  await expect(dlg.getByRole("button", { name: "Restart to update" })).toHaveCount(0);
  await dlg.getByRole("button", { name: "Check for Updates" }).click();
  await expect(line).toContainText("QuadCam 0.99.0 is available.");
  await expect(dlg.getByRole("link", { name: "Release notes" })).toBeVisible();
  await expect(dlg.getByRole("button", { name: "Check for Updates" })).toHaveCount(0);
  await expect(dlg.getByText("QuadCam 0.99.0 is available.", { exact: true })).toBeVisible();
  await dlg.getByRole("button", { name: "Restart to update" }).click();
  await expect(dlg.getByRole("progressbar", { name: "Download" })).toBeVisible();
  await expect(line).toContainText(/Downloading QuadCam 0\.99\.0… \d+%/);
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
  expect((await app.calls("update_restart")).length).toBe(1);
});

test("Settings > Advanced holds the restart back while a job runs", async ({ app, page }) => {
  await app.core("(c) => { c.update.job = 'the device job'; }");
  const dlg = await advanced(page);
  await dlg.getByRole("button", { name: "Check for Updates" }).click();
  const restart = dlg.getByRole("button", { name: "Restart to update" });
  await restart.click();
  await expect(dlg.getByRole("status", { name: "Update status" })).toContainText("QuadCam 0.99.0 is ready. It can restart after the device job ends.");
  await expect(restart).toBeDisabled();
  expect((await app.calls("update_restart")).length).toBe(0);
  await page.evaluate(() => {
    const c = window.__qc!.core;
    c.update.job = null;
    window.__qc!.emit("update-changed", { ...c.update.status, waiting_for: null });
  });
  await expect(restart).toBeEnabled();
  await restart.click();
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("Settings > Advanced with an update passes axe", async ({ page }) => {
      const dlg = await advanced(page);
      await dlg.getByRole("button", { name: "Check for Updates" }).click();
      await expect(dlg.getByRole("button", { name: "Restart to update" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
    });
  });
}

test("Check for Updates under the version becomes Restart to update on any pane", async ({ app, page }) => {
  await page.evaluate(() => window.__qc!.emit("menu", "settings"));
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Check for Updates" }).click();
  const restart = dlg.getByRole("button", { name: "Restart to update" });
  await expect(restart).toBeVisible();
  await expect(dlg.getByRole("button", { name: "Check for Updates" })).toHaveCount(0);
  await restart.click();
  await expect.poll(() => app.core<number>("(c) => c.update.restarts")).toBe(1);
});
