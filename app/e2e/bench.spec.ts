// The Bench page and copying settings between quads, on the mock core: the queue per
// device with its plug-in prompt and Next session, statuses, a radio card apply (the card
// unmounts after it), Try with Keep and Revert, Read first, a card apply that fails its
// read-back and puts the files back, Mount and Done, and the copy dialog with its checks.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const RADIO_LINK = `{ kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }`;
const radio = `{ id: "${RADIO_ID}", kind: "radio", link: ${RADIO_LINK}, identity: { board: "tx16s" } }`;

async function openBench(page: Page) {
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Bench/ }).click();
  await expect(page.getByRole("heading", { name: "Bench" })).toBeVisible();
}

/** Stages a radio card change through the core, as the CLI or an agent would. */
async function stageCard(app: AppFixture, title: string, value: string, status: "ready" | "try" | "read_first" = "ready") {
  const id = await app.core<string>(
    `c => c.dispatch("gear_change_stage", { device: "${RADIO_ID}", title: "${title}", edits: [{ kind: "radio", ops: [{ op: "set_scalar", key: "contrast", value: "${value}" }] }], note: null, editor: null, draft: false }).id`,
  );
  if (status !== "ready") await app.core(`c => c.dispatch("gear_change_update", { id: "${id}", status: "${status}", title: null, edits: null, note: null, order: null })`);
  return id;
}

test("the Bench lists a device's changes, names the next session and asks to plug the radio in", async ({ app, page }) => {
  await app.open();
  await openBench(page);
  await expect(page.getByText("Nothing staged.")).toBeVisible();

  await stageCard(app, "Raise contrast", "25");
  await stageCard(app, "Lower contrast", "15", "try");
  await openBench(page);
  const group = page.getByRole("region", { name: "Field radio" });
  await expect(group.getByText("Not connected")).toBeVisible();
  await expect(group.getByText("Next session:")).toContainText("Raise contrast");
  await expect(group.getByRole("status")).toContainText("Plug in Field radio in USB Storage mode to apply 2 changes.");
  await expect(group.getByRole("button", { name: /^Review/ }).first()).toBeDisabled();
  expect(await new AxeBuilder({ page }).analyze().then((r) => r.violations)).toEqual([]);

  // Plugged in: the prompt goes and Review opens.
  await app.core(`c => c.plug([${radio}])`);
  await expect(group.getByText("Plugged in")).toBeVisible();
  await expect(group.getByRole("status")).toHaveCount(0);
  await expect(group.getByRole("button", { name: "Review 2 changes…" })).toBeEnabled();
});

