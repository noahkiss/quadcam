import { area, clipCell, clipTable, dialog, expect, selectedClips, test } from "../fixtures";

area("library");

const card = (page: import("@playwright/test").Page, name: string) => page.getByRole("article", { name });

test.beforeEach(async ({ app }) => {
  await app.open();
  await expect(card(app.page, "gap-run")).toBeVisible();
});

test("groups clips by flying day, newest first", async ({ page }) => {
  await expect(page.getByRole("article")).toHaveCount(3);
  const days = page.getByRole("heading", { level: 2, name: /2026$/ });
  await expect(days).toHaveText([/Sep(tember)? 28, 2026/, /Sep(tember)? 27, 2026/]);
  await expect(page.getByText("Home field · Whoop · 2 clips · 2 min flying")).toBeVisible();
});

test("click selects; Command-click and Shift-click extend; Escape clears", async ({ page }) => {
  await card(page, "backyard-loops").click();
  await expect(clipCell(page, "backyard-loops")).toHaveAttribute("aria-selected", "true");
  await card(page, "river-dive").click({ modifiers: ["Meta"] });
  const bar = page.getByRole("toolbar", { name: "Selected clips" });
  await expect(bar).toContainText("2 selected");
  await card(page, "gap-run").click({ modifiers: ["Shift"] });
  await expect(selectedClips(page)).toHaveCount(3);
  await page.keyboard.press("Escape");
  await expect(selectedClips(page)).toHaveCount(0);
  await expect(bar).toBeHidden();
});

