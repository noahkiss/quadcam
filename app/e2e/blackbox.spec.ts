// An FC's Blackbox segment on the mock core: pull, the stored pulls with their guessed
// flights, the setting that decides the erase, and the manual erase asking first.
import AxeBuilder from "@axe-core/playwright";
import { dialog, expect, test, type AppFixture } from "./fixtures";
import type { Page } from "@playwright/test";

async function openBlackbox(app: AppFixture, page: Page) {
  await app.open("gear");
  await app.core(`c => {
    const fc = { id: c.gearSeed.FC.id, kind: "fc", link: { kind: "serial", port: "/dev/cu.usbmodem0", vid: 0x0483, pid: 0x5740, product: null }, identity: {} };
    c.plug([fc], []);
  }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Connected/ }).click();
  await page.getByRole("button", { name: /Whoop FC/ }).first().click();
  const fc = page.getByRole("region", { name: "Whoop FC" });
  await fc.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Blackbox" }).click();
  return fc;
}

test("Pull blackbox stores the flash and keeps it while the erase setting is off", async ({ app, page }) => {
  const fc = await openBlackbox(app, page);
  await expect(fc.getByText("No blackbox pulled yet.")).toBeVisible();
  await expect(fc.getByRole("button", { name: "Erase flash" })).toBeDisabled();
  await fc.getByRole("button", { name: "Pull blackbox" }).click();
  await expect(page.getByText("3 logs stored (4 MB). Flash kept.")).toBeVisible();
  expect((await app.method("gear_blackbox_pull_click")).at(-1)).toEqual({ port: "/dev/cu.usbmodem0", keep: false, confirm: true });
  const table = fc.getByRole("table", { name: "Blackbox pulls" });
  await expect(table.getByRole("row")).toHaveCount(2);
  await expect(table.getByRole("row").nth(1)).toContainText("Kept");
  // The logs, and the guess label under them.
  const logs = fc.getByRole("table", { name: "Logs" });
  await expect(logs.getByRole("row")).toHaveCount(4);
  await expect(fc.getByText(/pair .* by order, newest with newest/)).toBeVisible();
  expect(await new AxeBuilder({ page }).analyze().then((r) => r.violations)).toEqual([]);
});

test("with the setting on, a pull erases the flash and says it is safe to unplug", async ({ app, page }) => {
  const fc = await openBlackbox(app, page);
  await app.core(`c => { c.settings.values.gearEraseBlackbox = true; }`);
  await fc.getByRole("button", { name: "Pull blackbox" }).click();
  await expect(page.getByText("3 logs stored (4 MB). Flash erased. Safe to unplug.")).toBeVisible();
  await expect(fc.getByRole("table", { name: "Blackbox pulls" }).getByRole("row").nth(1)).toContainText("Erased");
  // Nothing left to pull.
  await fc.getByRole("button", { name: "Pull blackbox" }).click();
  await expect(page.getByText("The blackbox flash is empty.")).toBeVisible();
});

test("Erase flash asks first and needs a stored pull of what the flash holds", async ({ app, page }) => {
  const fc = await openBlackbox(app, page);
  await fc.getByRole("button", { name: "Pull blackbox" }).click();
  await expect(fc.getByRole("table", { name: "Blackbox pulls" })).toBeVisible();
  await fc.getByRole("button", { name: "Erase flash" }).click();
  const ask = dialog(page, "Erase the blackbox flash?");
  await ask.getByRole("button", { name: "Cancel" }).click();
  expect(await app.method("gear_blackbox_erase_click")).toHaveLength(0);
  await fc.getByRole("button", { name: "Erase flash" }).click();
  await dialog(page, "Erase the blackbox flash?").getByRole("button", { name: "Erase" }).click();
  expect((await app.method("gear_blackbox_erase_click")).at(-1)).toEqual({ port: "/dev/cu.usbmodem0", confirm: true });
  await expect(page.getByText("Flash erased. Safe to unplug.")).toBeVisible();
  await expect(fc.getByRole("table", { name: "Blackbox pulls" }).getByRole("row").nth(1)).toContainText("Erased");
});

test("Settings > Gear holds the blackbox erase setting and the Pull blackbox step", async ({ app, page }) => {
  await app.open();
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Gear" }).click();
  await expect(dlg.getByLabel("Erase blackbox after download")).not.toBeChecked();
  // Each kind's steps sit in its pop-up, a check mark on each step that runs.
  const steps = async (kind: string) => {
    await dlg.getByRole("button", { name: new RegExp(`^${kind}: `) }).click();
    return page.getByRole("menu", { name: `Steps for ${kind}` });
  };
  let menu = await steps("FC");
  await expect(menu.getByRole("menuitemcheckbox", { name: "Pull blackbox" })).toHaveAttribute("aria-checked", "false");
  // Apply ready changes runs for an FC and a radio only.
  await expect(menu.getByRole("menuitemcheckbox", { name: "Apply ready changes" })).toBeVisible();
  await page.keyboard.press("Escape");
  menu = await steps("Radio");
  await expect(menu.getByRole("menuitemcheckbox", { name: "Pull blackbox" })).toHaveCount(0);
  await expect(menu.getByRole("menuitemcheckbox", { name: "Apply ready changes" })).toBeVisible();
  await page.keyboard.press("Escape");
  await dlg.getByLabel("Erase blackbox after download").check();
  menu = await steps("FC");
  await menu.getByRole("menuitemcheckbox", { name: "Pull blackbox" }).click();
  await expect(dlg.getByRole("button", { name: "FC: Back up, Pull blackbox" })).toBeVisible();
  await dlg.getByRole("button", { name: "Done" }).click();
  const w = (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values).at(-1)!;
  expect(w.gearEraseBlackbox).toBe(true);
  expect(w.gearOnConnect).toMatchObject({ fc: ["backup", "blackbox"] });
});
