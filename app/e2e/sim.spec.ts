// The sim's radio calibration page on the mock core (`ipc/mock/sim.ts`): which radio the
// joystick is, the suggestions from the aircraft with their source, the short check for a
// known model, changing a suggestion, editing an end, Recalibrate, and saving by radio key.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const REST = [1024, 1024, 0, 1024, 0, 0, 0, 0];
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });

async function openSim(app: AppFixture, page: Page) {
  await app.open();
  await app.core(`c => c.radioFrame(${JSON.stringify(REST)})`);
  await side(page).getByRole("button", { name: /^Sim radio/ }).click();
  await expect(page.getByRole("heading", { name: "Sim radio" })).toBeVisible();
}

const step = (page: Page) => page.getByRole("status", { name: "Step" });

test("a known radio model gets the short check, suggestions show their source, and the calibration saves", async ({ app, page }) => {
  await openSim(app, page);
  await expect(page.getByRole("status", { name: "Radio" })).toContainText("Test Radio Joystick: No saved radio is this model yet");
  const sug = page.getByRole("region", { name: "Suggestions" });
  await expect(sug.locator('[data-target="arm"]')).toContainText("SA down (from your radio model)");
  await expect(sug.locator('[data-target="turtle"]')).toContainText("SE down (from your radio model)");
  await expect(sug.locator('[data-target="reset"]')).toContainText("TrimRudLeft (from your radio model)");
  await expect(sug).toContainText("Roll CH1, pitch CH2, throttle CH3, yaw CH4 (from your radio model)");

  // The short check: move, then the reset control (the arm switch is known), then review.
  await expect(step(page)).toHaveText("Move each stick all the way both ways.");
  const start = (await app.method("gear_sim_calibrate")).find((p) => (p as { action: string }).action === "start") as { quick: boolean; arm_known: boolean };
  expect(start).toMatchObject({ quick: true, arm_known: true });
  await page.getByRole("button", { name: "Done" }).click();
  await expect(step(page)).toHaveText("Press the button or trim to use for reset.");
  await page.getByRole("button", { name: "Keep Button 2" }).click();
  await expect(step(page)).toHaveText("Check each axis, then save.");

  // Live output: the calibrated sticks.
  await app.core(`c => c.radioFrame([2048, 1024, 2048, 0, 0, 0, 0, 0])`);
  await expect(page.getByRole("img", { name: "Right stick: Pitch 0%, Roll +100%" })).toBeVisible();

  // Change a suggestion: the turtle switch, cleared.
  await sug.getByRole("button", { name: "Change Turtle" }).click();
  await expect(step(page)).toHaveText("Move the control for Turtle.");
  await page.getByRole("button", { name: "Clear" }).click();
  await expect(sug.locator('[data-target="turtle"]')).toContainText("None");

  // An edited end is marked; names follow Reverse; Recalibrate clears the edit.
  const axes = page.getByRole("table", { name: "Axes" });
  await axes.getByRole("spinbutton", { name: "Pitch Forward end" }).fill("1900");
  await expect(axes.getByRole("row", { name: /Pitch/ })).toContainText("(edited)");
  await axes.getByRole("checkbox", { name: "Pitch reverse" }).check();
  await expect(axes.getByRole("spinbutton", { name: "Pitch Back end" })).toHaveValue("1900");
  await page.getByRole("button", { name: "Save" }).click();
  const saved = (await app.method("gear_sim_calibration_save")).at(-1) as { radio: string; calibration: { pitch: { edited_high: number; reverse: boolean } } };
  expect(saved.radio).toBe("usb-00000000000000f1");
  expect(saved.calibration.pitch).toMatchObject({ edited_high: 1900, reverse: true });
  await page.getByRole("button", { name: "Recalibrate" }).click();
  await expect(step(page)).toHaveText("Move both sticks around their full travel.");
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Sim radio page passes axe", async ({ app, page }) => {
      await openSim(app, page);
      await page.getByRole("button", { name: "Done" }).click();
      await page.getByRole("button", { name: "No reset control" }).click();
      await expect(page.getByRole("table", { name: "Axes" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/sim-radio-${scheme}.png`, fullPage: true });
    });
  });
}
