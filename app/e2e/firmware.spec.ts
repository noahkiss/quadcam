// Firmware and splash on the mock core (WP10): the Firmware page and its check (the network is
// read only when asked), a source that fails, an EdgeTX flash through the apply sheet with its
// DFU guard, and the Splash segment from a PNG to the flash plan. axe on the page.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

async function openFirmware(app: AppFixture, page: Page) {
  await app.open();
  await side(page).getByRole("button", { name: /^Firmware/ }).click();
  await expect(page.getByRole("heading", { name: "Firmware", exact: true })).toBeVisible();
}

/** The saved radio becomes a Pocket on EdgeTX 2.12.3 (a firmware pair QuadCam has proven) or 2.12.4 (the splash pair too). */
const makePocket = (app: AppFixture, version = "2.12.3") => app.core(`c => { c.gear.devices.find(d => d.id === "${RADIO_ID}").identity = { board: "pocket", firmware: "EdgeTX", version: "${version}" }; c.emit("gear-changed"); }`);

test("the page reads the network only when asked, and says what it found", async ({ app, page }) => {
  await openFirmware(app, page);
  const table = page.getByRole("table", { name: "Firmware" });
  await expect(page.getByText("Not checked yet.")).toBeVisible();
  await expect(table.getByRole("row", { name: /Field radio/ })).toContainText("Unknown");
  expect((await app.method("gear_firmware")).every((c) => c.check !== true)).toBe(true);

  await page.getByRole("button", { name: "Check for updates" }).click();
  await expect(page.getByText(/^Checked /)).toBeVisible();
  expect((await app.method("gear_firmware")).at(-1)).toMatchObject({ check: true });
  const radio = table.getByRole("row", { name: /Field radio/ });
  await expect(radio).toContainText("2.11.2");
  await expect(radio).toContainText("2.12.4");
  await expect(radio).toContainText("Update available");
  await expect(radio).toContainText("QuadCam will not flash it: Board tx16s is not proven.");
  await expect(radio.getByRole("button")).toHaveCount(0);
  const fc = table.getByRole("row", { name: /Whoop FC/ });
  await expect(fc).toContainText("Update available");
  await expect(fc).toContainText("QuadCam checks Betaflight versions. It does not flash them.");
  await expect(side(page).getByRole("button", { name: /^Firmware/ })).toContainText("2");

  const axe = await new AxeBuilder({ page }).analyze();
  expect(axe.violations).toEqual([]);
});

test("a source that fails is reported and the others still show", async ({ app, page }) => {
  await openFirmware(app, page);
  await app.core(`c => { c.firmware.failing = ["Betaflight"]; }`);
  await page.getByRole("button", { name: "Check for updates" }).click();
  await expect(page.getByText(/Betaflight: The download failed/)).toBeVisible();
  await expect(page.getByRole("row", { name: /Field radio/ })).toContainText("Update available");
  await expect(page.getByRole("row", { name: /Whoop FC/ })).toContainText("Unknown");
});

test("a flash shows its checks, needs the DFU radio, and writes only on Apply", async ({ app, page }) => {
  await openFirmware(app, page);
  await makePocket(app);
  await app.core(`c => { c.firmware.dfu = 0; }`);
  await page.getByRole("button", { name: "Check for updates" }).click();
  await page.getByRole("button", { name: "Flash 2.12.4…" }).click();
  const sheet = page.getByRole("dialog", { name: "Flash Field radio" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("2.12.3");
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("fw-radiomaster-pocket-v2.12.4.bin");
  const checks = sheet.getByRole("list", { name: "Checks" });
  await expect(checks).toContainText("Known board and version");
  await expect(checks).toContainText("No radio is in DFU mode.");
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeDisabled();
  expect(await app.calls("gear_flash_click")).toEqual([]);
  await sheet.getByRole("button", { name: "Cancel" }).click();

  await app.core(`c => { c.firmware.dfu = 1; }`);
  await page.getByRole("button", { name: "Flash 2.12.4…" }).click();
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeEnabled();
  expect(await app.method("gear_flash")).toEqual([]);
  await sheet.getByRole("button", { name: "Apply" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("Verified");
  await expect(result.getByRole("list", { name: "Steps" })).toContainText("Back up the current firmware");
  await expect(result.getByRole("list", { name: "Steps" })).toContainText("Leave DFU");
  expect((await app.calls("gear_flash_click")).length).toBe(1);
});

test("a bad read back is reported and the radio stays in DFU", async ({ app, page }) => {
  await openFirmware(app, page);
  await makePocket(app);
  await app.core(`c => { c.firmware.failNext = true; }`);
  await page.getByRole("button", { name: "Check for updates" }).click();
  await page.getByRole("button", { name: "Flash 2.12.4…" }).click();
  const sheet = page.getByRole("dialog", { name: "Flash Field radio" });
  await sheet.getByRole("button", { name: "Apply" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("The radio stays in DFU mode");
  await expect(result.getByRole("listitem").filter({ hasText: "Read back" })).toContainText("failed");
  await expect(sheet.getByRole("button", { name: "Restore backup" })).toHaveCount(0);
});

async function openSplash(app: AppFixture, page: Page) {
  await app.open();
  await side(page).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Field radio/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Splash" }).click();
}

test("the splash segment previews a PNG, refuses a colour radio, and plans a flash with it", async ({ app, page }) => {
  await openSplash(app, page);
  await app.core(`c => { c.dialogAnswers.push(["/Users/me/Pictures/logo.png"]); }`);
  await page.getByRole("button", { name: "Choose image…" }).click();
  // The radio is a TX16S: its splash is not supported yet.
  await expect(page.getByText("This radio's splash format is not supported yet (board tx16s).")).toBeVisible();
  await expect(page.getByRole("button", { name: "Make firmware…" })).toBeDisabled();

  await makePocket(app, "2.12.4");
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Overview" }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Splash" }).click();
  await app.core(`c => { c.dialogAnswers.push(["/Users/me/Pictures/logo.png"]); }`);
  await page.getByRole("button", { name: "Choose image…" }).click();
  await expect(page.getByRole("img", { name: /Splash preview, 128 by 64/ })).toBeVisible();
  await page.getByLabel("Threshold").fill("200");
  await page.getByLabel("Invert").check();
  await expect.poll(async () => (await app.method("gear_splash")).at(-1)).toMatchObject({ image: "/Users/me/Pictures/logo.png", threshold: 200, invert: true, board: "pocket" });

  await page.getByRole("button", { name: "Make firmware…" }).click();
  const sheet = page.getByRole("dialog", { name: "Flash Field radio" });
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("Splash markers");
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("Splash:");
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  const click = JSON.stringify((await app.calls("gear_flash_click")).at(-1));
  expect(click).toContain('"threshold":200');
  expect(click).toContain('"invert":true');
});
