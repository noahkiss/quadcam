// Staged changes and the FC apply sheet on the mock core: stage a raw `set` from the FC's
// Changes segment, see the plug-in bar, review the diff and checks, apply, and see the
// verified result; a refused line; an agent's request waiting for the click, approved (its
// result in the sheet), arriving while the person's apply runs (it waits), and an ExpressLRS
// flash; axe.
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

  // The click carries the digest of the plan the sheet shows, as the core plans it.
  const id = String((await app.method("gear_apply_plan")).at(-1)?.id);
  const planned = await app.core<{ digest: string }>(`c => c.handle("gear_apply_plan", { params: { id: "${id}", port: null } })`);
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await expect(sheet.getByRole("list", { name: "Steps" })).toContainText("Verify");
  expect((await app.method("gear_apply_click")).at(-1)).toMatchObject({ id, confirm: true, digest: planned.digest });
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
  await expect(sheet.getByText("asked to apply this change.", { exact: false })).toBeVisible();
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await app.calls("answer_apply_request")).map((c) => c.args)).toEqual([{ id: 9, approve: false }]);
});

/** An agent's request to apply `change`, as the core emits it. */
async function agentRequest(page: Page, id: number, change: unknown, title: string) {
  const c = change as { id: string };
  const plan = { change: c.id, device: { board: "BETAFPVG473" }, checks: [{ name: "One FC plugged in", ok: true }], diff: [{ kind: "lines", label: title, lines: [{ op: "add", text: title }] }], digest: "d" };
  await page.evaluate(([i, ch, p]) => window.__qc!.emit("agent-apply-request", { id: i, change: ch, plan: p }), [id, change, plan] as [number, unknown, unknown]);
}

test("an approved agent request shows its progress, its result and Restore backup", async ({ app, page }) => {
  await openChanges(app, page);
  await stageSetting(page, "osd_cap_alarm", "1500");
  const change = (await app.core(`c => c.gear.changeStore.changes[0]`)) as { id: string; title: string };
  await agentRequest(page, 9, change, change.title);
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect.poll(async () => (await app.calls("answer_apply_request")).map((c) => c.args)).toEqual([{ id: 9, approve: true }]);
  // The core closes the request once answered; the sheet stays and shows the write.
  await page.evaluate(() => window.__qc!.emit("agent-apply-closed", 9));
  await expect(sheet.getByRole("status").filter({ hasText: "Applying" })).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeDisabled();
  const report = { change: change.id, device: FC_ID, status: "failed", steps: [{ name: "Backup", state: "done" }, { name: "Save", state: "failed", detail: "the FC did not answer" }], backup: "bk-1", after_backup: null, sent: [], failed_line: null, verify: [], saved: false, message: "The FC did not answer.", at: "2026-10-10T12:00:00Z" };
  await page.evaluate((r) => window.__qc!.emit("agent-apply-result", { id: 9, report: r, error: null }), report);
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("The FC did not answer.");
  await expect(sheet.getByRole("button", { name: "Restore backup" })).toBeVisible();
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(sheet).toHaveCount(0);
});

test("an agent request while the person's apply runs waits for that sheet to close", async ({ app, page }) => {
  await openChanges(app, page);
  await stageSetting(page, "osd_cap_alarm", "1500");
  await page.getByRole("button", { name: "Review…" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await page.evaluate(() => window.__qc!.hold("gear_apply_click"));
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("status").filter({ hasText: "Applying" })).toBeVisible();

  const other = { id: "chg-agent", device: "radio-1", title: "Agent card change", status: "ready", edits: [], base_backup: "", editor: "agent", note: "", order: 1, history: [] };
  await agentRequest(page, 12, other, "Agent card change");
  // The person's sheet keeps its title, its progress and its change.
  await expect(sheet.getByText("Another apply request waits.", { exact: false })).toBeVisible();
  await expect(sheet.getByRole("status").filter({ hasText: "Applying" })).toBeVisible();
  await expect(sheet).toContainText("Set osd_cap_alarm = 1500");
  await expect(sheet).not.toContainText("Agent card change");

  // The person's write ends: its own result lands in its own sheet.
  await page.evaluate(() => window.__qc!.release("gear_apply_click"));
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await expect(sheet).not.toContainText("Agent card change");
  expect(await app.calls("answer_apply_request")).toHaveLength(0);
  await sheet.getByRole("button", { name: "Done" }).click();

  // Then the agent's request opens.
  const ask = page.getByRole("dialog", { name: /^Apply to/ });
  await expect(ask.getByText("asked to apply this change.", { exact: false })).toBeVisible();
  await expect(ask).toContainText("Agent card change");
  await ask.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await app.calls("answer_apply_request")).map((c) => c.args)).toEqual([{ id: 12, approve: false }]);
});

test("an agent's ExpressLRS flash request says it flashes", async ({ app, page }) => {
  await openChanges(app, page);
  const change = { id: "elrs-flash", device: "elrs-0001", title: "Flash ExpressLRS", status: "ready", edits: [], base_backup: "", editor: "user", note: "", order: 0, history: [] };
  await agentRequest(page, 14, change, "ExpressLRS 3.5.3");
  const sheet = page.getByRole("dialog", { name: "Flash ExpressLRS device" });
  await expect(sheet).toContainText("reads the chip's flash with esptool and keeps it as a backup");
  await expect(sheet).not.toContainText("writes each option");
  expect(await new AxeBuilder({ page }).analyze().then((r) => r.violations)).toEqual([]);
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect.poll(async () => (await app.calls("answer_apply_request")).map((c) => c.args)).toEqual([{ id: 14, approve: true }]);
  await expect(sheet.getByRole("status").filter({ hasText: "Flashing" })).toBeVisible();
  await page.evaluate(() => window.__qc!.emit("agent-apply-result", { id: 14, report: null, error: "The device did not enter its bootloader." }));
  await expect(sheet.getByRole("alert")).toContainText("did not enter its bootloader");
  await expect(sheet.getByRole("button", { name: "Apply" })).toBeDisabled();
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect(sheet).toHaveCount(0);
  expect(await app.calls("answer_apply_request")).toHaveLength(1);
});

test("Review… is off for a Draft or Read first change", async ({ app, page }) => {
  await openChanges(app, page);
  await stageSetting(page, "osd_cap_alarm", "1500");
  const row = page.getByRole("list", { name: "Staged changes" }).getByRole("listitem");
  await expect(row.getByRole("button", { name: "Review…" })).toBeEnabled();
  for (const status of ["draft", "read_first"]) {
    await app.core(`c => { c.gear.changeStore.changes[0].status = "${status}"; }`);
    await page.evaluate(() => window.__qc!.emit("gear-changed"));
    await expect(row.getByRole("button", { name: "Review…" })).toBeDisabled();
  }
});
