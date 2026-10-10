import { dialog, expect, test } from "../fixtures";
import type { Page } from "@playwright/test";

const NOTHING_FOUND = "Nothing found. If macOS asked to allow an accessory, click Allow.";
const side = (page: Page) => page.getByRole("navigation", { name: "Library" });
const bar = (page: Page) => page.getByRole("contentinfo", { name: "Gear status" });

test("Gear sits in the sidebar, collapses, and says what to do when nothing is found", async ({ page, app }) => {
  await app.open();
  await expect(page.getByRole("article").first()).toBeVisible();
  await expect(bar(page)).toHaveText("No gear connected");

  const toggle = side(page).getByRole("button", { name: "Gear" });
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await side(page).getByRole("button", { name: /^Connected/ }).click();
  await expect(page.getByRole("heading", { name: "Connected" })).toBeVisible();
  await expect(page.getByText(NOTHING_FOUND)).toBeVisible();

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(side(page).getByRole("button", { name: /^Connected/ })).toBeHidden();
  await toggle.click();

  // Back to the library from the sidebar.
  await side(page).getByRole("button", { name: /^All clips/ }).click();
  await expect(page.getByRole("article").first()).toBeVisible();
});

test("the status bar shows each kind plugged in, and a click opens the device", async ({ page, app }) => {
  await app.open("gear");
  await expect(bar(page).getByRole("button", { name: "Radio: Connected" })).toBeVisible();
  await expect(bar(page).getByRole("button", { name: "DJI: Working" })).toBeVisible();
  await expect(bar(page).getByRole("button", { name: "DVR card: Needs attention" })).toBeVisible();

  await bar(page).getByRole("button", { name: "Radio: Connected" }).click();
  const radio = page.getByRole("region", { name: "Field radio" });
  await expect(radio.getByRole("heading", { name: "Field radio" })).toBeVisible();
  await expect(radio).toContainText("tx16s");
  await expect(radio).toContainText("/Volumes/RADIO");
  await expect(radio).toContainText("Last backup");
});

test("a device QuadCam does not know gets a name and an aircraft", async ({ page, app }) => {
  await app.open("gear");
  await bar(page).getByRole("button", { name: "DVR card: Needs attention" }).click();
  const pageRegion = page.getByRole("region", { name: "DVR card" });
  await expect(pageRegion).toContainText("Needs attention");
  await pageRegion.getByRole("button", { name: "Save…" }).click();
  const sheet = dialog(page, "Save this DVR card");
  await sheet.getByLabel("Name").fill("Whoop DVR");
  await sheet.getByLabel("Aircraft").selectOption("Whoop");
  await sheet.getByRole("button", { name: "Save" }).click();
  await expect(sheet).toBeHidden();
  expect(await app.method("gear_device_save")).toEqual([{ id: "dvr-5a6b7c8d9e0f1a2b", name: "Whoop DVR", aircraft: "Whoop" }]);
  await expect(page.getByRole("heading", { name: "Whoop DVR" })).toBeVisible();
  await expect(bar(page).getByRole("button", { name: "DVR card: Connected" })).toBeVisible();
});

test("an unmounted card is safe to unplug, then still inserted with a reminder to dismiss", async ({ page, app }) => {
  await app.open("gear");
  await expect(bar(page).getByRole("button", { name: "Radio: Connected" })).toBeVisible();
  // QuadCam finished a job on the radio's card and released it; the reminder is armed and
  // due at once.
  await app.core(`c => {
    c.settings.values.gearCues = { reminder_grace_s: 0 };
    c.gear.reminders = ["disk4"];
    c.plug([c.gearSeed.dvrConnected(), c.gearSeed.gogglesConnected()], [c.gearSeed.radioConnected()]);
  }`);
  await expect(bar(page).getByRole("button", { name: "Radio: Still inserted" })).toBeVisible();
  await bar(page).getByRole("button", { name: "Dismiss the reminder for Field radio" }).click();
  expect(await app.method("gear_dismiss_reminder")).toEqual([{ handle: "disk4" }]);
  await expect(bar(page).getByRole("button", { name: "Radio: Safe to unplug" })).toBeVisible();
  await expect(bar(page).getByRole("button", { name: /Dismiss/ })).toBeHidden();

  // Pulled: it leaves the bar.
  await app.core(`c => c.plug([c.gearSeed.dvrConnected(), c.gearSeed.gogglesConnected()], [])`);
  await expect(bar(page).getByRole("button", { name: /^Radio/ })).toBeHidden();
});

