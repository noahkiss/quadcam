// The Rates segment on the mock core (a saved FC's page, Rates segment): profiles with names,
// curves and the table, the throttle curve, compare with a profile, another quad and a sim's
// profile, the sims list, and axe in both themes. The mock answers with what the real core read
// from a synthetic three-profile dump and from synthetic sim files (e2e/fixtures/rates.json,
// sims.json).
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

/** The saved FC's page, on its Rates segment. */
async function openRates(app: AppFixture, page: Page) {
  await app.open();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Rates" }).click();
}

const DUMP = "/Users/pilot/bench/quad.dump_all.txt";

test("shows each profile with its curves, table and throttle", async ({ app, page }) => {
  await openRates(app, page);
  await expect(page.getByText(/before_apply dump all/)).toBeVisible();
  const calls = await app.method("gear_rates");
  expect(calls[0]).toEqual({ paths: [], device: "fc-0a1b2c3d4e5f6071" });

  const profiles = page.getByRole("group", { name: "Rate profile" });
  await expect(profiles.getByRole("button", { name: "0 FREE" })).toHaveAttribute("aria-pressed", "true");
  await expect(profiles.getByRole("button", { name: "1 RACE" })).toBeVisible();
  await expect(profiles.getByRole("button", { name: "2 CINE" })).toBeVisible();
  await expect(page.getByText("In use")).toBeVisible();
  await expect(page.getByText("Betaflight rates", { exact: true })).toBeVisible();

  const table = page.getByRole("table", { name: "Rates of 0 FREE" });
  await expect(table.getByRole("row", { name: /Roll 127 72 40 1998 152°\/s 907°\/s/ })).toBeVisible();
  await expect(page.getByRole("img", { name: /Roll: 152°\/s per full stick at the centre, 907°\/s at full stick/ })).toBeVisible();
  await expect(page.getByRole("img", { name: /Throttle: mid 30, expo 50, hover 34, limit Off 100%/ })).toBeVisible();

  await profiles.getByRole("button", { name: "1 RACE" }).click();
  await expect(page.getByText("Actual rates", { exact: true })).toBeVisible();
  const race = page.getByRole("table", { name: "Rates of 1 RACE" });
  await expect(race.getByRole("row", { name: /Roll 7 67 54 1998 70°\/s 670°\/s/ })).toBeVisible();
  await expect(page.getByText("In use")).toHaveCount(0);

  await profiles.getByRole("button", { name: "2 CINE" }).click();
  await expect(page.getByText("Quick rates", { exact: true })).toBeVisible();
  await expect(page.getByRole("img", { name: /Throttle: .* limit Clip 80%/ })).toBeVisible();
  await expect(page.getByText("Throttle: mid 50, expo 0, limit Clip 80%")).toBeVisible();
});

test("lists the sims and says which match the quad", async ({ app, page }) => {
  await openRates(app, page);
  const sims = page.getByRole("region", { name: "Sims" });
  const row = (name: string) => sims.getByRole("listitem").filter({ has: page.getByText(name, { exact: true }) }).first();
  // FREE (profile 0) is the profile in use; Liftoff's Freestyle and The Zone's profile 0 equal it.
  await expect(row("Liftoff")).toContainText("Matches the quad");
  await expect(row("Liftoff")).toContainText("Running");
  await expect(row("Liftoff")).toContainText("Freestyle maximum 907 / 907 / 800 °/s, same as the quad");
  await expect(row("Liftoff")).toContainText("Race maximum 667 / 667 / 667 °/s, differs by up to");
  await expect(row("Liftoff: Micro Drones")).toContainText("Differs from the quad");
  await expect(row("Uncrashed")).toContainText("throttle differs");
  await expect(row("Uncrashed")).toContainText("empty profile");
  await expect(row("The Zone")).toContainText("Matches the quad");
  await expect(row("The Zone")).toContainText("uses the actual type");
  await expect(row("Velocidrone")).toContainText("Off");
  await expect(row("Velocidrone")).toContainText("Off until a sample save exists");
  await expect((await app.method("gear_sims")).at(-1)).toEqual({ paths: [], device: "fc-0a1b2c3d4e5f6071", profile: 0 });

  // Against the Actual profile the sims differ, and the fit error is shown.
  await page.getByRole("group", { name: "Rate profile" }).getByRole("button", { name: "1 RACE" }).click();
  await expect(row("Liftoff")).toContainText("Differs from the quad");
  await expect(row("The Zone")).toContainText("Differs from the quad");
  expect((await app.method("gear_sims")).at(-1)).toEqual({ paths: [], device: "fc-0a1b2c3d4e5f6071", profile: 1 });
});

test("compares with another profile, another quad and a sim's profile", async ({ app, page }) => {
  await openRates(app, page);
  const compare = page.getByLabel("Compare with");
  const table = page.getByRole("table", { name: "Rates of 0 FREE" });
  await expect(table.getByRole("columnheader", { name: /Maximum, / })).toHaveCount(0);

  await compare.selectOption({ label: "1 RACE" });
  await expect(table.getByRole("columnheader", { name: "Maximum, 1 RACE" })).toBeVisible();
  await expect(table.getByRole("row", { name: /^Roll .* 907°\/s 670°\/s/ })).toBeVisible();
  await expect(page.getByRole("img", { name: /Roll: .* 1 RACE: 670°\/s at full stick/ })).toBeVisible();
  await expect(page.getByRole("img", { name: /Throttle: .* 1 RACE is dashed/ })).toBeVisible();

  await compare.selectOption({ label: "Liftoff · Race" });
  await expect(table.getByRole("columnheader", { name: "Maximum, Liftoff · Race" })).toBeVisible();
  await expect(table.getByRole("row", { name: /^Roll .* 907°\/s 667°\/s/ })).toBeVisible();

  // A second quad with its own backup: its profile in use is on offer.
  await app.core(`c => {
    c.gear.devices.push({ id: "fc-9999aaaabbbbcccc", kind: "fc", name: "Five inch", aircraft: null, identity: {}, last_seen: "2026-10-05T10:00:00Z", last_backup: "fc-9999aaaabbbbcccc/2026-10-05T100000-manual" });
    c.emit("gear-changed");
  }`);
  await expect(compare.locator("option", { hasText: "Five inch" })).toHaveCount(1);
  await compare.selectOption("quad:fc-9999aaaabbbbcccc");
  await expect(table.getByRole("columnheader", { name: "Maximum, Five inch · 1 RACE" })).toBeVisible();
  expect((await app.method("gear_rates")).at(-1)).toEqual({ paths: [], device: "fc-9999aaaabbbbcccc" });
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Rates segment passes axe", async ({ app, page }) => {
      await openRates(app, page);
      await page.getByLabel("Compare with").selectOption({ label: "1 RACE" });
      await expect(page.getByRole("img", { name: /Roll:/ })).toBeVisible();
      await expect(page.getByRole("region", { name: "Sims" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/rates-${scheme}.png`, fullPage: true });
    });
  });
}

test("a dump file replaces the backup", async ({ app, page }) => {
  await openRates(app, page);
  await app.core(`c => c.dialogAnswers.push([${JSON.stringify(DUMP)}])`);
  await page.getByRole("button", { name: "Open dump…" }).click();
  await expect.poll(async () => (await app.method("gear_rates")).at(-1)).toEqual({ paths: [DUMP], device: null });
  await expect(page.getByText("quad.dump_all.txt")).toBeVisible();
  expect((await app.method("gear_sims")).at(-1)).toEqual({ paths: [DUMP], device: null, profile: 0 });
});