test("a Try change on a radio card applies, unmounts the card, then waits for Keep", async ({ app, page }) => {
  await app.open();
  await app.core(`c => c.plug([${radio}])`);
  const id = await stageCard(app, "Try more contrast", "27", "try");
  await openBench(page);
  const group = page.getByRole("region", { name: "Field radio" });
  await expect(group.getByLabel("Status of Try more contrast")).toHaveValue("try");

  await group.getByRole("button", { name: "Review…" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("contrast: 27");
  await expect(sheet.getByRole("list", { name: "Checks" })).toContainText("Card check");
  await expect(sheet).toContainText("unmounts it");
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await expect(sheet).toContainText("This was a Try change.");
  await sheet.getByRole("button", { name: "Done" }).click();

  // The card is unmounted and still in; the change waits for the decision.
  await expect(group.getByText("Unmounted, still in")).toBeVisible();
  await expect(group.getByRole("list", { name: "Applied changes for Field radio" })).toContainText("Try more contrast");
  await group.getByRole("button", { name: "Keep" }).click();
  expect((await app.method("gear_change_keep")).at(-1)).toMatchObject({ id });
  await expect(page.getByText("Nothing staged.")).toBeVisible();
  await page.getByRole("button", { name: "History" }).click();
  await expect(page.getByRole("list", { name: "Applied changes" })).toContainText("Verified");
});

test("Revert stages a restore of the card files and marks the change Reverted when it verifies", async ({ app, page }) => {
  await app.open();
  await app.core(`c => c.plug([${radio}])`);
  await stageCard(app, "Try contrast", "28", "try");
  await openBench(page);
  const group = page.getByRole("region", { name: "Field radio" });
  await group.getByRole("button", { name: "Review…" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  await sheet.getByRole("button", { name: "Apply" }).click();
  await sheet.getByRole("button", { name: "Done" }).click();

  await group.getByRole("button", { name: "Revert…" }).click();
  const revert = (await app.method("gear_change_revert")).at(-1);
  expect(revert).toBeTruthy();
  const sheet2 = page.getByRole("dialog", { name: "Apply to Field radio" });
  await expect(sheet2).toContainText("Revert: Try contrast");
  await expect(sheet2.getByRole("region", { name: "Changes" })).toContainText("RADIO/radio.yml");
  await sheet2.getByRole("button", { name: "Apply" }).click();
  await expect(sheet2.getByRole("region", { name: "Result" })).toContainText("Verified");
  await sheet2.getByRole("button", { name: "Done" }).click();
  await page.getByRole("button", { name: "History" }).click();
  await expect(page.getByRole("list", { name: "Applied changes" })).toContainText("Reverted");
});

test("a Read first change does not apply until it is marked Ready", async ({ app, page }) => {
  await app.open();
  await app.core(`c => c.plug([${radio}])`);
  await stageCard(app, "Check the value", "30", "read_first");
  await openBench(page);
  const group = page.getByRole("region", { name: "Field radio" });
  await expect(group).toContainText("Read the real value on the device");
  const item = group.getByRole("list", { name: "Staged changes for Field radio" });
  await expect(item.getByRole("button", { name: "Review…" })).toBeDisabled();
  await group.getByLabel("Status of Check the value").selectOption("ready");
  await expect(item.getByRole("button", { name: "Review…" })).toBeEnabled();
  expect((await app.method("gear_change_update")).at(-1)).toMatchObject({ status: "ready" });
});

test("a card apply that reads back wrong puts the files back and says so", async ({ app, page }) => {
  await app.open();
  await app.core(`c => { c.plug([${radio}]); c.gear.changeStore.failCard = true; }`);
  await stageCard(app, "Doomed contrast", "29");
  await openBench(page);
  await page.getByRole("region", { name: "Field radio" }).getByRole("button", { name: "Review…" }).first().click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  await sheet.getByRole("button", { name: "Apply" }).click();
  const result = sheet.getByRole("region", { name: "Result" });
  await expect(result).toContainText("The card is as it was");
  await expect(result.getByRole("list", { name: "Steps" })).toContainText("Roll back");
  await expect(sheet.getByRole("button", { name: "Restore backup" })).toHaveCount(0);
});

test("Mount mounts an unmounted card to browse and Done unmounts it", async ({ app, page }) => {
  await app.open();
  await app.core(`c => c.plug([], [${radio}])`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Field radio/ }).click();
  await page.getByRole("button", { name: "Mount" }).click();
  expect((await app.method("gear_card_mount")).at(-1)).toMatchObject({ device: RADIO_ID });
  await expect(page.getByText(/Mounted until/)).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  expect((await app.method("gear_card_unmount")).at(-1)).toMatchObject({ device: RADIO_ID });
  await expect(page.getByText(/Mounted until/)).toHaveCount(0);
});

async function secondFc(app: AppFixture, version: string) {
  await app.core(`c => {
    c.gear.devices.push({ id: "fc-5a5a5a5a5a5a5a5a", kind: "fc", name: "Five-inch FC", aircraft: null, identity: { board: "STM32F411", firmware: "Betaflight", version: "${version}" }, last_seen: null, last_backup: "fc-5a5a5a5a5a5a5a5a/2026-10-05T100000-connect" });
    c.gear.changeStore.dumps["fc-5a5a5a5a5a5a5a5a"] = { osd_cap_alarm: "1800", p_roll: "50" };
    c.emit("gear-changed");
  }`);
}

test("copy settings: pick the quads and the parts, read the diff, stage one change", async ({ app, page }) => {
  await app.open();
  await secondFc(app, "4.5.1");
  await openBench(page);
  await page.getByRole("button", { name: "Copy settings…" }).click();
  const d = page.getByRole("dialog", { name: "Copy settings" });
  await expect(d.getByLabel("Copy from")).toHaveValue("fc-5a5a5a5a5a5a5a5a");
  await expect(d.getByRole("button", { name: "Stage" })).toBeDisabled();
  await d.getByLabel("OSD").check();
  await expect(d.getByRole("list", { name: "Checks" })).toContainText("Same release");
  const diff = d.getByRole("region", { name: "Changes for Whoop FC" });
  await expect(diff).toContainText("set osd_cap_alarm = 1800");
  await expect(diff).toContainText("set osd_cap_alarm = 2200");
  const v = await new AxeBuilder({ page }).analyze().then((r) => r.violations.map((x) => `${x.id}: ${x.nodes.map((n) => n.html.slice(0, 120)).join(' | ')}`));
  expect(v).toEqual([]);
  await d.getByRole("button", { name: "Stage" }).click();
  const staged = (await app.method("gear_copy_stage")).at(-1);
  expect(staged).toMatchObject({ from: "fc-5a5a5a5a5a5a5a5a", to: "fc-0a1b2c3d4e5f6071", parts: ["osd"] });
  const group = page.getByRole("region", { name: "Whoop FC" });
  await expect(group.getByRole("list", { name: "Staged changes for Whoop FC" })).toContainText("Copy OSD from Five-inch FC");
  await expect(group).toContainText("set osd_cap_alarm = 1800");
});

test("copy settings refuses another release and says why", async ({ app, page }) => {
  await app.open();
  await secondFc(app, "2025.12.5");
  await openBench(page);
  await page.getByRole("button", { name: "Copy settings…" }).click();
  const d = page.getByRole("dialog", { name: "Copy settings" });
  await d.getByLabel("OSD").check();
  await expect(d.getByRole("list", { name: "Checks" })).toContainText("settings change between releases");
  await expect(d.getByRole("button", { name: "Stage" })).toBeDisabled();
});
