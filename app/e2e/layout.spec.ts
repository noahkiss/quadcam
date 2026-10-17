// The responsive frame on every page: at 420, 900, 1400 and 2400 px the window never scrolls
// sideways, the content column fills the window less the sidebar up to --content-max (1760
// px) and centres past it, and below 720 px the sidebar is an overlay behind a toolbar button.
// With QC_SHOTS=<dir> it also saves a screenshot of each page at each width.
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

const WIDTHS = [420, 900, 1400, 2400] as const;
const CAP = 1760;
const SIDEBAR = 232;
const shots = process.env.QC_SHOTS;

/** Sidebar items to open, by their accessible name (counts follow the label). */
const PAGES = ["All clips", "Connected", "Devices", "Bench", "Controls", "Voices", "Sim", "Pack up", "Flights", "Packs", "Repairs", "Firmware", "Storage"] as const;

const nav = (page: Page) => page.getByRole("navigation", { name: "Library" });

/** Opens a sidebar item; in a narrow window through the toolbar's sidebar button. */
async function go(page: Page, name: string) {
  const narrow = (page.viewportSize()?.width ?? 1280) <= 720;
  if (narrow) await page.getByRole("button", { name: "Show sidebar" }).click();
  await nav(page).getByRole("button", { name: new RegExp(`^${name}`) }).first().click();
  if (narrow) await expect(nav(page)).toBeHidden();
}

/** The scroller that frames the page, and the box its content fills. */
async function measure(page: Page) {
  return page.evaluate(() => {
    const main = document.querySelector("main")!;
    const frame = (main.querySelector("section[class*='page'], [role='grid'], [class*='scroll']") as HTMLElement | null) ?? main;
    const cs = getComputedStyle(frame);
    const left = frame.getBoundingClientRect().left + parseFloat(cs.paddingLeft);
    const width = frame.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    const r = main.getBoundingClientRect();
    return {
      docScroll: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      frameScroll: frame.scrollWidth - frame.clientWidth,
      mainLeft: r.left,
      mainWidth: r.width,
      inner: frame.clientWidth,
      frameLeft: frame.getBoundingClientRect().left,
      left,
      width,
    };
  });
}

for (const name of PAGES) {
  test(`${name}: fills the window to ${CAP} px, centres past it, never scrolls sideways`, async ({ app, page }) => {
    await page.setViewportSize({ width: 1400, height: 900 });
    await app.open("gear", { settings: { simPreview: true } });
    await go(page, name);
    for (const w of WIDTHS) {
      await page.setViewportSize({ width: w, height: 900 });
      await page.waitForTimeout(150);
      const m = await measure(page);
      expect(m.docScroll, `${name} at ${w}: the window scrolls sideways`).toBeLessThanOrEqual(0);
      expect(m.frameScroll, `${name} at ${w}: the page scrolls sideways`).toBeLessThanOrEqual(1);
      const column = w <= 720 ? w : w - SIDEBAR;
      expect(Math.round(m.mainWidth)).toBe(column);
      // The scroller's own width less a scroll bar, where one takes room.
      const pad = w <= 720 ? 16 : 24;
      if (m.inner - 2 * pad > CAP) {
        expect(Math.round(m.width)).toBe(CAP);
        expect(Math.abs(m.left - m.frameLeft - (m.inner - CAP) / 2)).toBeLessThanOrEqual(1);
      } else {
        expect(Math.round(m.width)).toBe(Math.round(m.inner - 2 * pad));
      }
      if (shots) await page.screenshot({ path: `${shots}/layout-${name.toLowerCase().replace(/\s+/g, "-")}-${w}.png`, fullPage: false });
    }
  });
}

test("a narrow window shows the sidebar over the page from the toolbar", async ({ app, page }) => {
  await page.setViewportSize({ width: 420, height: 800 });
  await app.open();
  await expect(nav(page)).toBeHidden();
  const button = page.getByRole("button", { name: "Show sidebar" });
  await expect(button).toHaveAttribute("aria-expanded", "false");
  await button.click();
  await expect(nav(page)).toBeVisible();
  await expect(page.getByRole("button", { name: "Hide sidebar" })).toHaveAttribute("aria-expanded", "true");
  // Escape closes it; a pick closes it too.
  await page.keyboard.press("Escape");
  await expect(nav(page)).toBeHidden();
  await page.getByRole("button", { name: "Show sidebar" }).click();
  await nav(page).getByRole("button", { name: /^Connected/ }).click();
  await expect(nav(page)).toBeHidden();
  await expect(page.getByRole("heading", { name: "Connected" })).toBeVisible();
});

test("a narrow library folds sort and view into a menu and keeps Import", async ({ app, page }) => {
  await page.setViewportSize({ width: 420, height: 800 });
  await app.open();
  await expect(page.getByRole("button", { name: "Import…" })).toBeVisible();
  await expect(page.getByLabel("Sort by")).toBeHidden();
  await page.getByRole("button", { name: "View options" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Show as list" }).click();
  await expect(page.locator("table")).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(page.getByLabel("Sort by")).toBeVisible();
  await expect(page.getByRole("button", { name: "Show sidebar" })).toBeHidden();
});

test("a long library folder fits its first-run card at every width", async ({ app, page }) => {
  const folder = "/var/folders/zs/0123456789abcdefghijklmn/T/tmp.b8v81W12mS/a-very-long-folder-name-for-flights/Movies/quadcam";
  await app.open("empty", { settings: { outputDir: folder } });
  const card = page.locator("[class*='card']").filter({ has: page.getByRole("heading", { name: "Library folder" }) });
  const path = card.locator("[data-component='path']");
  await expect(path).toHaveAttribute("title", folder);
  await expect(path).toContainText("/quadcam");
  for (const w of [420, 900, 1400]) {
    await page.setViewportSize({ width: w, height: 900 });
    await page.waitForTimeout(150);
    const m = await page.evaluate(() => {
      const first = document.querySelector("section[aria-label='First run']") as HTMLElement;
      const setup = first.querySelector("section[aria-labelledby='setup-title']") as HTMLElement;
      const path = setup.querySelector("[data-component='path']") as HTMLElement;
      const card = path.closest("[class*='card']") as HTMLElement;
      const tail = path.lastElementChild!.getBoundingClientRect();
      const box = card.getBoundingClientRect();
      return {
        docScroll: document.documentElement.scrollWidth - document.documentElement.clientWidth,
        firstScroll: first.scrollWidth - first.clientWidth,
        setupScroll: setup.scrollWidth - setup.clientWidth,
        cardOut: box.right - setup.getBoundingClientRect().right,
        pathOut: path.getBoundingClientRect().right - box.right,
        tailShown: tail.width > 0 && tail.right <= box.right,
      };
    });
    expect(m.docScroll, `at ${w}: the window scrolls sideways`).toBeLessThanOrEqual(0);
    expect(m.firstScroll, `at ${w}: the first-run page scrolls sideways`).toBeLessThanOrEqual(1);
    expect(m.setupScroll, `at ${w}: Set up once scrolls sideways`).toBeLessThanOrEqual(1);
    expect(m.cardOut, `at ${w}: the card passes Set up once`).toBeLessThanOrEqual(0);
    expect(m.pathOut, `at ${w}: the path passes its card`).toBeLessThanOrEqual(0);
    expect(m.tailShown, `at ${w}: the last segment is hidden`).toBe(true);
  }
});
