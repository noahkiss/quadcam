// ExpressLRS (preview) on the mock core: off until the setting is on, a read through a radio
// and an FC, option changes through the apply sheet, and a flash plan and run with the binding
// phrase never shown. axe on the page.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

async function openFirmware(app: AppFixture, page: Page, settings: Record<string, unknown> = {}) {
  await app.open("library", { settings });
  await side(page).getByRole("button", { name: /^Firmware/ }).click();
  await expect(page.getByRole("heading", { name: "ExpressLRS (preview)" })).toBeVisible();
}

test("the ELRS tools are off until the setting is on", async ({ app, page }) => {
  await openFirmware(app, page);
  const box = page.getByRole("checkbox", { name: "Show the ELRS tools" });
  await expect(box).not.toBeChecked();
  await expect(page.getByRole("button", { name: /^Read the module/ })).toHaveCount(0);
  await box.check();
  await expect(page.getByRole("button", { name: "Read the module in Field radio" })).toBeVisible();
  await expect(page.getByText("QuadCam has not tried these on a real radio")).toBeVisible();
  expect((await app.method("settings_set")).some((v) => (v.values as Record<string, unknown>).elrsPreview === true)).toBe(true);
  const axe = await new AxeBuilder({ page }).analyze();
  expect(axe.violations).toEqual([]);
});

test("a read through the radio and the FC fills the table and warns about mixed majors", async ({ app, page }) => {
  await openFirmware(app, page, { elrsPreview: true });
  await page.getByRole("button", { name: "Read the module in Field radio" }).click();
  const table = page.getByRole("table", { name: "ExpressLRS devices" });
  const tx = table.getByRole("row", { name: /Transmitter/ });
  await expect(tx).toContainText("RM Pocket 2.4GHz TX");
  await expect(tx).toContainText("4.1.0");
  await expect(page.getByText(/restart the radio when you are done/)).toBeVisible();
  await page.getByRole("button", { name: "Read the receiver in Whoop FC" }).click();
  const rx = table.getByRole("row", { name: /Receiver/ });
  await expect(rx).toContainText("3.5.3");
  await expect(page.getByText(/do not link/)).toBeVisible();
  expect(await app.calls("gear_elrs_read")).toHaveLength(2);
});

test("an option change is staged and applied through the apply sheet", async ({ app, page }) => {
  await openFirmware(app, page, { elrsPreview: true });
  await page.getByRole("button", { name: "Read the module in Field radio" }).click();
  await page.getByRole("button", { name: "Options" }).click();
  const group = page.getByRole("group", { name: /Options of/ });
  await group.getByLabel("Packet rate").selectOption("250Hz(-108dBm)");
  await group.getByRole("button", { name: "Stage changes" }).click();
  const sheet = page.getByRole("dialog", { name: /Apply to/ });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("packet_rate: 250Hz(-108dBm)");
  await expect(sheet).toContainText("QuadCam reads the device again first");
  const staged = await app.method("gear_change_stage");
  expect(staged.at(-1)).toMatchObject({ edits: [{ kind: "elrs_options", options: [{ option: "packet_rate", value: "250Hz(-108dBm)" }] }] });
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("reports the new options");
});

test("a flash needs the phrase, then runs only on Apply", async ({ app, page }) => {
  await openFirmware(app, page, { elrsPreview: true });
  await page.getByRole("button", { name: "Read the receiver in Whoop FC" }).click();
  await page.getByRole("button", { name: "Flash 4.1.0…" }).click();
  const sheet = page.getByRole("dialog", { name: /^Flash / });
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("A binding phrase is set");
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("Set the binding phrase first");
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeDisabled();
  await sheet.getByRole("button", { name: "Cancel" }).click();

  await page.getByLabel("Binding phrase").fill("bench-phrase-not-real");
  await page.getByRole("button", { name: "Save phrase" }).click();
  await page.getByRole("button", { name: "Flash 4.1.0…" }).click();
  await expect(sheet.getByRole("region", { name: "Firmware image" })).toContainText("UID fingerprint");
  await expect(sheet).not.toContainText("bench-phrase-not-real");
  expect(await app.calls("gear_elrs_flash_click")).toEqual([]);
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  expect(await app.calls("gear_elrs_flash_click")).toHaveLength(1);
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText("bench-phrase-not-real")).toHaveCount(0);
});
