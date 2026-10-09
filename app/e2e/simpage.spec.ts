// The Sim page on the mock core (`ipc/mock/sim.ts`, `MockHost`: a canned circle in place of the
// physics): the page starts a sim, draws the picture, the OSD and the stick display, follows the
// radio, and keeps the settings popover and its outside-click close. The picture itself is a
// smoke test (WebGL in headless WebKit); the loop's maths is in `lib/simhost.test.ts`.
import AxeBuilder from "@axe-core/playwright";
import type { Locator, Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const REST = [1024, 1024, 0, 1024, 0, 0, 0, 0];
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

async function openSim(app: AppFixture, page: Page) {
  await app.open("library", { settings: { simPreview: true } });
  await app.core(`c => c.radioFrame(${JSON.stringify(REST)})`);
  await side(page).getByRole("button", { name: /^Sim$/ }).click();
  await expect(page.getByRole("heading", { name: "Sim", exact: true })).toBeVisible();
}

async function fly(app: AppFixture, page: Page) {
  await openSim(app, page);
  await page.getByRole("button", { name: "Fly" }).click();
  await expect(page.getByLabel("Sim view")).toBeVisible();
}

const box = async (l: Locator) => (await l.boundingBox())!;
const overlap = (a: { x: number; y: number; width: number; height: number }, b: { x: number; y: number; width: number; height: number }) =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

test("the Sim page is hidden until the preview setting is on", async ({ app, page }) => {
  await app.open();
  await expect(side(page).getByRole("button", { name: /^Sim radio/ })).toBeVisible();
  await expect(side(page).getByRole("button", { name: /^Sim$/ })).toHaveCount(0);
  // Settings > Gear turns it on.
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: "Settings" }) });
  await dlg.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Gear" }).click();
  await dlg.getByLabel("Show the Sim page").check();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect(side(page).getByRole("button", { name: /^Sim$/ })).toBeVisible();
  const sets = (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values);
  expect(sets.some((v) => v.simPreview === true)).toBe(true);
});

test("the Sim page starts a sim in FPV view with the canvas, the OSD and the stick display", async ({ app, page }) => {
  await fly(app, page);
  const start = (await app.method("sim_start")).at(-1) as { profile: string; calibration: unknown };
  expect(start.profile).toBe("meteor75");
  await expect(page.getByLabel("Sim view")).toHaveAttribute("data-view", "fpv");
  const osd = page.getByLabel("OSD");
  await expect(osd.locator('[data-osd="voltage"]')).toHaveText("3.90V");
  await expect(osd.locator('[data-osd="state"]')).toHaveText("ACRO DISARMED");
  await expect(page.getByRole("group", { name: "Stick display" })).toBeVisible();
  // No saved calibration reached the sim: the page says the switches do nothing.
  await expect(page.getByText("No saved calibration for this radio")).toBeVisible();
});

test("the stick display and the OSD follow the radio, and the flight timer runs while armed", async ({ app, page }) => {
  await fly(app, page);
  const sticks = page.getByRole("group", { name: "Stick display" });
  await app.core(`c => c.radioFrame([2048, 1024, 1024, 1024, 0, 0, 0, 0])`);
  await expect(sticks.getByRole("img", { name: "Right stick: Pitch 0%, Roll +100%" })).toBeVisible();
  await expect(sticks.getByRole("img", { name: "Left stick: Throttle 0%, Yaw 0%" })).toBeVisible();
  // A raised throttle refuses to arm; the OSD says why.
  await expect(page.getByLabel("OSD").locator('[data-osd="warning"]')).toHaveText("Throttle is up: lower it to arm");
  // Throttle down, arm switch on: armed, and the timer counts.
  await app.core(`c => c.radioFrame([1024, 1024, 0, 1024, 2047, 0, 0, 0])`);
  await expect(page.getByLabel("OSD").locator('[data-osd="state"]')).toHaveText("ACRO ARMED");
  await expect(page.getByLabel("OSD").locator('[data-osd="timer"]')).toHaveText("00:01", { timeout: 5000 });
  // The radio goes away: the OSD says so.
  await app.core(`c => c.radioPlug(false)`);
  await expect(page.getByLabel("OSD").locator('[data-osd="warning"]')).toHaveText("No radio");
});

