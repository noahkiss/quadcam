// The switch map and the Controls page on the mock core. The mock answers with the map the
// real core made from the synthetic whoop (e2e/fixtures/switchmap.json) and streams a fake
// radio in USB Joystick mode (`core.radioFrame`). Axes are CH1-8, 0..2048: 1024 is centre.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const CARD = "/Users/pilot/bench/RADIO-COPY";
const DUMP = "/Users/pilot/bench/quad.diff_all.txt";
/** SA down (armed), SB mid, SC down, SD up; sticks centred, throttle low. */
const ARMED = [1024, 1024, 0, 1024, 2048, 1024, 2048, 0];

const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

async function openSwitches(app: AppFixture, page: Page) {
  await app.open();
  await side(page).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Switches" }).click();
  await app.core(`c => c.dialogAnswers.push(${JSON.stringify(CARD)}, [${JSON.stringify(DUMP)}])`);
  await page.getByRole("button", { name: "Open card…" }).click();
  await page.getByRole("button", { name: "Open dump…" }).click();
}

/** A control's rows in the map. */
const control = (page: Page, name: string) => page.getByRole("table", { name: "Switch map" }).getByRole("rowgroup").filter({ has: page.getByRole("rowheader", { name: new RegExp(`^${name}`) }) });

test("the Switches segment shows what each switch does and marks where it is", async ({ app, page }) => {
  await openSwitches(app, page);
  await expect.poll(async () => (await app.method("gear_switch_map")).at(-1)).toEqual({ radio: CARD, fc: [DUMP] });
  const sa = control(page, "SA");
  await expect(sa.getByRole("row", { name: /down CH5 2012 ARM/ })).toBeVisible();
  await expect(control(page, "SC").getByRole("row", { name: /mid CH7 1500 Rate profile 2/ })).toBeVisible();
  await expect(page.getByRole("region", { name: "Conflicts" })).toContainText('Sound "lapclr" is not on the card');

  // Live from the radio's joystick: SA down is marked, ARM is on.
  await page.getByRole("group", { name: "Live" }).getByRole("button", { name: "Radio" }).click();
  await expect.poll(async () => (await app.method("gear_radio_watch")).at(-1)).toEqual({ on: true });
  await app.core(`c => c.radioFrame(${JSON.stringify(ARMED)})`);
  await expect(sa.locator('tr[aria-current="true"]')).toContainText("down");
  await expect(control(page, "SB").locator('tr[aria-current="true"]')).toContainText("mid");
  await expect(page.getByText("Modes on: ARM, HORIZON · Rate profile 3")).toBeVisible();

  await page.getByRole("group", { name: "Live" }).getByRole("button", { name: "Off" }).click();
  await expect.poll(async () => (await app.method("gear_radio_watch")).at(-1)).toEqual({ on: false });
  await expect(page.locator('tr[aria-current="true"]')).toHaveCount(0);
});

test("live from the FC reads its channels once a second", async ({ app, page }) => {
  await openSwitches(app, page);
  await expect(page.getByRole("table", { name: "Switch map" })).toBeVisible();
  // The FC is plugged in and reports SA down, SB up.
  await app.core(`c => { c.fcRc = [1500, 1500, 988, 1500, 2012, 988, 988, 988]; c.plug([{ id: c.gearSeed.FC.id, kind: "fc", link: { kind: "serial", port: "/dev/cu.usbmodemTEST1", vid: 0x0483, pid: 0x5740, product: null }, identity: {} }]); }`);
  await page.getByRole("group", { name: "Live" }).getByRole("button", { name: "FC" }).click();
  await expect(page.getByText("Modes on: ARM, ANGLE · Rate profile 1")).toBeVisible();
  await expect(control(page, "SB").locator('tr[aria-current="true"]')).toContainText("up");
  const reads = (await app.method("gear_switch_map")).filter((p) => p.live);
  expect(reads[0]).toMatchObject({ live: true, port: "/dev/cu.usbmodemTEST1" });
});

