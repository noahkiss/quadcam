// The Sim page's Profiles section on the mock core (`ipc/mock/simfit.ts`): a fit shows each
// group kept or dropped against the base, Save sends the fit's digest only after the confirm,
// the saved profile lists with Delete, and the Fly section's Aircraft menu lists it.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, expect, test, type AppFixture } from "./fixtures";

const profile = (name: string, model: string) => ({ name, aircraft: "", camera_make: "Generic", camera_model: "", video_system: "Analog", keywords: [], author: "", place: null, edgetx_models: [model], gear: {} });
const settings = { simPreview: true, profiles: [profile("Whoop", "WHOOP"), profile("Five-inch", "ALPHA")] };

async function openProfiles(app: AppFixture, page: Page) {
  await app.open("library", { settings });
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Sim( |$)/ }).click();
  await page.getByRole("group", { name: "Sim sections" }).getByRole("button", { name: "Profiles" }).click();
  await expect(page.getByRole("region", { name: "Fit", exact: true })).toBeVisible();
}

async function fit(page: Page) {
  const form = page.getByRole("region", { name: "Fit", exact: true });
  await form.getByLabel("Aircraft").selectOption("Whoop");
  await form.getByLabel("Starts from").selectOption("air65ii");
  await form.getByLabel("Weight with pack (g)").fill("26");
  await form.getByRole("button", { name: "Fit Profile" }).click();
  return page.getByRole("region", { name: "Result: Whoop" });
}

test("a fit shows each group kept or dropped, and Save sends the digest after the confirm", async ({ app, page }) => {
  await openProfiles(app, page);
  const fitted = page.getByRole("region", { name: "Fitted profiles" });
  await expect(fitted.getByText("No fitted profiles yet.")).toBeVisible();
  const form = page.getByRole("region", { name: "Fit", exact: true });
  await expect(form.getByRole("button", { name: "Fit Profile" })).toBeDisabled();

  const result = await fit(page);
  expect((await app.method("gear_sim_fit")).at(-1)).toEqual({ base: "air65ii", aircraft: "Whoop", mass_g: 26 });
  const groups = result.getByRole("table", { name: "Fit groups" });
  const row = (name: string) => groups.getByRole("row").filter({ has: page.getByRole("rowheader", { name }) });
  await expect(row("Motor and pack")).toContainText("Kept");
  await expect(row("Motor and pack")).toContainText("battery.resistance 0.026 → 0.031 Ω");
  await expect(row("Roll gains")).toContainText("Dropped");
  await expect(row("Roll gains")).toContainText("the fitted roll gains validate worse than air65ii's");
  await expect(result.getByRole("status", { name: "Checks" })).toHaveText("Checks passed: 5 of 6 fitted, 4 of 6 with 65 mm whoop.");

  // Cancel in the confirm saves nothing.
  await result.getByRole("button", { name: "Save…" }).click();
  let dlg = page.getByRole("dialog", { name: "Save Whoop?" });
  await dlg.getByRole("button", { name: "Cancel" }).click();
  expect(await app.method("gear_sim_fit_save")).toEqual([]);

  await result.getByRole("button", { name: "Save…" }).click();
  dlg = page.getByRole("dialog", { name: "Save Whoop?" });
  await dlg.getByRole("button", { name: "Save" }).click();
  const fit0 = (await app.method("gear_sim_fit")).at(-1);
  const save = (await app.method("gear_sim_fit_save")).at(-1) as { fit: unknown; digest: string; confirm: boolean };
  expect(save.fit).toEqual(fit0);
  expect(save.confirm).toBe(true);
  expect(save.digest).toMatch(/^[0-9a-f]{8}$/);
  await expect(fitted.getByRole("row").filter({ hasText: "Whoop" })).toContainText("26 g");
  await expect(page.getByRole("region", { name: "Result: Whoop" })).toHaveCount(0);

  // A second fit of the same aircraft says it replaces the saved profile.
  const again = await fit(page);
  await expect(again.getByRole("button", { name: "Replace…" })).toBeVisible();

  // The Fly section lists the fitted profile under Your quads.
  await page.getByRole("group", { name: "Sim sections" }).getByRole("button", { name: "Fly" }).click();
  await expect(page.getByLabel("Aircraft").locator('optgroup[label="Your quads"] option')).toHaveText(["Whoop"]);
});

test("Delete removes a fitted profile after the confirm", async ({ app, page }) => {
  await openProfiles(app, page);
  const result = await fit(page);
  await result.getByRole("button", { name: "Save…" }).click();
  await page.getByRole("dialog", { name: "Save Whoop?" }).getByRole("button", { name: "Save" }).click();
  const fitted = page.getByRole("region", { name: "Fitted profiles" });
  await fitted.getByRole("button", { name: "Actions for Whoop" }).click();
  await page.getByRole("menuitem", { name: "Delete…" }).click();
  await page.getByRole("dialog", { name: "Delete Whoop?" }).getByRole("button", { name: "Delete" }).click();
  expect((await app.method("gear_sim_profile_delete")).at(-1)).toEqual({ id: "whoop", confirm: true });
  await expect(fitted.getByText("No fitted profiles yet.")).toBeVisible();
});

test("a fit the core refuses shows its message", async ({ app, page }) => {
  await openProfiles(app, page);
  await app.core(`c => { c.settings.values.profiles = []; }`);
  const form = page.getByRole("region", { name: "Fit", exact: true });
  await form.getByLabel("Aircraft").selectOption("Five-inch");
  await form.getByRole("button", { name: "Fit Profile" }).click();
  await expect(page.getByText('No FC is linked to the aircraft "Five-inch"')).toBeVisible();
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Profiles section passes axe with a fit shown", async ({ app, page }) => {
      await openProfiles(app, page);
      await expect((await fit(page)).getByRole("table", { name: "Fit groups" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