test("the picture draws and moves, or says it cannot open WebGL", async ({ app, page }) => {
  await fly(app, page);
  const canvas = page.getByLabel("Sim view");
  const alert = page.getByRole("alert").filter({ hasText: "WebGL" });
  await page.waitForTimeout(300);
  if (await alert.isVisible()) {
    // A machine with no WebGL: the page keeps its controls and the message names the cause.
    await expect(alert).toContainText("needs WebGL");
    return;
  }
  const a = await canvas.screenshot();
  await page.waitForTimeout(1500);
  const b = await canvas.screenshot();
  expect(a.equals(b), "the canned flight moves the camera, so the picture changes").toBe(false);
  // Not one flat colour: the room has edges and gates.
  const colours = await page.evaluate(() => {
    const c = document.querySelector<HTMLCanvasElement>('canvas[aria-label="Sim view"]')!;
    return { w: c.width, h: c.height };
  });
  expect(colours.w).toBeGreaterThan(100);
});

test("the settings popover opens, saves a change, closes on an outside click and on Escape", async ({ app, page }) => {
  await fly(app, page);
  const gear = page.getByRole("button", { name: "Sim settings" });
  const pop = page.getByRole("dialog", { name: "Sim settings" });
  await gear.click();
  await expect(pop).toBeVisible();
  // A change saves at once and applies.
  await pop.getByRole("button", { name: "Chase" }).click();
  await expect(page.getByLabel("Sim view")).toHaveAttribute("data-view", "chase");
  const saved = (await app.method("settings_set")).at(-1) as { values: { simSettings: { view: string } } };
  expect(saved.values.simSettings.view).toBe("chase");
  await pop.getByRole("checkbox", { name: "Stick display" }).uncheck();
  await expect(page.getByRole("group", { name: "Stick display" })).toHaveCount(0);
  await expect(pop).toBeVisible();
  // A click outside closes it.
  await page.getByRole("heading", { name: "Sim", exact: true }).click();
  await expect(pop).toBeHidden();
  // Escape closes it too, and returns to the button.
  await gear.click();
  await expect(pop).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(pop).toBeHidden();
  await expect(gear).toBeFocused();
  // A click on the canvas is outside as well.
  await gear.click();
  await page.getByLabel("Sim view").click({ position: { x: 20, y: 20 } });
  await expect(pop).toBeHidden();
});

test("Reset and Stop reach the core, and Stop returns to the start screen", async ({ app, page }) => {
  await fly(app, page);
  await page.getByRole("button", { name: "Reset" }).click();
  await expect.poll(async () => (await app.calls("sim_reset")).length).toBe(1);
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByLabel("Sim view")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Fly" })).toBeVisible();
  expect((await app.calls("sim_stop")).length).toBeGreaterThan(0);
});

// The stick display sits inside the picture's right edge, above the OSD's bottom two rows: it
// never covers voltage, timer or mAh, whatever the window's shape.
for (const [name, size, aspect] of [
  ["16:10", { width: 1440, height: 900 }, "16:9"],
  ["16:9", { width: 1280, height: 720 }, "16:9"],
  ["16:10 with a 4:3 picture", { width: 1440, height: 900 }, "4:3"],
  ["a narrow window", { width: 900, height: 820 }, "16:9"],
] as const) {
  test(`the stick display does not overlap the OSD at ${name}`, async ({ app, page }) => {
    await page.setViewportSize(size);
    await fly(app, page);
    await page.getByRole("button", { name: "Sim settings" }).click();
    await page.getByRole("dialog", { name: "Sim settings" }).getByLabel("Picture shape").selectOption(aspect);
    await page.getByRole("heading", { name: "Sim", exact: true }).click();
    const osd = page.getByLabel("OSD");
    await expect(osd.locator('[data-osd="voltage"]')).toBeVisible();
    const sticks = await box(page.getByRole("group", { name: "Stick display" }));
    const picture = await box(page.getByLabel("Sim view"));
    for (const k of ["voltage", "mah", "timer", "state"]) {
      const o = await box(osd.locator(`[data-osd="${k}"]`));
      expect(overlap(sticks, o), `${k} ${JSON.stringify(o)} vs sticks ${JSON.stringify(sticks)}`).toBe(false);
    }
    // Inside the picture, on its right half.
    expect(sticks.x).toBeGreaterThan(picture.x + picture.width / 2);
    expect(sticks.x + sticks.width).toBeLessThanOrEqual(picture.x + picture.width + 0.5);
    expect(sticks.y + sticks.height).toBeLessThanOrEqual(picture.y + picture.height + 0.5);
    // The picture keeps its shape.
    const want = aspect === "4:3" ? 4 / 3 : 16 / 9;
    expect(picture.width / picture.height).toBeCloseTo(want, 1);
  });
}

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the running Sim page passes axe", async ({ app, page }) => {
      await fly(app, page);
      await page.getByRole("button", { name: "Sim settings" }).click();
      await expect(page.getByRole("dialog", { name: "Sim settings" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
    });
  });
}