test("arrow keys move the selection", async ({ page }) => {
  await card(page, "river-dive").click();
  await page.keyboard.press("ArrowRight");
  await expect(clipCell(page, "backyard-loops")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowRight");
  await expect(clipCell(page, "gap-run")).toHaveAttribute("aria-selected", "true");
});

test("number keys rate, P X U flag, and Command-Z undoes", async ({ app, page }) => {
  await card(page, "backyard-loops").click();
  await page.keyboard.press("5");
  await expect(card(page, "backyard-loops").getByRole("img", { name: "5 of 5 stars" })).toBeVisible();
  expect((await app.method("library_rate")).at(-1)!).toMatchObject({ rating: 5 });
  await page.keyboard.press("p");
  await expect(card(page, "backyard-loops").getByTitle("Pick")).toBeVisible();
  await page.keyboard.press("x");
  await expect(card(page, "backyard-loops").getByTitle("Rejected")).toBeVisible();
  await page.keyboard.press("u");
  await expect(card(page, "backyard-loops").getByTitle("Rejected")).toHaveCount(0);
  await page.keyboard.press("Meta+z");
  await expect(card(page, "backyard-loops").getByTitle("Rejected")).toBeVisible();
  await page.keyboard.press("Meta+z");
  await page.keyboard.press("Meta+z");
  await page.keyboard.press("Meta+z");
  await expect(card(page, "backyard-loops").getByRole("img", { name: "2 of 5 stars" })).toBeVisible();
  await page.keyboard.press("Shift+Meta+z");
  await expect(card(page, "backyard-loops").getByRole("img", { name: "5 of 5 stars" })).toBeVisible();
});

test("the stars on a card rate it; the same star again clears it", async ({ app, page }) => {
  await card(page, "gap-run").getByRole("button", { name: "2 stars" }).click();
  await expect(card(page, "gap-run").getByRole("img", { name: "2 of 5 stars" })).toBeVisible();
  await card(page, "gap-run").getByRole("button", { name: "2 stars" }).click();
  expect((await app.method("library_rate")).at(-1)!).toMatchObject({ rating: 0 });
});

test("Return renames the selected clip; Escape cancels", async ({ app, page }) => {
  await card(page, "backyard-loops").click();
  await page.keyboard.press("Enter");
  const field = page.getByRole("textbox", { name: "Clip name" });
  await expect(field).toBeFocused();
  await field.fill("garden loops");
  await page.keyboard.press("Enter");
  await expect(card(page, "garden loops")).toBeVisible();
  expect((await app.method("library_rename"))[0]).toEqual({ id: "x07fee4b870d01a6f", name: "garden loops" });
  await card(page, "garden loops").click();
  await page.keyboard.press("Enter");
  await page.getByRole("textbox", { name: "Clip name" }).fill("nope");
  await page.keyboard.press("Escape");
  await expect(card(page, "garden loops")).toBeVisible();
  expect(await app.method("library_rename")).toHaveLength(1);
  await page.keyboard.press("Meta+z");
  await expect(card(page, "backyard-loops")).toBeVisible();
});

test("Command-Delete moves to the Trash after asking; undo puts it back", async ({ app, page }) => {
  await card(page, "river-dive").click();
  await page.keyboard.press("Meta+Backspace");
  const dlg = dialog(page, "Move river-dive to the Trash?");
  await expect(dlg).toBeVisible();
  await dlg.getByRole("button", { name: "Move to Trash" }).click();
  await expect(card(page, "river-dive")).toHaveCount(0);
  await expect(page.getByRole("status")).toContainText("moved to the Trash");
  await page.keyboard.press("Meta+z");
  await expect(card(page, "river-dive")).toBeVisible();
  expect(await app.method("library_untrash")).toHaveLength(1);
});

test("search matches name, note, place and aircraft", async ({ page }) => {
  const search = page.getByRole("searchbox", { name: "Search the library" });
  await search.fill("river");
  await expect(page.getByRole("article")).toHaveCount(1);
  await search.fill("two packs");
  await expect(page.getByRole("article")).toHaveCount(1);
  await expect(card(page, "gap-run")).toBeVisible();
  await search.fill("whoop");
  await expect(page.getByRole("article")).toHaveCount(2);
  await search.fill("nothing like this");
  await expect(page.getByText("No clips match.")).toBeVisible();
});

test("sidebar groups filter the library", async ({ page }) => {
  const side = page.getByRole("navigation", { name: "Library" });
  await side.getByRole("button", { name: /Picks/ }).click();
  await expect(page.getByRole("article")).toHaveCount(1);
  await side.getByRole("button", { name: /Rejected/ }).click();
  await expect(page.getByRole("article")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Move 1 to Trash" })).toBeVisible();
  await side.getByRole("button", { name: /Riverside park/ }).click();
  await expect(card(page, "river-dive")).toBeVisible();
  await side.getByRole("button", { name: /Five-inch/ }).click();
  await expect(page.getByRole("article")).toHaveCount(1);
  await side.getByRole("button", { name: /All clips/ }).click();
  await expect(page.getByRole("article")).toHaveCount(3);
});

test("sort and view settings save per machine", async ({ app, page }) => {
  await page.getByRole("combobox", { name: "Sort by" }).selectOption("name");
  await expect(page.getByRole("article")).toHaveCount(3);
  await expect(page.getByRole("article").first()).toHaveAccessibleName("backyard-loops");
  await page.getByRole("button", { name: "List", exact: true }).click();
  await expect(clipTable(page)).toBeVisible();
  await expect(clipTable(page).locator("tbody tr[data-id]")).toHaveCount(3);
  const sets = (await app.method("settings_set")).map((p) => (p as { values: object }).values);
  expect(sets).toContainEqual({ libSort: { key: "name", dir: "asc" } });
  expect(sets).toContainEqual({ libView: "list" });
});

test("right-click opens the clip menu; arrows move, Escape closes", async ({ page }) => {
  await card(page, "gap-run").click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Clip actions" });
  await expect(menu).toBeVisible();
  for (const item of ["Rename", "Edit details", "Trim and cuts", "Share…", "Add to Drone Album", "Show in Finder", "Find dead air again", "Move to Trash"]) {
    await expect(menu.getByRole("menuitem", { name: new RegExp(`^${item.replace(/[…]/g, ".")}`) })).toBeVisible();
  }
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem").nth(1)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
});

test("the selection bar rates, flags and trashes several clips", async ({ app, page }) => {
  await card(page, "backyard-loops").click();
  await card(page, "river-dive").click({ modifiers: ["Meta"] });
  const bar = page.getByRole("toolbar", { name: "Selected clips" });
  await bar.getByRole("button", { name: "3 stars" }).click();
  expect((await app.method("library_rate")).at(-1)!).toMatchObject({ rating: 3 });
  await bar.getByRole("button", { name: "Pick" }).click();
  expect((await app.method("library_rate")).at(-1)!).toMatchObject({ flag: "pick" });
  await bar.getByRole("button", { name: "Trash" }).click();
  await expect(dialog(page, "Move 2 clips to the Trash?")).toBeVisible();
});

test("menu bar items run the same actions", async ({ app, page }) => {
  await card(page, "backyard-loops").click();
  await app.page.evaluate(() => window.__qc!.emit("menu", "rate-4"));
  await expect(card(page, "backyard-loops").getByRole("img", { name: "4 of 5 stars" })).toBeVisible();
  await app.page.evaluate(() => window.__qc!.emit("menu", "view-list"));
  await expect(clipTable(page)).toBeVisible();
  const state = (await app.core<{ enabled: Record<string, boolean>; checked: Record<string, boolean> }>("c => c.menuState"));
  expect(state.enabled.pick).toBe(true);
  expect(state.checked["view-list"]).toBe(true);
});

test("a core change from elsewhere shows at once", async ({ app, page }) => {
  await app.core("c => c.rate(['xd0d144c9ce319e86'], 1, null)");
  await expect(card(page, "gap-run").getByRole("img", { name: "1 of 5 stars" })).toBeVisible();
});
