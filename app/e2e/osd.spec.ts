// The OSD segment on the mock core (a saved FC's page, OSD segment): open a dump, read each profile, hover
// names, the check, and axe in both themes. The mock answers with the view the real core drew
// from the synthetic PAL dump (e2e/fixtures/osd.json).
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

/** The saved FC's page, on its OSD segment. */
async function openOsd(app: AppFixture, page: Page) {
  await app.open();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "OSD" }).click();
}

const DUMP = "/Users/pilot/bench/quad.dump_all.txt";

test("opens a dump and shows each profile with its check", async ({ app, page }) => {
  await openOsd(app, page);
  // The FC's latest backup shows first.
  await expect(page.getByText(/before_apply dump all/)).toBeVisible();
  await app.core(`c => c.dialogAnswers.push([${JSON.stringify(DUMP)}])`);
  await page.getByRole("button", { name: "Open dump…" }).click();

  const calls = await app.method("gear_osd");
  expect(calls[0]).toEqual({ paths: [], device: "fc-0a1b2c3d4e5f6071", grid: null });
  expect(calls.at(-1)).toEqual({ paths: [DUMP], device: null, grid: null });
  const profiles = page.getByRole("group", { name: "OSD profile" });
  // The profile in use (osd_profile = 2) opens first; problem counts are on the segments.
  await expect(profiles.getByRole("button", { name: "2 CRUISE (1)" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("img", { name: /OSD profile 2 on a PAL grid/ })).toBeVisible();
  await expect(page.getByRole("list", { name: "Problems" })).toContainText("rssi_dbm: 2 cells off screen.");
  await expect(page.locator('[data-element="rssi_dbm"]')).toHaveAttribute("title", "RSSI (dBm)");

  await profiles.getByRole("button", { name: "1 RACE (1)" }).click();
  await expect(page.getByRole("list", { name: "Problems" })).toContainText("mah_drawn overlaps current");
  const table = page.getByRole("table", { name: "Elements in profile 1" });
  await expect(table.getByRole("row", { name: /Capacity used 1 14/ })).toBeVisible();
  await expect(page.getByText("2 problems across the profiles.")).toBeVisible();

  await profiles.getByRole("button", { name: "3", exact: true }).click();
  await expect(page.getByRole("list", { name: "Problems" })).toHaveCount(0);

  await page.getByRole("group", { name: "Grid" }).getByRole("button", { name: "HD" }).click();
  await expect.poll(async () => (await app.method("gear_osd")).at(-1)?.grid).toBe("HD");
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the OSD segment passes axe", async ({ app, page }) => {
      await openOsd(app, page);
      await app.core(`c => c.dialogAnswers.push([${JSON.stringify(DUMP)}])`);
      await page.getByRole("button", { name: "Open dump…" }).click();
      await expect(page.getByRole("img", { name: /OSD profile/ })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/osd-${scheme}.png`, fullPage: true });
    });
  });
}
