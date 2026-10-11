// A radio's "Aircraft on this radio" on the mock core: the profiles that name the radio, each
// with its model on the card, and Add and Remove saving the profile's `gear.radio`.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const profile = (name: string, model: string, gear: Record<string, string>) => ({ name, aircraft: "", camera_make: "Generic", camera_model: "", video_system: "Analog", keywords: [], author: "", place: null, edgetx_models: [model], gear });
const settings = { profiles: [profile("Whoop", "WHOOP", { radio: RADIO_ID, edgetx_model: "model01.yml" }), profile("Five-inch", "ALPHA", {})] };
const bar = (page: Page) => page.getByRole("contentinfo", { name: "Gear status" });

async function radioPage(page: Page) {
  await bar(page).getByRole("button", { name: "Radio: Connected" }).click();
  return page.getByRole("region", { name: "Field radio" });
}

test("a radio lists its aircraft from the profiles, and Add and Remove save the profile", async ({ app, page }) => {
  await app.open("gear", { settings });
  const radio = await radioPage(page);
  const section = radio.getByRole("region", { name: "Aircraft on this radio" });
  const rows = section.getByRole("table").getByRole("row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1)).toContainText("Whoop");
  await expect(rows.nth(1)).toContainText("Selected");
  await expect(rows.nth(1)).toContainText("BRAVO 2 · model01.yml");
  await expect(rows.nth(1)).toContainText("On the card");
  // A radio has no single aircraft link.
  await expect(radio.getByRole("term").filter({ hasText: /^Aircraft$/ })).toHaveCount(0);

  await section.getByLabel("Aircraft to add").selectOption("Five-inch");
  await section.getByRole("button", { name: "Add aircraft" }).click();
  await expect.poll(async () => (await app.method("profile_save")).at(-1)).toEqual({ name: "Five-inch", fields: { gear: { radio: RADIO_ID } }, new_name: null });
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(2)).toContainText("ALPHA · model00.yml");
  await expect(section.getByLabel("Aircraft to add")).toHaveCount(0);

  await section.getByRole("button", { name: "Remove Whoop" }).click();
  await expect.poll(async () => (await app.method("profile_save")).at(-1)).toEqual({ name: "Whoop", fields: { gear: { edgetx_model: "model01.yml" } }, new_name: null });
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1)).toContainText("Five-inch");
});

test("Edit on a radio saves the name only; an FC keeps its one aircraft", async ({ app, page }) => {
  await app.open("gear", { settings });
  const radio = await radioPage(page);
  await radio.getByRole("button", { name: "Edit…" }).click();
  await expect(page.getByRole("dialog").getByLabel("Aircraft")).toHaveCount(0);
  await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await app.method("gear_device_save")).at(-1)).toEqual({ id: RADIO_ID, name: "Field radio", aircraft: null });

  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  const fc = page.getByRole("region", { name: "Whoop FC" });
  await expect(fc.getByRole("definition").filter({ hasText: /^Whoop/ })).toBeVisible();
  await expect(fc.getByRole("region", { name: "Aircraft on this radio" })).toHaveCount(0);
  await fc.getByRole("button", { name: "Edit…" }).click();
  await expect(page.getByRole("dialog").getByLabel("Aircraft")).toHaveValue("Whoop");
});

test("Settings > Aircraft edits the same link the radio page lists", async ({ app, page }) => {
  await app.open("gear", { settings });
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = page.getByRole("dialog", { name: "Settings" });
  await dlg.getByRole("button", { name: "Aircraft" }).click();
  await expect(dlg.getByLabel("Radio", { exact: true })).toHaveValue(RADIO_ID);
  await expect(dlg.getByLabel("EdgeTX model file")).toHaveValue("model01.yml");
  await dlg.getByLabel("Radio", { exact: true }).selectOption("");
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => ((await app.method("settings_set")).at(-1)?.values as { profiles?: { gear: unknown }[] })?.profiles?.[0]?.gear).toEqual({ edgetx_model: "model01.yml" });
  const radio = await radioPage(page);
  await expect(radio.getByRole("region", { name: "Aircraft on this radio" })).toContainText("No aircraft yet.");
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("Aircraft on this radio passes axe", async ({ app, page }) => {
      await app.open("gear", { settings });
      const radio = await radioPage(page);
      await expect(radio.getByRole("region", { name: "Aircraft on this radio" }).getByRole("row")).toHaveCount(2);
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
