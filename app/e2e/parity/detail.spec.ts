import { area, dialog, expect, test } from "../fixtures";

area("detail");

const params = (c: { args: Record<string, unknown> }) => c.args.params as Record<string, unknown>;

test.beforeEach(async ({ app, page }) => {
  await app.open();
  await page.getByRole("article", { name: "gap-run" }).dblclick();
  await expect(page.getByRole("button", { name: "Library" })).toBeVisible();
});

test("double-click opens the clip; Escape goes back", async ({ page }) => {
  await expect(page.getByRole("tab", { name: "Details" })).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("article", { name: "gap-run" })).toBeVisible();
});

test("Space and T open the selected clip", async ({ page }) => {
  await page.keyboard.press("Escape");
  await page.getByRole("article", { name: "river-dive" }).click();
  await page.keyboard.press("t");
  await expect(page.getByRole("button", { name: "Library" })).toBeVisible();
  await page.getByRole("button", { name: "Library" }).click();
  await page.getByRole("article", { name: "river-dive" }).click();
  await page.keyboard.press(" ");
  await expect(page.getByRole("button", { name: "Library" })).toBeVisible();
});

test("the Flight tab shows the radio-log numbers", async ({ page }) => {
  await page.getByRole("tab", { name: "Flight" }).click();
  await expect(page.getByRole("tab", { name: "Flight" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByText("Armed time")).toBeVisible();
  await expect(page.getByText("3.42 V")).toBeVisible();
  await expect(page.getByText("2 packs")).toBeVisible();
});

test("Details edits go to the core and undo", async ({ app, page }) => {
  await page.getByLabel("Note").fill("windy");
  await page.getByLabel("Note").press("Tab");
  await expect.poll(async () => (await app.calls("core_call", "library_edit")).map(params)).toContainEqual({ id: "dce3e50d5303bc2b", note: "windy" });
  await page.getByRole("combobox", { name: "Place" }).selectOption("Riverside park");
  await expect.poll(async () => (await app.calls("core_call", "library_edit")).map(params)).toContainEqual({ id: "dce3e50d5303bc2b", place: "Riverside park" });
  await page.getByRole("combobox", { name: "Aircraft" }).selectOption("Five-inch");
  await expect.poll(async () => (await app.calls("core_call", "library_edit")).map(params)).toContainEqual({ id: "dce3e50d5303bc2b", profile: "Five-inch" });
  await page.getByLabel("Add keyword").fill("gaps");
  await page.getByLabel("Add keyword").press("Enter");
  await expect.poll(async () => (await app.calls("core_call", "library_edit")).map(params)).toContainEqual({ id: "dce3e50d5303bc2b", keywords: ["FPV", "backyard", "gaps"] });
  await page.getByRole("button", { name: "Library" }).focus();
  await page.keyboard.press("Meta+z");
  await expect.poll(async () => (await app.calls("core_call", "library_edit")).map(params).at(-1)).toEqual({ id: "dce3e50d5303bc2b", keywords: ["FPV", "backyard"] });
});

test("keys rate and flag the open clip", async ({ page }) => {
  await page.keyboard.press("1");
  await expect(page.getByRole("img", { name: "1 of 5 stars" })).toBeVisible();
  await page.keyboard.press("x");
  await expect(page.getByRole("button", { name: "Reject", pressed: true })).toBeVisible();
});

test("Return renames the open clip", async ({ app, page }) => {
  await page.keyboard.press("Enter");
  await page.getByRole("textbox", { name: "Clip name" }).fill("gap run two");
  await page.keyboard.press("Enter");
  await expect(page.getByText("gap run two").first()).toBeVisible();
  expect((await app.calls("core_call", "library_rename")).map(params)).toEqual([{ id: "dce3e50d5303bc2b", name: "gap run two" }]);
});

test("typed in and out points add a cut, then Save writes it", async ({ app, page }) => {
  await page.getByRole("textbox", { name: "In point" }).fill("0:30.0");
  await page.getByRole("textbox", { name: "In point" }).press("Tab");
  await page.getByRole("textbox", { name: "Out point" }).fill("0:40.0");
  await page.getByRole("textbox", { name: "Out point" }).press("Tab");
  await page.getByRole("button", { name: "Add cut" }).click();
  await expect.poll(async () => (await app.calls("core_call", "library_cuts")).map(params)).toContainEqual({
    id: "dce3e50d5303bc2b", cuts: [{ start: 10, end: 25 }, { start: 30, end: 40 }], removed_cuts: null,
  });
  await page.getByRole("button", { name: "Save 1 cut" }).click();
  await expect(page.getByRole("status")).toContainText("1 cut saved.");
});

test("a moment frames the in and out points", async ({ page }) => {
  await page.getByRole("button", { name: "Roll", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "In point" })).toHaveValue("0:19.0");
  await expect(page.getByRole("textbox", { name: "Out point" })).toHaveValue("0:22.5");
});

test("removing a saved cut asks what happens to its file", async ({ app, page }) => {
  await page.getByRole("button", { name: "Remove cut 1" }).click();
  const dlg = dialog(page, "Remove the cut?");
  await expect(dlg).toBeVisible();
  await dlg.getByRole("button", { name: "Move to Trash" }).click();
  await expect.poll(async () => (await app.calls("core_call", "library_cuts")).map(params)).toContainEqual({ id: "dce3e50d5303bc2b", cuts: [], removed_cuts: "trash" });
  await expect(page.getByRole("status")).toContainText("Cut file moved to the Trash.");
});

test("Use keep ranges turns them into cuts", async ({ app, page }) => {
  await page.getByRole("button", { name: "Use keep ranges (2)" }).click();
  await expect.poll(async () => (await app.calls("core_call", "library_cuts")).map(params)).toContainEqual({
    id: "dce3e50d5303bc2b", cuts: [{ start: 10, end: 25 }, { start: 0, end: 50 }, { start: 60, end: 120 }], removed_cuts: null,
  });
});

test("Play loads the preview", async ({ app, page }) => {
  await page.getByRole("button", { name: "Play" }).click();
  await expect.poll(async () => (await app.calls("core_call", "library_preview")).length).toBe(1);
  await expect(page.locator("video").filter({ visible: true })).toHaveCount(1);
});

test("the menu bar's next and previous clip step through the library", async ({ app, page }) => {
  await app.page.evaluate(() => window.__qc!.emit("menu", "prev-clip"));
  await expect(page.getByText("backyard-loops", { exact: true }).filter({ visible: true })).toHaveCount(1);
  await app.page.evaluate(() => window.__qc!.emit("menu", "next-clip"));
  await expect(page.getByText("gap-run", { exact: true }).filter({ visible: true })).toHaveCount(1);
});
