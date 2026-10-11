// The app's own update on the mock core: the banner, Later, the download with progress, the
// restart held back while a job runs, Check for Updates from the menu, and the setting.
import { test, expect, dialog } from "./fixtures";

const banner = (page: import("@playwright/test").Page) => page.getByRole("status").filter({ hasText: /QuadCam 0\.99\.0/ });
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
  await dlg.getByRole("button", { name: "Check now" }).click();
  await expect(dlg.getByRole("status", { name: "Update status" })).toContainText("QuadCam 0.99.0 is available.");
  await box.uncheck();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => (await app.method("settings_set")).some((p) => (p.values as Record<string, unknown>)?.updateCheck === false)).toBe(true);
});
