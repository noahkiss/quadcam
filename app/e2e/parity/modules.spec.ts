import { dialog, expect, test } from "../fixtures";

test.beforeEach(async ({ app, page }) => {
  await app.open();
  await expect(page.getByRole("article").first()).toBeVisible();
});

test("Install shows the license first; Cancel downloads nothing, Download installs", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Modules" }).click();
  const row = dlg.getByRole("row", { name: /ffmpeg and ffprobe/ });
  await expect(row).toContainText("Not installed");

  await row.getByRole("button", { name: "Install ffmpeg and ffprobe" }).click();
  const prompt = dialog(page, "Install ffmpeg and ffprobe?");
  await expect(prompt).toBeVisible();
  await expect(prompt).toContainText("9.0.2");
  await expect(prompt.getByRole("link", { name: "GPL-3.0-or-later" })).toBeVisible();
  await expect(prompt).toContainText("https://example.invalid/source");
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(prompt).toBeHidden();
  expect(await app.method("module_install")).toEqual([]);

  await row.getByRole("button", { name: "Install ffmpeg and ffprobe" }).click();
  await prompt.getByRole("button", { name: "Download" }).click();
  await expect(row).toContainText("9.0.2");
  expect(await app.method("module_install")).toEqual([{ name: "ffmpeg", confirm: true }]);
  await expect(dlg).toContainText("modules/ffmpeg/9.0.2/ffmpeg");

  await row.getByRole("button", { name: "Remove ffmpeg and ffprobe" }).click();
  await dialog(page, "Remove ffmpeg and ffprobe?").getByRole("button", { name: "Remove" }).click();
  await expect(row).toContainText("Not installed");
  await expect(dlg).toContainText("/opt/homebrew/bin/ffmpeg");
});

test("Check for updates offers the newer pin", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Modules" }).click();
  await expect(dlg.getByRole("row", { name: /esptool/ })).toContainText("Not installed");
  // What the check finds: esptool 5.4.0 installed, 5.5.0 published.
  await app.core(`c => {
    const m = c.modules.find((x) => x.name === "esptool");
    m.installed = { name: "esptool", version: "5.4.0", installed_at: "", license: "GPL-2.0-or-later", license_url: "", source: "", homepage: "", assets: [], tools: {}, size: 1 };
    m.folder = "/x";
    m.newest = { ...m.pinned, version: "5.5.0" };
    m.update = true;
  }`);
  await dlg.getByRole("button", { name: "Check for updates" }).click();
  await expect(dlg.getByRole("status")).toHaveText("Updates are available.");
  await expect(dlg.getByRole("button", { name: "Update esptool" })).toHaveText("Update to 5.5.0");
  expect(await app.method("modules_check")).toHaveLength(1);
});

test("the ffmpeg source saves with Done", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Modules" }).click();
  await dlg.getByLabel("Use ffmpeg from").selectOption("homebrew");
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect(dlg).toBeHidden();
  const writes = (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values);
  expect(writes.at(-1)?.ffmpegSource).toBe("homebrew");
});

test("Acknowledgements shows the third-party notices", async ({ page }) => {
  await page.evaluate(() => window.__qc!.emit("menu", "acknowledgements"));
  const dlg = dialog(page, "Acknowledgements");
  await expect(dlg).toBeVisible();
  await expect(dlg.getByLabel("Third-party notices")).toContainText("QuadCam third-party notices");
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect(dlg).toBeHidden();
});
