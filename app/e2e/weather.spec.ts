import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test } from "./fixtures";

const card = (page: Page, name: string) => page.getByRole("article", { name });
const inspector = (page: Page) => page.getByRole("complementary", { name: "Inspector" });

async function openSettings(page: Page, section: string) {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: section }).click();
  return dlg;
}

test("weather is off by default and asks nothing until turned on", async ({ app, page }) => {
  await app.open();
  await expect(card(page, "river-dive")).toBeVisible();
  const dlg = await openSettings(page, "Import");
  const box = dlg.getByRole("checkbox", { name: /Look up weather/ });
  await expect(box).not.toBeChecked();
  expect(await app.method("library_weather")).toEqual([]);
  await box.check();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values)).toContainEqual(expect.objectContaining({ weather: true }));
  await expect.poll(async () => (await app.method("library_weather")).length).toBe(1);
  await card(page, "river-dive").dblclick();
  await expect(inspector(page).getByText(/wind \d+ (mph|km\/h) from NW, gusts \d+/)).toBeVisible();
});

test("with weather on, clips with a place and a time get it once", async ({ app, page }) => {
  await app.open("library", { settings: { weather: true } });
  await expect.poll(async () => (await app.method("library_weather")).length).toBe(1);
  const ids = (await app.method("library_weather"))[0].ids as string[];
  expect(ids).toHaveLength(3);
  // Opening a clip shows it; nothing is asked again.
  await card(page, "gap-run").dblclick();
  await expect(inspector(page).getByText("Weather")).toBeVisible();
  expect(await app.method("library_weather")).toHaveLength(1);
});

test("the open clip shows the sky, cloud cover, precipitation and humidity", async ({ app, page }) => {
  await app.open("library", { settings: { weather: true } });
  await expect.poll(async () => (await app.method("library_weather")).length).toBe(1);
  await card(page, "gap-run").dblclick();
  const meta = inspector(page);
  const row = (label: string) => meta.locator("dt", { hasText: label }).locator("+ dd");
  await expect(row("Sky")).toHaveText("Partly cloudy");
  await expect(row("Cloud cover")).toHaveText("40%");
  await expect(row("Precipitation")).toHaveText("0.4 mm");
  await expect(row("Humidity")).toHaveText("71%");
  // The archive has no visibility.
  await expect(meta.locator("dt", { hasText: "Visibility" })).toHaveCount(0);
});

test("a weather record saved before the sky values is looked up once more", async ({ app, page }) => {
  await app.open("library", { settings: { weather: true }, oldWeather: true });
  await expect.poll(async () => (await app.method("library_weather")).length).toBe(1);
  expect((await app.method("library_weather"))[0].ids).toHaveLength(3);
  await card(page, "gap-run").dblclick();
  await expect(inspector(page).getByText("Partly cloudy")).toBeVisible();
  await page.getByRole("button", { name: "Library" }).click();
  await card(page, "river-dive").dblclick();
  await expect(inspector(page).getByText("Partly cloudy")).toBeVisible();
  expect(await app.method("library_weather")).toHaveLength(1);
});

test("a time near sunset tags the clip, and search finds the tag", async ({ app, page }) => {
  await app.open();
  await card(page, "backyard-loops").dblclick();
  const time = inspector(page).getByLabel("Time of day");
  await time.fill("18:45");
  await time.press("Enter");
  await expect(inspector(page).getByText("Sunset")).toBeVisible();
  await page.getByRole("button", { name: "Library" }).click();
  await expect(card(page, "backyard-loops").getByRole("img", { name: "Sunset" })).toBeVisible();
  await page.getByRole("searchbox", { name: "Search the library" }).fill("sunset");
  await expect(card(page, "backyard-loops")).toBeVisible();
  await expect(card(page, "river-dive")).toHaveCount(0);
});

test("Use current location adds a named place, and a row can be named from its coordinates", async ({ app, page }) => {
  await app.open();
  const dlg = await openSettings(page, "Places");
  await dlg.getByRole("button", { name: "Use Current Location" }).click();
  await expect(dlg.getByRole("textbox", { name: "Place name" }).last()).toHaveValue("Current spot");
  await expect(dlg.getByRole("spinbutton", { name: "Latitude" }).last()).toHaveValue("40.21");
  expect(await app.method("place_here")).toHaveLength(1);

  await dlg.getByRole("button", { name: "Add Place" }).click();
  await dlg.getByRole("spinbutton", { name: "Latitude" }).last().fill("40.5");
  await dlg.getByRole("spinbutton", { name: "Longitude" }).last().fill("-75.5");
  await dlg.getByRole("button", { name: "Name from coordinates" }).last().click();
  await expect(dlg.getByRole("textbox", { name: "Place name" }).last()).toHaveValue("Springfield Park");
  expect((await app.method("place_reverse"))[0]).toMatchObject({ lat: 40.5, lon: -75.5 });
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("sun and weather in the open clip pass axe", async ({ app, page }) => {
      await app.open("library", { settings: { weather: true } });
      await card(page, "backyard-loops").dblclick();
      const time = inspector(page).getByLabel("Time of day");
      await time.fill("18:00");
      await time.press("Enter");
      await expect(inspector(page).getByText("Golden hour")).toBeVisible();
      await expect(inspector(page).getByText(/wind \d+/)).toBeVisible();
      await expect(inspector(page).getByText("Partly cloudy")).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
