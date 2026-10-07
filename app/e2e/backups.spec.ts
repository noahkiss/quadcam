import AxeBuilder from "@axe-core/playwright";
import { dialog, expect, test } from "./fixtures";
import type { Page } from "@playwright/test";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });
const bar = (page: Page) => page.getByRole("contentinfo", { name: "Gear status" });

async function radioBackups(page: Page) {
  await bar(page).getByRole("button", { name: "Radio: Connected" }).click();
  const radio = page.getByRole("region", { name: "Field radio" });
  await radio.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Backups" }).click();
  return radio;
}

test("a radio's backups: changes from the one before, files, pin, back up now", async ({ page, app }) => {
  await app.open("gear");
  const radio = await radioBackups(page);
  const table = radio.getByRole("table", { name: "Backups" });
  await expect(table.getByRole("row")).toHaveCount(4);
  await expect(table.getByRole("row").nth(1)).toContainText("Manual");
  // The newest is shown: its changes from the one before.
  const detail = radio.getByRole("region", { name: /^Backup of / });
  await expect(detail.getByRole("region", { name: "Files" })).toContainText("MODELS/model01.yml");
  await expect(detail.getByRole("region", { name: "MODELS/model01.yml" })).toContainText("countdownBeep: 2");

  await detail.getByRole("group", { name: "Backup view" }).getByRole("button", { name: "Files" }).click();
  await detail.getByRole("button", { name: /RADIO\/radio\.yml/ }).click();
  await expect(detail.getByLabel("RADIO/radio.yml")).toContainText("semver: 2.11.2");

  await table.getByRole("checkbox").nth(1).check();
  expect((await app.method("gear_backup_pin")).at(-1)).toMatchObject({ pinned: true });

  await radio.getByRole("button", { name: "Back up now" }).click();
  await expect(page.getByText("No changes since the last backup.")).toBeVisible();
  expect((await app.method("gear_backup")).at(-1)).toEqual({ mount: "/Volumes/RADIO" });
  await app.core(`c => { c.gear.dirty = "${RADIO_ID}"; }`);
  await radio.getByRole("button", { name: "Back up now" }).click();
  await expect(page.getByText(/^Backed up: /)).toBeVisible();
  await expect(table.getByRole("row")).toHaveCount(5);
});

test("a failed card check offers a repair, which asks first", async ({ page, app }) => {
  await app.open("gear");
  await app.core(`c => { c.gear.cardFails = ["${RADIO_ID}"]; }`);
  const radio = await radioBackups(page);
  await radio.getByRole("button", { name: "Check card" }).click();
  await expect(radio.getByText(/^Card check failed: /)).toBeVisible();
  await expect(bar(page).getByRole("button", { name: "Radio: Needs attention" })).toBeVisible();
  await radio.getByRole("button", { name: "Repair…" }).click();
  const ask = dialog(page, "Repair this card?");
  await ask.getByRole("button", { name: "Repair" }).click();
  await expect(page.getByText("Repaired. The card checks out.")).toBeVisible();
  await expect(radio.getByText(/^Card check failed: /)).toBeHidden();
  expect(await app.method("gear_card_repair")).toHaveLength(1);
  expect((await app.method("gear_card_repair"))[0]).toMatchObject({ confirm: true });
});

test("Storage: sizes per device, prune after a confirm, import a folder after a dry run", async ({ page, app }) => {
  await app.open("gear");
  await side(page).getByRole("button", { name: /^Storage/ }).click();
  await expect(page.getByRole("heading", { name: "Storage" })).toBeVisible();
  const table = page.getByRole("table", { name: "Storage by device" });
  await expect(table.getByRole("row", { name: /Field radio/ })).toContainText("3");
  await expect(table.getByRole("row", { name: /Whoop FC/ })).toBeVisible();

  await page.getByRole("button", { name: "Prune now" }).click();
  const prune = dialog(page, "Prune backups?");
  await expect(prune).toContainText("1 backup");
  await prune.getByRole("button", { name: "Prune" }).click();
  await expect(page.getByText("Pruned 1 backup.")).toBeVisible();
  expect((await app.method("gear_prune")).map((p) => p.dry_run)).toEqual([true, false]);

  await app.core(`c => { c.dialogAnswers = ["/Users/someone/old-backups"]; }`);
  await page.getByRole("button", { name: "Import backups…" }).click();
  const imp = dialog(page, "Import these backups?");
  await expect(imp).toContainText("2 new backups");
  await imp.getByRole("button", { name: "Import" }).click();
  await expect(page.getByText("Imported 2 backups.")).toBeVisible();
  const report = page.getByRole("region", { name: "Import" });
  await expect(report).toContainText("Already kept");
  await expect(report).toContainText("No saved Radio with board zorro");
  expect((await app.method("gear_import_backups")).map((p) => p.dry_run)).toEqual([true, false]);
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("Backups and Storage pass axe", async ({ app, page }) => {
      await app.open("gear");
      await app.core(`c => { c.gear.cardFails = ["${RADIO_ID}"]; }`);
      const radio = await radioBackups(page);
      await radio.getByRole("button", { name: "Check card" }).click();
      await expect(radio.getByText(/^Card check failed: /)).toBeVisible();
      const axe = async () => {
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      };
      await axe();
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/backups-${scheme}.png`, fullPage: true });
      await side(page).getByRole("button", { name: /^Storage/ }).click();
      await expect(page.getByRole("table", { name: "Storage by device" })).toBeVisible();
      await axe();
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/storage-${scheme}.png`, fullPage: true });
    });
  });
}
