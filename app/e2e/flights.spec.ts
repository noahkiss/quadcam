// Flights, packs, Pack up, Repairs and the clip's crash log on the mock core. The flights
// are the real core's answer on the synthetic log (e2e/fixtures/flights.json).
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { dialog, expect, test, type AppFixture } from "./fixtures";

const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

async function openPage(app: AppFixture, page: Page, name: string) {
  await app.open();
  await side(page).getByRole("button", { name: new RegExp(`^${name}`) }).click();
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}

test("flights of a day with their measures; a pack is set from the row", async ({ app, page }) => {
  await openPage(app, page, "Flights");
  const table = page.getByRole("table", { name: "Flights on 2026-10-04" });
  await expect(table.getByRole("row")).toHaveCount(4);
  const first = table.getByRole("row", { name: /10:00/ });
  await expect(first).toContainText("1:00");
  await expect(first).toContainText("43 %");
  await expect(first).toContainText("3.50 / 3.30 V");
  await expect(first).toContainText("3.95 V");
  await expect(first).toContainText("360 mAh");
  await expect(first.getByLabel("Pack for 10:00")).toHaveValue("A1");
  await expect(table.getByLabel("Pack for 10:03").locator("option").first()).toHaveText("None (next: A2)");

  await table.getByLabel("Pack for 10:03").selectOption("A2");
  await expect.poll(() => app.method("gear_flight_set")).toEqual([{ flight: "20261004T100300-whoop", pack: "A2", place: null }]);
  await expect(table.getByLabel("Pack for 10:03")).toHaveValue("A2");

  await first.getByRole("button", { name: "10:00" }).click();
  const detail = page.getByRole("region", { name: "Flight at 10:00" });
  await expect(detail).toContainText("first third 40 %");
  await expect(detail).toContainText("after 0:50");
  await expect(detail.getByRole("table", { name: "Dropouts" })).toContainText("Telemetry only");
});

test("the session report reads the day and copies as Markdown", async ({ app, page }) => {
  await openPage(app, page, "Flights");
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Session report" }).click();
  const r = page.getByRole("region", { name: "Session report" });
  await expect(r).toContainText("Air time");
  await expect(r).toContainText("1:50");
  await expect(r).toContainText("LQ 60 %, RSSI -95 dB at 10:00");
  await expect(r.getByRole("table", { name: "Pack use" })).toContainText("A1");
  await page.getByRole("button", { name: "Copy as Markdown" }).click();
  await expect(page.getByRole("status")).toContainText("report");
});

test("the import's Finish step opens the session report", async ({ app, page }) => {
  await app.open("finished-card");
  await page.getByRole("button", { name: "Import…" }).first().click();
  await page.getByRole("button", { name: "Show report" }).click();
  await expect(page.getByRole("heading", { name: "Flights", exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Session report" })).toContainText("Air time");
  expect((await app.method("gear_session_report")).every((p) => p.day === null)).toBe(true);
});

test("packs: history, mark charged, and Pack up passes once all are charged", async ({ app, page }) => {
  await openPage(app, page, "Packs");
  const packs = page.getByRole("table", { name: "Packs" });
  await expect(packs.getByRole("row", { name: /A1/ })).toContainText("Flown");
  await packs.getByRole("button", { name: "A1" }).click();
  await expect(page.getByRole("table", { name: "Flights on A1" }).getByRole("row")).toHaveCount(2);

  for (const label of ["A1", "A2", "A3"]) await packs.getByRole("row", { name: new RegExp(label) }).getByRole("button", { name: "Mark charged" }).click();
  await expect(packs.getByRole("button", { name: "Mark charged" })).toHaveCount(0);
  expect((await app.method("gear_pack_save")).map((p) => p.charged)).toEqual([true, true, true]);

  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Charging" }).click();
  await expect(page.getByRole("table", { name: "Pack types" })).toContainText("4.35 V");

  await side(page).getByRole("button", { name: /^Pack up/ }).click();
  const checks = page.getByRole("list", { name: "Checks" });
  await expect(checks).toContainText("3 of 3 charged.");
  await expect(checks).toContainText("No card in the Mac.");
});

test("a pack is added in a sheet", async ({ app, page }) => {
  await openPage(app, page, "Packs");
  await page.getByRole("button", { name: "Add pack…" }).click();
  const sheet = dialog(page, "Add pack");
  await sheet.getByLabel("Label").fill("B1");
  await sheet.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("table", { name: "Packs" }).getByRole("button", { name: "B1" })).toBeVisible();
});

test("a crash logged on a clip shows there and under Repairs", async ({ app, page }) => {
  await app.open();
  await page.getByRole("article", { name: "gap-run" }).dblclick();
  await page.getByRole("tab", { name: "Flight" }).click();
  const crashes = page.getByRole("region", { name: "Crashes" });
  await expect(crashes).toContainText("None logged.");
  await crashes.getByRole("button", { name: "Log crash…" }).click();
  const sheet = dialog(page, "Log crash");
  await sheet.getByLabel("Time in clip").fill("1:05");
  await sheet.getByLabel("What broke").fill("front left arm");
  await sheet.getByLabel("Parts used").fill("arm, prop");
  await sheet.getByRole("button", { name: "Save" }).click();
  const [saved] = await app.method("gear_crash_save");
  expect(saved).toMatchObject({ time_s: 65, broke: "front left arm", parts: ["arm", "prop"] });
  await expect(crashes).toContainText("front left arm");

  await page.getByRole("button", { name: "Library" }).click();
  await side(page).getByRole("button", { name: /^Repairs/ }).click();
  await expect(page.getByRole("table", { name: "Crashes" })).toContainText("arm, prop");
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    for (const name of ["Flights", "Packs", "Pack up"]) {
      test(`${name} passes axe`, async ({ app, page }) => {
        await openPage(app, page, name);
        if (name === "Flights") await page.getByRole("button", { name: "10:00" }).click();
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      });
    }
  });
}
