// The glass chrome (macOS 26 and later): the app sets `__QUADCAM_GLASS__` before the page
// loads. The sidebar runs the window's full height under the traffic lights and is clear over
// the glass; the toolbar spans the content column and drags the window. Without the flag the
// frame keeps the standard layout.
import { expect, test } from "./fixtures";

const box = async (loc: import("@playwright/test").Locator) => (await loc.boundingBox())!;

test("glass: the sidebar runs the full height, the toolbar sits beside it", async ({ app, page }) => {
  await page.addInitScript(() => (window.__QUADCAM_GLASS__ = true));
  await app.open();
  await expect(page.locator("html")).toHaveAttribute("data-glass", "");
  const side = await box(page.getByRole("navigation", { name: "Library" }));
  const bar = await box(page.locator("header").first());
  const view = page.viewportSize()!;
  expect(side.y).toBe(0);
  expect(side.height).toBe(view.height);
  expect(side.width).toBe(232);
  expect(bar.x).toBe(232);
  expect(bar.y).toBe(0);
  expect(bar.height).toBe(52);
  // The page is clear where the glass shows; the content paints its own background.
  const bg = (sel: string) => page.locator(sel).first().evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(await bg("body")).toBe("rgba(0, 0, 0, 0)");
  expect(await bg("nav")).toBe("rgba(0, 0, 0, 0)");
  expect(await bg("main")).not.toBe("rgba(0, 0, 0, 0)");
  // The toolbar and the strip under the traffic lights drag the window.
  await expect(page.locator("header").first()).toHaveAttribute("data-tauri-drag-region", "deep");
  await expect(page.locator("nav > [data-tauri-drag-region]")).toHaveCount(1);
  await expect(page.locator("header h1")).toBeHidden();
});

test("without glass the toolbar spans the window above the sidebar", async ({ app, page }) => {
  await app.open();
  await expect(page.locator("html")).not.toHaveAttribute("data-glass");
  const side = await box(page.getByRole("navigation", { name: "Library" }));
  const bar = await box(page.locator("header").first());
  expect(bar.x).toBe(0);
  expect(bar.width).toBe(page.viewportSize()!.width);
  expect(side.y).toBe(52);
  expect(side.width).toBe(232);
  await expect(page.locator("header").first()).not.toHaveAttribute("data-tauri-drag-region");
  await expect(page.locator("nav > [data-tauri-drag-region]")).toBeHidden();
});
