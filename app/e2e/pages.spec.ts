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