test("a device plugged in shows in the sidebar; Forget removes a saved one", async ({ page, app }) => {
  await app.open();
  await expect(page.getByRole("article").first()).toBeVisible();
  await app.core(`c => c.plug([c.gearSeed.radioConnected()])`);
  await side(page).getByRole("button", { name: /^Field radio/ }).click();
  await expect(page.getByRole("heading", { name: "Field radio" })).toBeVisible();

  await side(page).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  const fc = page.getByRole("region", { name: "Whoop FC" });
  await expect(fc).toContainText("Not connected");
  await expect(fc).toContainText("4.5.1");
  // An FC's page has the OSD segment; Overview links to it.
  await fc.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "OSD" }).click();
  await expect(fc.getByRole("group", { name: "Sections" }).getByRole("button", { name: "OSD" })).toHaveAttribute("aria-pressed", "true");
  await expect(fc.getByRole("button", { name: /Open/ })).toBeVisible();
  await fc.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Overview" }).click();
  await fc.getByRole("button", { name: "Forget…" }).click();
  await dialog(page, "Forget Whoop FC?").getByRole("button", { name: "Forget" }).click();
  expect(await app.method("gear_device_forget")).toEqual([{ id: "fc-0a1b2c3d4e5f6071" }]);
  await expect(page.getByText("This device is not connected or saved.")).toBeVisible();
});

test("Settings > Gear saves the changed Gear keys with Done and shows the version", async ({ page, app }) => {
  await app.open();
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await expect(dlg.getByLabel("Version")).toHaveText("QuadCam 0.6.4Build 0123abcde");
  await dlg.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Gear" }).click();
  await dlg.getByLabel("Back up on connect").uncheck();
  await dlg.getByLabel("Keep recent backups").fill("5");
  await dlg.getByLabel("Pull blackbox: FC").check();
  await dlg.getByLabel("Mute all cues").check();
  await dlg.getByLabel("Quiet hours for speech and sound").check();
  await dlg.getByLabel("From").fill("21:30");
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect(dlg).toBeHidden();
  const writes = (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values);
  const w = writes.at(-1)!;
  expect(w.gearAutoBackup).toBe(false);
  expect(w.gearKeepRecent).toBe(5);
  expect(w.gearOnConnect).toMatchObject({ fc: ["backup", "blackbox"], radio: ["backup"] });
  expect(w.gearCues).toMatchObject({ mute: true, quiet_hours: { start: "21:30", end: "07:00" }, voice_source: "macos" });
  expect(w.gearKeepWeeks).toBeUndefined();

  // Reopened, the pane shows what was saved.
  await page.getByRole("button", { name: "Settings" }).click();
  await dlg.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Gear" }).click();
  await expect(dlg.getByLabel("Back up on connect")).not.toBeChecked();
  await expect(dlg.getByLabel("Mute all cues")).toBeChecked();
});

test("an FC's background reads can be paused and resumed from its page", async ({ page, app }) => {
  await app.open("gear");
  await app.core(`c => {
    const fc = { id: c.gearSeed.FC.id, kind: "fc", link: { kind: "serial", port: "/dev/cu.usbmodem0", vid: 0x0483, pid: 0x5740, product: null }, identity: {} };
    c.plug([fc, c.gearSeed.radioConnected()], []);
  }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Connected/ }).click();
  await page.getByRole("button", { name: /Whoop FC/ }).first().click();
  const fc = page.getByRole("region", { name: "Whoop FC" });
  await expect(fc).toContainText("Background reads");
  await fc.getByRole("button", { name: "Pause reads" }).click();
  expect(await app.method("gear_poll_pause")).toEqual([{ port: "/dev/cu.usbmodem0", paused: true }]);
  await expect(fc.getByRole("button", { name: "Resume reads" })).toBeVisible();
  await fc.getByRole("button", { name: "Resume reads" }).click();
  await expect(fc.getByRole("button", { name: "Pause reads" })).toBeVisible();
});
