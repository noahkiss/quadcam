// Staged changes and the FC apply sheet on the mock core: stage a raw `set` from the FC's
// Changes segment, see the plug-in bar, review the diff and checks, apply, and see the
// verified result; a refused line; an agent's request waiting for the click; axe.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const FC_ID = "fc-0a1b2c3d4e5f6071";
const PORT = "/dev/cu.usbmodemFAKE1";

async function openChanges(app: AppFixture, page: Page, plugged = true) {
  await app.open();
  if (plugged) {
    await app.core(`c => c.plug([{ id: "${FC_ID}", kind: "fc", link: { kind: "serial", port: "${PORT}", vid: 1155, pid: 22336, serial_number: null, manufacturer: null, product: null }, identity: { board: "BETAFPVG473" } }])`);
  }
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Changes" }).click();
}

async function stageSetting(page: Page, name: string, value: string) {
  await page.getByRole("button", { name: "Edit setting…" }).click();
  const d = page.getByRole("dialog", { name: "Edit setting" });
  await d.getByLabel("Setting").fill(name);
  await d.getByLabel("Value").fill(value);
  await d.getByRole("button", { name: "Stage" }).click();
}

test("stage a setting, review it, apply it and see it verified", async ({ app, page }) => {
  await openChanges(app, page);
  await stageSetting(page, "osd_cap_alarm", "1500");
  const list = page.getByRole("list", { name: "Staged changes" });
  await expect(list).toContainText("Set osd_cap_alarm = 1500");
  expect((await app.method("gear_change_stage")).at(-1)).toMatchObject({ device: FC_ID, edits: [{ kind: "fc_set", name: "osd_cap_alarm", value: "1500" }] });

  // The Overview shows the plug-in bar.
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Overview" }).click();
  await expect(page.getByRole("status").filter({ hasText: "1 change ready" })).toBeVisible();
  await page.getByRole("button", { name: "Review…" }).click();

  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("set osd_cap_alarm = 1500");
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("set osd_cap_alarm = 2200");
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("One FC plugged in");
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeEnabled();
  expect(await new AxeBuilder({ page }).analyze().then((r) => r.violations)).toEqual([]);

  // Return does not press Apply.
  await page.keyboard.press("Enter");
  expect(await app.method("gear_apply_click")).toHaveLength(0);

  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await expect(sheet.getByRole("list", { name: "Steps" })).toContainText("Verify");
  expect((await app.method("gear_apply_click")).at(-1)).toMatchObject({ confirm: true, digest: `digest-${(await app.method("gear_apply_click")).at(-1)?.id}` });
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "change ready" })).toHaveCount(0);
});

test("a setting the FC does not have is refused when staged", async ({ app, page }) => {
  await openChanges(app, page);
  await stageSetting(page, "no_such_setting", "1");
  await expect(page.getByRole("alert")).toContainText("`no_such_setting` is not a setting on this FC.");
});

test("with no FC plugged in the sheet shows the failed check and Apply stays off", async ({ app, page }) => {
  await openChanges(app, page, false);
  await stageSetting(page, "osd_ah_pos", "4000");
  await page.getByRole("button", { name: "Review…" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("No FC is plugged in");
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeDisabled();
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect(sheet).toHaveCount(0);
});

test("a refused line fails the apply and offers the restore", async ({ app, page }) => {
  await openChanges(app, page);
  await app.core(`c => { c.gear.changeStore.failNext = "set osd_ah_pos = 4000"; }`);
  await stageSetting(page, "osd_ah_pos", "4000");
  await page.getByRole("button", { name: "Review…" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("The FC refused `set osd_ah_pos = 4000`");
  await expect(sheet.getByRole("button", { name: "Restore backup" })).toBeVisible();
});

test("an agent's apply request waits for the person's click", async ({ app, page }) => {
  await openChanges(app, page);
  await stageSetting(page, "osd_cap_alarm", "1500");
  const change = (await app.core(`c => c.gear.changeStore.changes[0]`)) as { id: string; title: string };
  const plan = { change: change.id, device: { board: "BETAFPVG473" }, checks: [{ name: "One FC plugged in", ok: true }], diff: [{ kind: "lines", label: change.title, lines: [{ op: "add", text: "set osd_cap_alarm = 1500" }] }], digest: "d" };
  await page.evaluate(([c, p]) => window.__qc!.emit("agent-apply-request", { id: 9, change: c, plan: p }), [change, plan] as [unknown, unknown]);
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await expect(sheet.getByText("An agent asked to apply this change.", { exact: false })).toBeVisible();
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await app.calls("answer_apply_request")).map((c) => c.args)).toEqual([{ id: 9, approve: false }]);
});
