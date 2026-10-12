import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test } from "./fixtures";
import type { MapFrame, MapPin } from "../src/bindings";

const card = (page: Page, name: string) => page.getByRole("article", { name });
const sidebar = (page: Page) => page.getByRole("navigation", { name: "Library" });
const lastFrame = (app: { core: <T>(fn: string) => Promise<T> }) => app.core<MapFrame | null>("(c) => c.mapFrame");
const pins = (app: { core: <T>(fn: string) => Promise<T> }) => app.core<MapPin[] | null>("(c) => c.map && c.map.pins");

test("the Map page is off until Settings turns it on", async ({ app, page }) => {
  await app.open();
  await expect(card(page, "river-dive")).toBeVisible();
  await expect(sidebar(page).getByRole("button", { name: "Map" })).toHaveCount(0);
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Places" }).click();
  const box = dlg.getByRole("checkbox", { name: /Show the Map page/ });
  await expect(box).not.toBeChecked();
  await box.check();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values)).toContainEqual(expect.objectContaining({ mapView: true }));
  await expect(sidebar(page).getByRole("button", { name: "Map" })).toBeVisible();
});

test("the map shows a pin per place with its clip count, where the page keeps its spot", async ({ app, page }) => {
  await app.open("library", { settings: { mapView: true } });
  await sidebar(page).getByRole("button", { name: "Map" }).click();
  await expect(page.getByRole("heading", { name: "Map" })).toBeVisible();
  await expect(page.getByText("2 places · 3 clips on the map")).toBeVisible();
  await expect.poll(() => pins(app)).toEqual([
    expect.objectContaining({ id: "place:Home field", kind: "place", glyph: "2", lat: 40, lon: -75 }),
    expect.objectContaining({ id: "place:Riverside park", kind: "place", glyph: "1" }),
  ]);
  // The first pins zoom the map to show them all.
  expect((await app.calls("map_pins")).find((c) => (c.args.pins as unknown[]).length)?.args.fit).toBe(true);
  // The native map goes exactly where the placeholder is.
  const box = (await page.locator("[data-map-host]").boundingBox())!;
  await expect.poll(() => lastFrame(app)).toEqual(expect.objectContaining({ x: Math.round(box.x * 2) / 2, y: Math.round(box.y * 2) / 2, w: Math.round(box.width * 2) / 2 }));

  await page.getByRole("group", { name: "Map style" }).getByRole("button", { name: "Satellite" }).click();
  await expect.poll(() => app.core<string>("(c) => c.map.style")).toBe("satellite");
  expect((await app.core<{ fit: boolean }>("(c) => c.map")).fit).toBe(false);
  await page.getByRole("button", { name: "Show all" }).click();
  await expect.poll(() => app.core<boolean>("(c) => c.map.fit")).toBe(true);
});

test("the map hides under a dialog and when the page closes", async ({ app, page }) => {
  await app.open("library", { settings: { mapView: true } });
  await sidebar(page).getByRole("button", { name: "Map" }).click();
  await expect.poll(() => lastFrame(app)).not.toBeNull();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect.poll(() => lastFrame(app)).toBeNull();
  await page.keyboard.press("Escape");
  await expect.poll(() => lastFrame(app)).not.toBeNull();
  await sidebar(page).getByRole("button", { name: /^All clips/ }).click();
  await expect(card(page, "river-dive")).toBeVisible();
  await expect.poll(() => lastFrame(app)).toBeNull();
});

test("a place's pin shows its clips; a clip's pin opens the clip", async ({ app, page }) => {
  await app.open("library", { settings: { mapView: true } });
  await sidebar(page).getByRole("button", { name: "Map" }).click();
  await expect.poll(() => pins(app)).not.toBeNull();
  await app.core("(c) => c.mapPick('place:Home field')");
  await expect(card(page, "gap-run")).toBeVisible();
  await expect(card(page, "backyard-loops")).toBeVisible();
  await expect(card(page, "river-dive")).toHaveCount(0);
  await expect(sidebar(page).getByRole("button", { name: /^Home field/ })).toHaveAttribute("aria-current", "true");

  await sidebar(page).getByRole("button", { name: "Map" }).click();
  await expect(page.getByRole("heading", { name: "Map" })).toBeVisible();
  const id = await app.core<string>("(c) => c.lib.clips.find((x) => x.name === 'river-dive').id");
  await app.core(`(c) => c.mapPick('clip:${id}')`);
  await expect(page.getByRole("complementary", { name: "Inspector" })).toBeVisible();
});

test("an empty map says what is missing", async ({ app, page }) => {
  await app.open("empty", { settings: { mapView: true, places: [] } });
  await sidebar(page).getByRole("button", { name: "Map" }).click();
  await expect(page.getByText("No places or clip locations yet.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Show all" })).toBeDisabled();
});

for (const scheme of axeSchemes) {
  test(`the Map page passes axe (${scheme})`, async ({ app, page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await app.open("library", { settings: { mapView: true } });
    await sidebar(page).getByRole("button", { name: "Map" }).click();
    await expect(page.getByRole("heading", { name: "Map" })).toBeVisible();
    const r = await new AxeBuilder({ page }).include("main").analyze();
    expect(r.violations).toEqual([]);
  });
}
