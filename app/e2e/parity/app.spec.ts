import { area, expect, test } from "../fixtures";

area("app");

test("an empty library shows the first-run screen", async ({ app, page }) => {
  await app.open("empty");
  await expect(page.getByRole("heading", { name: "Import your first flights" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Set up once" })).toBeVisible();
  await expect(page.getByText("~/Movies/quadcam")).toBeVisible();
});

test("without ffmpeg, a banner says so and Import is off", async ({ app, page }) => {
  await app.open("no-tools");
  await expect(page.getByText("ffmpeg not found.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Import…" }).first()).toBeDisabled();
});

test("page text is not selectable; text fields are", async ({ app, page }) => {
  await app.open();
  await expect(page.getByRole("article").first()).toBeVisible();
  const select = (sel: string) => page.locator(sel).first().evaluate((e) => getComputedStyle(e).webkitUserSelect || getComputedStyle(e).userSelect);
  expect(await select("body")).toBe("none");
  expect(await select("input[type=search]")).not.toBe("none");
});

test("the web view's context menu shows only over text fields", async ({ app, page }) => {
  await app.open();
  await expect(page.getByRole("article").first()).toBeVisible();
  const prevented = (sel: string) =>
    page.locator(sel).first().evaluate((e) => !e.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
  expect(await prevented("h2")).toBe(true);
  expect(await prevented("input[type=search]")).toBe(false);
});

test("a large library scrolls and keeps keyboard selection", async ({ app, page }) => {
  await app.open("many");
  await expect(page.getByRole("article", { name: "gap-run" })).toBeVisible();
  await page.getByRole("article", { name: "gap-run" }).click();
  for (let i = 0; i < 12; i++) await page.keyboard.press("ArrowDown");
  await expect(page.locator('article[aria-selected="true"]')).toBeInViewport();
});