test("the Controls page draws the sticks in the chosen mode, the channels and the map", async ({ app, page }) => {
  await app.open();
  await side(page).getByRole("button", { name: /^Controls/ }).click();
  await expect(page.getByRole("heading", { name: "Controls" })).toBeVisible();
  await expect(page.getByRole("status", { name: "Radio" })).toHaveText("No radio in USB Joystick mode. Plug the radio in and choose USB Joystick on it.");

  // Throttle up, yaw right, roll left; button 2 on.
  await app.core(`c => c.radioFrame([0, 1024, 2048, 2048, 0, 0, 0, 0], 2)`);
  await expect(page.getByRole("status", { name: "Radio" })).toHaveText("Test Radio Joystick");
  // Mode 2 (default): throttle and yaw on the left, pitch and roll on the right.
  await expect(page.getByRole("group", { name: "Stick mode" }).getByRole("button", { name: "Mode 2" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("img", { name: "Left stick: Throttle +100%, Yaw +100%" })).toBeVisible();
  await expect(page.getByRole("img", { name: "Right stick: Pitch 0%, Roll -100%" })).toBeVisible();
  await expect(page.getByRole("figure", { name: "Left stick" })).toContainText("Left and right: turns the nose");
  await expect(page.getByRole("meter", { name: "CH3" })).toHaveAttribute("aria-valuenow", "2012");
  await expect(page.getByRole("list", { name: "Buttons" }).getByRole("listitem", { name: "Button 2, on" })).toBeVisible();

  // Mode 1: pitch and yaw on the left. The choice is saved.
  await page.getByRole("group", { name: "Stick mode" }).getByRole("button", { name: "Mode 1" }).click();
  await expect(page.getByRole("img", { name: "Left stick: Pitch 0%, Yaw +100%" })).toBeVisible();
  await expect(page.getByRole("img", { name: "Right stick: Throttle +100%, Roll -100%" })).toBeVisible();
  await expect.poll(async () => (await app.method("settings_set")).at(-1)).toEqual({ values: { stickMode: 1 } });

  // The map from a card, marked as the switches move.
  await app.core(`c => c.dialogAnswers.push(${JSON.stringify(CARD)})`);
  await page.getByRole("button", { name: "Open card…" }).click();
  await app.core(`c => c.radioFrame(${JSON.stringify(ARMED)})`);
  await expect(control(page, "SA").locator('tr[aria-current="true"]')).toContainText("down");
  await app.core(`c => c.radioFrame([1024, 1024, 0, 1024, 0, 1024, 2048, 0])`);
  await expect(control(page, "SA").locator('tr[aria-current="true"]')).toContainText("up");

  // Pulled: the page says so, and the stream stops when the page closes.
  await app.core(`c => c.radioPlug(false)`);
  await expect(page.getByRole("status", { name: "Radio" })).toContainText("No radio in USB Joystick mode");
  await side(page).getByRole("button", { name: /^All clips/ }).click();
  await expect.poll(async () => (await app.method("gear_radio_watch")).at(-1)).toEqual({ on: false });
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Controls page and the Switches segment pass axe", async ({ app, page }) => {
      await openSwitches(app, page);
      await page.getByRole("group", { name: "Live" }).getByRole("button", { name: "Radio" }).click();
      await app.core(`c => c.radioFrame(${JSON.stringify(ARMED)})`);
      await expect(page.locator('tr[aria-current="true"]').first()).toBeVisible();
      const axe = async () => {
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        return r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`);
      };
      expect(await axe()).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/switches-${scheme}.png`, fullPage: true });
      await side(page).getByRole("button", { name: /^Controls/ }).click();
      await app.core(`c => c.radioFrame([512, 1536, 1800, 300, 2048, 1024, 2048, 0], 5)`);
      await expect(page.getByRole("status", { name: "Radio" })).toHaveText("Test Radio Joystick");
      expect(await axe()).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/controls-${scheme}.png`, fullPage: true });
    });
  });
}
