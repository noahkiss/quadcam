// The radio's Settings segment on the mock core: the radio's own settings (radio.yml) grouped
// as the radio's Radio Setup screen groups them, each edit staged through
// `gear_radio_settings_edit` into one "Radio settings" change with Review and Undo, values out
// of range refused before they stage, the card diff in the apply sheet, and axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const RADIO_LINK = `{ kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }`;
const radio = `{ id: "${RADIO_ID}", kind: "radio", link: ${RADIO_LINK}, identity: { board: "pocket" } }`;

/** The saved radio's Settings segment. */
async function openSettings(app: AppFixture, page: Page, plugged = false) {
  await app.open();
  if (plugged) await app.core(`c => c.plug([${radio}])`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("table", { name: "Devices" }).getByRole("button", { name: "Field radio", exact: true }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Settings" }).click();
}

const lastEdit = async (app: AppFixture) => (await app.method("gear_radio_settings_edit")).at(-1);

test("the settings read from the latest backup, grouped as the radio groups them", async ({ app, page }) => {
  await openSettings(app, page);
  await expect(page.getByText(/From backup · /)).toBeVisible();
  for (const name of ["Sound", "Haptic", "Display", "Alarms", "Time and GPS", "Sticks and USB", "Owner", "Model"]) await expect(page.getByRole("region", { name })).toBeVisible();
  const sound = page.getByRole("region", { name: "Sound" });
  await expect(sound.getByLabel("Sound mode")).toHaveValue("mode_all");
  await expect(sound.getByLabel("Beep volume")).toHaveValue("2");
  const alarms = page.getByRole("region", { name: "Alarms" });
  await expect(alarms.getByLabel("Battery low")).toHaveValue("6.6");
  await expect(alarms.getByText("3.0 to 12.0", { exact: true })).toBeVisible();
  await expect(alarms.getByRole("checkbox", { name: "RSSI shutdown alarm" })).toBeChecked();
  const sticks = page.getByRole("region", { name: "Sticks and USB" });
  await expect(sticks.getByLabel("Stick mode")).toHaveValue("1");
  await expect(sticks.getByLabel("Default channel order").locator("option:checked")).toHaveText("AETR");
  // A line the card lacks shows as such, with no control.
  await expect(sticks.getByText("Not on this card")).toBeVisible();
  await expect(sticks.getByLabel("USB mode")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Display" }).getByLabel("Brightness")).toHaveValue("100");
  await expect(page.getByRole("region", { name: "Owner" }).getByLabel("Owner ID")).toHaveValue("    test");
  await expect(page.getByRole("region", { name: "Model" }).getByLabel("Selected model")).toHaveValue("model01.yml");
});

test("each edit stages into one change, shows the radio's value, and Undo drops it", async ({ app, page }) => {
  await openSettings(app, page);
  const sound = page.getByRole("region", { name: "Sound" });
  await sound.getByLabel("Beep volume").selectOption("3");
  await expect.poll(() => lastEdit(app)).toEqual({ device: RADIO_ID, ops: [{ op: "set_scalar", key: "beepVolume", value: "3" }] });
  await expect(page.getByText("1 setting is staged.")).toBeVisible();
  await expect(sound.getByText("On the radio: 0")).toBeVisible();
  await expect(sound.getByText("Staged", { exact: true })).toBeVisible();
  // A number in the radio's units becomes the file's value.
  const alarms = page.getByRole("region", { name: "Alarms" });
  await alarms.getByLabel("Battery low").fill("7.2");
  await alarms.getByLabel("Battery low").press("Enter");
  await expect.poll(() => lastEdit(app)).toEqual({ device: RADIO_ID, ops: [{ op: "set_scalar", key: "vBatWarn", value: "72" }] });
  await alarms.getByRole("checkbox", { name: "RSSI shutdown alarm" }).uncheck();
  await expect.poll(() => lastEdit(app)).toEqual({ device: RADIO_ID, ops: [{ op: "set_scalar", key: "disableRssiPoweroffAlarm", value: "1" }] });
  await page.getByRole("region", { name: "Model" }).getByLabel("Selected model").selectOption("model00.yml");
  await expect.poll(() => lastEdit(app)).toEqual({ device: RADIO_ID, ops: [{ op: "select_model", file: "model00.yml" }] });
  await expect(page.getByText("4 settings are staged.")).toBeVisible();
  expect(await app.method("gear_change_stage")).toHaveLength(0);
  // Back to the radio's value: that setting drops out of the change.
  await sound.getByLabel("Beep volume").selectOption("2");
  await expect(page.getByText("3 settings are staged.")).toBeVisible();
  await expect(sound.getByText("Staged", { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Undo settings edits" }).click();
  await expect(page.getByText(/settings? (is|are) staged/)).toHaveCount(0);
  await expect(alarms.getByLabel("Battery low")).toHaveValue("6.6");
});

test("a value out of range is refused before it stages", async ({ app, page }) => {
  await openSettings(app, page);
  const alarms = page.getByRole("region", { name: "Alarms" });
  await alarms.getByLabel("Battery low").fill("13");
  await alarms.getByLabel("Battery low").press("Enter");
  await expect(alarms.getByRole("alert")).toHaveText("Use a value from 3.0 to 12.0 V.");
  const display = page.getByRole("region", { name: "Display" });
  await display.getByLabel("Backlight off after").fill("42");
  await display.getByLabel("Backlight off after").press("Enter");
  await expect(display.getByRole("alert")).toHaveText("Use steps of 5 s.");
  const owner = page.getByRole("region", { name: "Owner" });
  await owner.getByLabel("Owner ID").fill('a"b');
  await owner.getByLabel("Owner ID").press("Enter");
  await expect(owner.getByRole("alert")).toContainText("letters, digits");
  expect(await app.method("gear_radio_settings_edit")).toHaveLength(0);
});

test("Review shows the radio.yml diff, and the apply verifies; the next read shows the applied value", async ({ app, page }) => {
  await openSettings(app, page, true);
  await expect(page.getByText("From the radio's card")).toBeVisible();
  await page.getByRole("region", { name: "Display" }).getByLabel("Backlight", { exact: true }).selectOption("backlight_mode_on");
  await expect(page.getByText("1 setting is staged.")).toBeVisible();
  await page.getByRole("button", { name: "Review…" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  const diff = sheet.getByRole("region", { name: "Changes" });
  await expect(diff).toContainText("RADIO/radio.yml");
  await expect(diff).toContainText("backlightMode: backlight_mode_keys");
  await expect(diff).toContainText("backlightMode: backlight_mode_on");
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText(/settings? (is|are) staged/)).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Display" }).getByLabel("Backlight", { exact: true })).toHaveValue("backlight_mode_on");
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Settings segment passes axe", async ({ app, page }) => {
      await openSettings(app, page);
      await page.getByRole("region", { name: "Sound" }).getByLabel("Beep volume").selectOption("3");
      await expect(page.getByText("1 setting is staged.")).toBeVisible();
      const alarms = page.getByRole("region", { name: "Alarms" });
      await alarms.getByLabel("Battery low").fill("13");
      await alarms.getByLabel("Battery low").press("Enter");
      await expect(alarms.getByRole("alert")).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
