// The layout pass on each Gear page, on the mock core: summary lines in the headers, one
// visible action per row with the rest in its "…" menu, rows that need action first, and the
// folded sections.
import type { Page } from "@playwright/test";
import { dialog, expect, test } from "./fixtures";

const nav = (page: Page) => page.getByRole("navigation", { name: "Library" });
const go = (page: Page, name: string) => nav(page).getByRole("button", { name: new RegExp(`^${name}`) }).first().click();

test("Connected sums up in its header and puts card actions in each card's menu", async ({ app, page }) => {
  await app.open("gear");
  await go(page, "Connected");
  const header = page.locator("[data-component=page-header]");
  await expect(header).toContainText("3 plugged in");
  await expect(page.getByRole("group", { name: "Summary" })).toHaveCount(0);
  const cards = page.getByRole("list", { name: "Connected devices" }).getByRole("article");
  // A card that needs the person comes first.
  await expect(cards.first()).toHaveAttribute("aria-label", "DVR card");
  await page.getByRole("button", { name: "More actions for Field radio" }).click();
  await expect(page.getByRole("menuitem", { name: "Open" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Forget…" })).toBeVisible();
  await page.getByRole("menuitem", { name: "Open" }).click();
  await expect(page.getByRole("region", { name: "Field radio" })).toBeVisible();
});

test("Devices groups rows by kind, and a row's menu backs up, edits and forgets", async ({ app, page }) => {
  await app.open("gear");
  await go(page, "Devices");
  const table = page.getByRole("table", { name: "Devices" });
  await expect(table.getByRole("columnheader", { name: "Radios" })).toBeVisible();
  await expect(table.getByRole("columnheader", { name: "Quads" })).toBeVisible();
  await expect(table.getByRole("columnheader", { name: "Kind" })).toHaveCount(0);
  await expect(table.getByRole("row", { name: /Whoop FC/ })).toContainText("Not connected");

  await table.getByRole("button", { name: "More actions for Field radio" }).click();
  await page.getByRole("menuitem", { name: "Back up now" }).click();
  await expect(page.getByText("No changes since the last backup.")).toBeVisible();

  await table.getByRole("button", { name: "More actions for Whoop FC" }).click();
  await expect(page.getByRole("menuitem", { name: "Back up now" })).toBeDisabled();
  await page.getByRole("menuitem", { name: "Forget…" }).click();
  await dialog(page, "Forget Whoop FC?").getByRole("button", { name: "Forget" }).click();
  await expect(table.getByRole("row", { name: /Whoop FC/ })).toHaveCount(0);
});

test("a narrow Devices table stacks each row and keeps the row menu", async ({ app, page }) => {
  await page.setViewportSize({ width: 420, height: 800 });
  await app.open("gear");
  await page.getByRole("button", { name: "Show sidebar" }).click();
  await go(page, "Devices");
  const table = page.getByRole("table", { name: "Devices" });
  await expect(table.getByRole("columnheader", { name: "Name" })).toBeHidden();
  await expect(table.getByRole("button", { name: "More actions for Whoop FC" })).toBeVisible();
  await expect(table.getByRole("cell", { name: /Aircraft/ }).first()).toBeHidden();
});

const RADIO = `{ id: "radio-1f2e3d4c5b6a7980", kind: "radio", link: { kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }, identity: { board: "tx16s" } }`;
const stage = (title: string) =>
  `c => c.dispatch("gear_change_stage", { device: "radio-1f2e3d4c5b6a7980", title: "${title}", edits: [{ kind: "radio", ops: [{ op: "set_scalar", key: "contrast", value: "20" }] }], note: null, editor: null, draft: false }).id`;

test("the Bench sums up in its header; a row keeps Review and puts Discard in its menu", async ({ app, page }) => {
  await app.open();
  await app.core(`c => c.plug([${RADIO}])`);
  await app.core(stage("Raise contrast"));
  await app.core(stage("Lower contrast"));
  await go(page, "Bench");
  await expect(page.locator("[data-component=page-header]")).toContainText("2 staged");
  await expect(page.locator("[data-component=page-header]")).toContainText("1 of 1 device plugged in");
  const group = page.getByRole("region", { name: "Field radio" });
  await expect(group.getByRole("button", { name: "Discard" })).toHaveCount(0);
  await group.getByRole("button", { name: "More actions for Lower contrast" }).click();
  await expect(page.getByRole("menuitem", { name: /^By you/ })).toBeDisabled();
  await page.getByRole("menuitem", { name: "Discard" }).click();
  await expect(group.getByRole("table", { name: "Staged changes for Field radio" }).getByRole("row", { name: /Lower contrast/ })).toHaveCount(0);
  // Copy as Markdown sits in the header's … menu.
  await page.getByRole("button", { name: "Bench actions" }).click();
  await expect(page.getByRole("menuitem", { name: "Copy as Markdown" })).toBeVisible();
  await page.keyboard.press("Escape");

  // Narrow: Copy settings… joins the menu, and each change stacks.
  await page.setViewportSize({ width: 420, height: 900 });
  await expect(page.getByRole("button", { name: "Copy settings…" })).toBeHidden();
  await page.getByRole("button", { name: "Bench actions" }).click();
  await expect(page.getByRole("menuitem", { name: "Copy settings…" })).toBeVisible();
  await page.keyboard.press("Escape");
  if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/pages-bench-420.png` });
});

test("a flight opens in a drawer beside the table, and as a full-width sheet in a narrow window", async ({ app, page }) => {
  await app.open();
  await go(page, "Flights");
  const table = page.getByRole("table", { name: "Flights on 2026-10-04" });
  await table.getByRole("button", { name: "10:00" }).click();
  const drawer = page.getByRole("complementary", { name: "Flight at 10:00" });
  await expect(drawer).toBeVisible();
  await expect(table).toBeVisible();
  const [t, d] = [await table.locator("xpath=..").boundingBox(), await drawer.boundingBox()];
  expect(d!.x).toBeGreaterThan(t!.x + t!.width - 1);
  if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/pages-flight-drawer-1280.png` });
  await page.keyboard.press("Escape");
  await drawer.getByRole("button", { name: "Close" }).click();
  await expect(drawer).toHaveCount(0);

  await page.setViewportSize({ width: 420, height: 900 });
  await table.getByRole("button", { name: "10:00" }).click();
  const box = (await drawer.boundingBox())!;
  expect(Math.round(box.width)).toBe(420);
  await expect(drawer.getByRole("button", { name: "Close" })).toBeFocused();
  if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/pages-flight-sheet-420.png` });
  await page.keyboard.press("Escape");
  await expect(drawer).toHaveCount(0);
});

test("a pack retires from its menu and folds under the retired disclosure", async ({ app, page }) => {
  await app.open();
  await go(page, "Packs");
  await page.getByRole("table", { name: "Packs" }).getByRole("button", { name: "More actions for A3" }).click();
  await page.getByRole("menuitem", { name: "Retire" }).click();
  expect((await app.method("gear_pack_save")).at(-1)).toMatchObject({ pack: { label: "A3", retired: true } });
  const retired = page.getByRole("region", { name: "Retired packs" });
  await retired.getByText("1 retired").click();
  await expect(retired.getByRole("table", { name: "Retired packs" })).toContainText("A3");
});

test("Repairs puts open crashes first with Mark repaired and folds the repaired ones", async ({ app, page }) => {
  await app.open();
  await app.core(
    `c => { c.flights.crashes = [{ id: "crash-1", clip: null, time_s: 65, aircraft: "Whoop", day: "2026-10-04", broke: "arm", parts: ["frame"], note: "", repaired: false }, { id: "crash-2", clip: null, time_s: 10, aircraft: "Whoop", day: "2026-10-01", broke: "prop", parts: [], note: "", repaired: true }]; c.emit("gear-changed"); }`,
  );
  await go(page, "Repairs");
  await expect(page.locator("[data-component=page-header]")).toContainText("1 open");
  const card = page.getByRole("region", { name: "Whoop" });
  await expect(card.getByRole("table", { name: "Crashes on Whoop" })).toContainText("arm");
  await expect(card.getByRole("table", { name: "Crashes on Whoop" })).not.toContainText("prop");
  await card.getByText("1 repaired").click();
  await expect(card.getByRole("table", { name: "Repaired crashes on Whoop" })).toContainText("prop");
  await card.getByRole("button", { name: "Mark repaired" }).click();
  expect((await app.method("gear_crash_save")).at(-1)).toMatchObject({ id: "crash-1", repaired: true });
});

test("the Map page lists the places beside the map; Add place… searches and saves, a place's menu renames it", async ({ app, page }) => {
  await app.open("library", { settings: { mapView: true } });
  await nav(page).getByRole("button", { name: "Map" }).click();
  const list = page.getByRole("complementary", { name: "Places" });
  await expect(list).toContainText("Home field");
  await expect(list).toContainText("2 clips");
  if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/pages-map-1280.png` });
  await list.getByRole("button", { name: "Add place…" }).click();
  const pop = page.getByRole("dialog", { name: "Add place" });
  await pop.getByLabel("Search for a place").fill("Golden Gate Park");
  await pop.getByLabel("Search for a place").press("Enter");
  await pop.getByRole("list", { name: "Search results" }).getByRole("button").first().click();
  await expect.poll(async () => (await app.method("place_save")).length).toBe(1);

  await list.getByRole("button", { name: "More actions for Home field" }).click();
  await page.getByRole("menuitem", { name: "Rename…" }).click();
  const d = dialog(page, "Rename Home field");
  await d.getByRole("textbox").fill("Home");
  await d.getByRole("button", { name: "Rename" }).click();
  await expect(list).toContainText("Home");
});

test("Settings fills a narrow window and picks its section from a pop-up", async ({ app, page }) => {
  await page.setViewportSize({ width: 420, height: 800 });
  await app.open();
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await expect(dlg.getByRole("navigation", { name: "Settings sections" })).toBeHidden();
  await dlg.getByLabel("Section", { exact: true }).selectOption("gear");
  await expect(dlg.getByRole("button", { name: /^FC: / })).toBeVisible();
  const box = (await dlg.boundingBox())!;
  expect(Math.round(box.width)).toBe(420);
  // A wide sheet sets the Gear pane's groups in two columns.
  await page.setViewportSize({ width: 1440, height: 900 });
  if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/pages-settings-gear-1440.png` });
});
