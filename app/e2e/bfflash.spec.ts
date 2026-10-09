// The Betaflight flash preview on the mock core: the Settings > Gear switch, the FC row on
// the Firmware page, the flash sheet with its checks and recovery warning, and the report
// of a flash and of a bad read back. axe on the sheet.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const FC_ID = "fc-0a1b2c3d4e5f6071";
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

/** The saved FC becomes a V2 board on the older release the flash replaces. */
const makeFlashable = (app: AppFixture, version = "2025.12.5") => app.core(`c => { c.gear.devices.find(d => d.id === "${FC_ID}").identity = { board: "BETAFPVG473_V2", firmware: "Betaflight", version: "${version}" }; c.emit("gear-changed"); }`);

async function openFirmware(app: AppFixture, page: Page, preview: boolean) {
  await app.open("library", { settings: { bfFlashPreview: preview } });
  await side(page).getByRole("button", { name: /^Firmware/ }).click();
  await expect(page.getByRole("heading", { name: "Firmware", exact: true })).toBeVisible();
}

test("the preview is off by default and the FC row offers no flash", async ({ app, page }) => {
  await openFirmware(app, page, false);
  await makeFlashable(app);
  await page.getByRole("button", { name: "Check for updates" }).click();
  const fc = page.getByRole("row", { name: /Whoop FC/ });
  await expect(fc).toContainText("QuadCam checks Betaflight versions. It does not flash them.");
  await expect(fc.getByRole("button")).toHaveCount(0);
});

test("the Settings > Gear switch turns the preview on", async ({ app, page }) => {
  await app.open();
  await page.getByRole("button", { name: "Settings" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Settings" });
  await sheet.getByRole("tab", { name: "Gear" }).click();
  await sheet.getByLabel("Betaflight flashing (preview)").check();
  await sheet.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await app.method("settings_set")).some((v) => JSON.stringify(v).includes('"bfFlashPreview":true'))).toBe(true);
});

test("a flash shows its checks and the way back, and puts the settings back on Apply", async ({ app, page }) => {
  await openFirmware(app, page, true);
  await makeFlashable(app);
  await page.getByRole("button", { name: "Check for updates" }).click();
  const fc = page.getByRole("row", { name: /Whoop FC/ });
  await expect(fc.getByRole("button", { name: "Flash 2026.6.0…" })).toBeVisible();
  await fc.getByRole("button", { name: "Flash 2026.6.0…" }).click();
  const sheet = page.getByRole("dialog", { name: "Flash Whoop FC" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("2025.12.5");
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("betaflight_2026.6.0_BETAFPVG473_V2.hex");
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("Known board and version");
  await expect(sheet.getByRole("region", { name: "Warnings" })).toContainText("boot button");
  await expect(sheet.getByRole("region", { name: "Warnings" })).toContainText("preview");
  const axe = await new AxeBuilder({ page }).analyze();
  expect(axe.violations).toEqual([]);
  expect(await app.calls("gear_flash_click")).toEqual([]);
  await sheet.getByRole("button", { name: "Apply" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("Verified");
  const steps = result.getByRole("list", { name: "Steps" });
  await expect(steps).toContainText("Restart into the bootloader");
  await expect(steps).toContainText("Re-apply settings");
  await expect(result).toContainText("legacy_thing");
  expect((await app.calls("gear_flash_click")).length).toBe(1);
});

test("a bad read back is reported and the FC stays in its bootloader", async ({ app, page }) => {
  await openFirmware(app, page, true);
  await makeFlashable(app);
  await app.core(`c => { c.firmware.failNext = true; }`);
  await page.getByRole("button", { name: "Check for updates" }).click();
  await page.getByRole("row", { name: /Whoop FC/ }).getByRole("button", { name: "Flash 2026.6.0…" }).click();
  const sheet = page.getByRole("dialog", { name: "Flash Whoop FC" });
  await sheet.getByRole("button", { name: "Apply" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("stays in its bootloader");
  await expect(result).toContainText("boot button");
  await expect(result.getByRole("listitem").filter({ hasText: "Read back" })).toContainText("failed");
});

test("a board QuadCam cannot flash says why", async ({ app, page }) => {
  await openFirmware(app, page, true);
  await page.getByRole("button", { name: "Check for updates" }).click();
  const fc = page.getByRole("row", { name: /Whoop FC/ });
  await expect(fc).toContainText("QuadCam will not flash it: Board STM32F411 cannot be flashed by QuadCam yet.");
  await expect(fc.getByRole("button")).toHaveCount(0);
});
