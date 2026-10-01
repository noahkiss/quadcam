import type { Page } from "@playwright/test";
import { area, dialog, expect, test, type AppFixture } from "../fixtures";

area("settings");

const settingsDialog = (page: Page) => dialog(page, "Settings");
const writes = async (app: AppFixture) => (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values);

/** The written keys whose value differs from the recorded settings. */
async function changed(app: AppFixture) {
  const before = await app.core<Record<string, unknown>>("c => c.initialValues");
  const norm = (v: unknown): unknown =>
    Array.isArray(v) ? v.map(norm) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b)).map(([k, x]) => [k, norm(x)])) : v;
  const out: Record<string, unknown> = {};
  for (const w of await writes(app)) for (const [k, v] of Object.entries(w)) if (JSON.stringify(norm(v)) !== JSON.stringify(norm(before[k]))) out[k] = v;
  return out;
}

test.beforeEach(async ({ app, page }) => {
  await app.open();
  await expect(page.getByRole("article").first()).toBeVisible();
});

test("the gear opens Settings; Done saves only what changed", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = settingsDialog(page);
  await expect(dlg).toBeVisible();
  await dlg.getByRole("button", { name: "Import" }).click();
  await dlg.getByLabel("Format").selectOption("mov");
  await dlg.getByRole("button", { name: "Photos" }).click();
  await dlg.getByLabel("Album (empty: library only)").fill("FPV");
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect(dlg).toBeHidden();
  // Legacy also rewrites unchanged places (its compare is key-order sensitive); only
  // values that differ count.
  await expect.poll(async () => changed(app)).toEqual({ format: "mov", photosAlbum: "FPV" });
});

test("Cancel and Escape save nothing", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = settingsDialog(page);
  await dlg.getByRole("button", { name: "Import" }).click();
  await dlg.getByLabel("Format").selectOption("mov");
  await dlg.getByRole("button", { name: "Cancel" }).click();
  await expect(dlg).toBeHidden();
  await page.getByRole("button", { name: "Settings" }).click();
  await page.keyboard.press("Escape");
  await expect(dlg).toBeHidden();
  expect(await changed(app)).toEqual({});
});

test("Command-comma from the menu bar opens Settings", async ({ page }) => {
  await page.evaluate(() => window.__qc!.emit("menu", "settings"));
  await expect(settingsDialog(page)).toBeVisible();
});

test("the Library pane shows the layout example", async ({ page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = settingsDialog(page);
  await dlg.getByRole("radio", { name: /Day only/ }).check();
  await expect(dlg.getByText(/└─ \d{4}-\d{2}-\d{2}\//)).toBeVisible();
  await dlg.getByRole("checkbox", { name: /Add the place to day folders/ }).check();
  await expect(dlg.getByText(/\d{4}-\d{2}-\d{2} Home field\//).first()).toBeVisible();
});

test("place search fills a new place; Done saves the list", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = settingsDialog(page);
  await dlg.getByRole("button", { name: "Places" }).click();
  await dlg.getByRole("searchbox", { name: "Search for a place" }).fill("Mill pond");
  await dlg.getByRole("searchbox", { name: "Search for a place" }).press("Enter");
  await dlg.getByRole("button", { name: /^Mill pond 1 Mill pond Road/ }).click();
  await expect(dlg.getByRole("textbox", { name: "Place name" }).last()).toHaveValue("Mill pond");
  expect((await app.method("place_search"))).toEqual([{ query: "Mill pond", provider: "apple", limit: 6 }]);
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => changed(app)).toEqual({
    places: [{ name: "Home field", lat: 40, lon: -75 }, { name: "Riverside park", lat: 40.1, lon: -75.1 }, { name: "Mill pond", lat: 40.2, lon: -75.2 }],
  });
});

test("an aircraft profile is added and made the default", async ({ app, page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = settingsDialog(page);
  await dlg.getByRole("button", { name: "Aircraft" }).click();
  await dlg.getByRole("button", { name: "Add aircraft" }).click();
  await dlg.getByLabel("Name", { exact: true }).fill("Cine");
  await dlg.getByRole("button", { name: "DJI O4" }).click();
  await dlg.getByRole("checkbox", { name: /Default aircraft/ }).check();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => Object.keys(await changed(app))).toContain("profiles");
  const w = (await changed(app)) as { profiles: { name: string; video_system: string }[]; defaultProfile: string };
  expect(w.defaultProfile).toBe("Cine");
  expect(w.profiles.map((p) => [p.name, p.video_system])).toEqual([["Whoop", "Analog"], ["Five-inch", "Analog"], ["Cine", "DJI O4"]]);
});

test("a settings change from elsewhere shows at once", async ({ app, page }) => {
  await app.core("c => c.settingsSet({ libView: 'list' })");
  await expect(page.locator("table")).toBeVisible();
});
