// Restore a saved firmware copy on the mock core: the Saved firmware copies table, the
// restore sheet with its checks, the backup it states, the write only on Restore, a refused
// copy, and axe on the page and the sheet in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const COPY_ID = `${RADIO_ID}/2026-10-01T120000-read`;
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

/** The saved radio as a Pocket, one saved copy of it, then the Firmware page. */
async function openWithCopy(app: AppFixture, page: Page, copy: Record<string, unknown> = {}) {
  await app.open();
  await app.core(`c => {
    c.gear.devices.find(d => d.id === "${RADIO_ID}").identity = { board: "pocket", firmware: "EdgeTX", version: "2.12.4" };
    c.firmware.copies.push(Object.assign({ id: "${COPY_ID}", device: "${RADIO_ID}", taken_at: "2026-10-01T12:00:00Z", kind: "read", size: 524288, sha256: "ab".repeat(32), image_board: "pocket", image_version: "2.12.3" }, ${JSON.stringify(copy)}));
    c.emit("gear-changed");
  }`);
  await side(page).getByRole("button", { name: /^Firmware/ }).click();
  await expect(page.getByRole("heading", { name: "Firmware", exact: true })).toBeVisible();
  // The copies sit behind a disclosure.
  await page.getByRole("region", { name: "Saved firmware copies" }).getByText("Saved firmware copies", { exact: true }).click();
}

const copies = (page: Page) => page.getByRole("table", { name: "Saved firmware copies" });

test("a saved copy restores through the sheet, which names the copy and the backup, and writes only on Restore", async ({ app, page }) => {
  await openWithCopy(app, page);
  const row = copies(page).getByRole("row", { name: /Read firmware/ });
  await expect(row).toContainText("Field radio");
  await expect(row).toContainText("2.12.3 · pocket");
  await expect(row).toContainText("512 KB");
  await expect(row).toContainText("abababababab");
  await row.getByRole("button", { name: "Restore…" }).click();

  const sheet = page.getByRole("dialog", { name: "Restore firmware on Field radio" });
  const changes = sheet.getByRole("region", { name: "Changes" });
  await expect(changes).toContainText("2.12.4");
  await expect(changes).toContainText("2.12.3");
  await expect(changes).toContainText(COPY_ID);
  await expect(changes).toContainText(`SHA-256 ${"ab".repeat(32)}`);
  const checks = sheet.getByRole("list", { name: "Checks" });
  await expect(checks).toContainText("Copy matches its SHA-256");
  await expect(checks).toContainText("Copy is for this radio's board");
  await expect(checks).toContainText("One radio in DFU mode");
  await expect(sheet).toContainText("If a saved copy already holds it, that copy is the backup");
  await expect(sheet.getByRole("region", { name: "Warnings" })).toContainText("not proven EdgeTX 2.12.3");
  expect(await app.method("gear_firmware_restore_plan")).toEqual([{ device: RADIO_ID, copy: COPY_ID }]);
  expect(await app.calls("gear_firmware_restore_click")).toEqual([]);

  await sheet.getByRole("button", { name: "Restore" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("Verified");
  await expect(result).toContainText(COPY_ID);
  const steps = result.getByRole("list", { name: "Steps" });
  await expect(steps).toContainText("Copy the current firmware");
  await expect(steps).toContainText("before-restore");
  await expect(steps).toContainText("Leave DFU");
  expect((await app.calls("gear_firmware_restore_click")).length).toBe(1);
  expect(await app.method("gear_firmware_restore")).toEqual([]);
  await sheet.getByRole("button", { name: "Done" }).click();
  // The firmware the radio ran before is now a saved copy too.
  await expect(copies(page).getByRole("row", { name: /Before a restore/ })).toBeVisible();
});

test("a copy that fails its checks cannot be restored", async ({ app, page }) => {
  await openWithCopy(app, page);
  await app.core(`c => { c.firmware.dfu = 0; c.firmware.damaged = ["${COPY_ID}"]; }`);
  await copies(page).getByRole("button", { name: "Restore…" }).click();
  const sheet = page.getByRole("dialog", { name: "Restore firmware on Field radio" });
  const checks = sheet.getByRole("list", { name: "Checks" });
  await expect(checks).toContainText("does not match its recorded hash");
  await expect(checks).toContainText("No radio is in DFU mode.");
  await expect(sheet.getByRole("button", { name: "Restore" })).toBeDisabled();
  await sheet.getByRole("button", { name: "Cancel" }).click();

  // A copy of another board's firmware.
  await app.core(`c => { c.firmware.dfu = 1; c.firmware.damaged = []; c.firmware.copies[0].image_board = "tx16s"; }`);
  await copies(page).getByRole("button", { name: "Restore…" }).click();
  await expect(checks).toContainText("firmware for board tx16s, not pocket");
  await expect(sheet.getByRole("button", { name: "Restore" })).toBeDisabled();
  expect(await app.calls("gear_firmware_restore_click")).toEqual([]);
});

test("a bad read back is reported and the radio stays in DFU", async ({ app, page }) => {
  await openWithCopy(app, page);
  await app.core(`c => { c.firmware.failNext = true; }`);
  await copies(page).getByRole("button", { name: "Restore…" }).click();
  const sheet = page.getByRole("dialog", { name: "Restore firmware on Field radio" });
  await sheet.getByRole("button", { name: "Restore" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("The restore failed");
  await expect(result).toContainText("The radio stays in DFU mode");
  await expect(result.getByRole("listitem").filter({ hasText: "Write" }).first()).toContainText("failed");
  await expect(sheet.getByRole("button", { name: "Restore Backup" })).toHaveCount(0);
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("Saved firmware copies and the restore sheet pass axe", async ({ app, page }) => {
      await openWithCopy(app, page);
      const axe = async () => {
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      };
      await expect(copies(page)).toBeVisible();
      await axe();
      await copies(page).getByRole("button", { name: "Restore…" }).click();
      const sheet = page.getByRole("dialog", { name: "Restore firmware on Field radio" });
      await expect(sheet.getByRole("button", { name: "Restore" })).toBeEnabled();
      await axe();
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/firmware-restore-${scheme}.png`, fullPage: true });
    });
  });
}
