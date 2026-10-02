import { expect, selectedClips, test } from "../fixtures";

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
  await expect(selectedClips(page)).toBeInViewport();
});

for (const view of ["grid", "list"] as const) {
  test(`the ${view} view scrolls with the mouse wheel inside the window`, async ({ app, page }) => {
    await page.setViewportSize({ width: 1100, height: 600 });
    await app.open("many");
    await expect(page.getByRole("article").first()).toBeVisible();
    if (view === "list") await page.getByRole("radio", { name: /list/i }).or(page.getByRole("button", { name: /list/i })).first().click();
    const box = page.locator(view === "grid" ? '[role="grid"][aria-label="Clips"]' : "table").first();
    const scroller = view === "grid" ? box : box.locator("xpath=..");
    const fits = await scroller.evaluate((e) => e.getBoundingClientRect().bottom <= window.innerHeight + 1);
    expect(fits).toBe(true);
    const before = await scroller.evaluate((e) => [e.scrollTop, e.scrollHeight - e.clientHeight]);
    expect(before[1]).toBeGreaterThan(0);
    await scroller.hover();
    await page.mouse.wheel(0, 800);
    await expect.poll(() => scroller.evaluate((e) => e.scrollTop)).toBeGreaterThan(0);
  });
}
