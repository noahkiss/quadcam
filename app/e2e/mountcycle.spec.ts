import type { Page } from "@playwright/test";
import { dialog, expect, test } from "./fixtures";

const sheet = (page: Page) => dialog(page, "Import");
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

test("an import unmounts the card and Finish says it is safe to remove", async ({ app, page }) => {
  await app.open("card");
  await side(page).getByRole("button", { name: /DVR/ }).click();
  await expect(sheet(page)).toBeVisible();
  await page.keyboard.press("Meta+Enter");
  await expect(sheet(page).getByRole("heading", { name: /^Added \d+ clips?/ })).toBeVisible();
  await expect(sheet(page).getByText("Unmounted. Safe to remove.")).toBeVisible();
  // The card left the mounted list; the sidebar says it is unmounted.
  await sheet(page).getByRole("button", { name: "Done · show in Library" }).click();
  await expect(side(page).getByText("DVR unmounted")).toBeVisible();
});

test("an unmounted DVR card offers Mount, Import clips and Prepare card", async ({ app, page }) => {
  await app.open("gear");
  await app.core(`c => c.plug([c.gearSeed.radioConnected()], [c.gearSeed.dvrConnected()])`);
  await page.getByRole("button", { name: "DVR card: Safe to unplug" }).click();
  await expect(page.getByRole("button", { name: "Mount" })).toBeVisible();

  // Prepare card: the plan names the disk, and only the Erase click starts the erase.
  await page.getByRole("button", { name: "Prepare card…" }).click();
  expect((await app.method("card_prep_plan"))[0]).toMatchObject({ device: "dvr-5a6b7c8d9e0f1a2b" });
  const confirm = dialog(page, "Erase the card?");
  await expect(confirm).toContainText("Erase disk9");
  await page.keyboard.press("Enter");
  await expect(confirm).toBeVisible();
  expect(await app.calls("card_prep_click")).toHaveLength(0);
  await confirm.getByRole("button", { name: "Erase" }).click();
  await expect.poll(async () => (await app.calls("card_prep_click")).length).toBe(1);
  expect((await app.calls("card_prep_click"))[0].args).toMatchObject({ req: { confirm: true } });
});

test("Import clips mounts an unmounted card and loads it by device id", async ({ app, page }) => {
  await app.open("gear");
  await app.core(`c => c.plug([c.gearSeed.radioConnected()], [c.gearSeed.dvrConnected()])`);
  await page.getByRole("button", { name: "DVR card: Safe to unplug" }).click();
  await page.getByRole("button", { name: "Import clips…" }).click();
  await expect(sheet(page)).toBeVisible();
  expect((await app.method("load"))[0]).toEqual({ source: null, device: "dvr-5a6b7c8d9e0f1a2b", join: null });
});

test("a DJI card has no Format card, and the Finish step offers Prepare card", async ({ app, page }) => {
  await app.open("dji");
  await page.getByRole("button", { name: "Import…" }).first().click();
  await expect(sheet(page)).toBeVisible();
  await page.keyboard.press("Meta+Enter");
  await expect(sheet(page).getByRole("heading", { name: /^Added \d+ clips?/ })).toBeVisible();
  const box = sheet(page).getByRole("region", { name: "Prepare card" });
  await expect(box).toBeVisible();
  await expect(sheet(page).getByRole("region", { name: "Format card" })).toHaveCount(0);
  await box.getByRole("button", { name: "Prepare card…" }).click();
  const confirm = dialog(page, "Erase the card?");
  await expect(confirm).toContainText("exFAT");
  await confirm.getByRole("button", { name: "Cancel" }).click();
  expect(await app.calls("card_prep_click")).toHaveLength(0);
});
