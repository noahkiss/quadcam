// The rate editor and the sim sync on the mock core (WP8, sync half): edit a profile with the
// curve redrawing, stage it and see the apply sheet; convert a profile to another rate model
// with the fit; write the quad's rates into a sim profile through the apply sheet, with the
// refusal while the game runs; an agent's sim sync request; and axe on the editor.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const FC_ID = "fc-0a1b2c3d4e5f6071";
const PORT = "/dev/cu.usbmodemFAKE1";

/** The saved FC's page on its Rates segment; with `plugged`, the FC is on USB so an apply can pass. */
async function openRates(app: AppFixture, page: Page, plugged = false) {
  await app.open();
  if (plugged) {
    await app.core(`c => c.plug([{ id: "${FC_ID}", kind: "fc", link: { kind: "serial", port: "${PORT}", vid: 1155, pid: 22336, serial_number: null, manufacturer: null, product: null }, identity: { board: "BETAFPVG473" } }])`);
  }
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Rates" }).click();
}

const profiles = (page: Page) => page.getByRole("group", { name: "Rate profile" });

test("editing a profile redraws the curve, stages the values and opens the apply sheet", async ({ app, page }) => {
  await openRates(app, page, true);
  await expect(page.getByRole("img", { name: /Roll: 152°\/s per full stick at the centre, 907°\/s at full stick/ })).toBeVisible();
  await page.getByRole("button", { name: "Edit profile" }).click();
  const editor = page.getByRole("region", { name: "Edit rate profile" });
  await expect(editor.getByRole("button", { name: /^Stage/ })).toBeDisabled();

  // The curve follows the number: RC rate 127 to 150 raises the maximum.
  await editor.getByLabel("Roll RC rate").fill("150");
  await expect(page.getByRole("img", { name: /Roll: .*°\/s at full stick\. Before the edit: 907°\/s at full stick/ })).toBeVisible();
  await expect(page.getByRole("img", { name: /Roll: 1[0-9]{2}°\/s per full stick at the centre, 1[0-9]{3}°\/s at full stick/ })).toBeVisible();
  const drawn = (await app.method("gear_rates_preview")).at(-1) as { profile: { axes: { rc_rate: number }[] }; to: string | null };
  expect(drawn.profile.axes[0].rc_rate).toBe(150);
  expect(drawn.to).toBeNull();

  await editor.getByLabel("Throttle mid").fill("40");
  await editor.getByRole("button", { name: "Stage 2 changes" }).click();
  await expect(editor).toContainText("Staged: Rate profile 0 FREE: 2 values. Nothing is written until you apply it.");
  const staged = (await app.method("gear_change_stage")).at(-1) as { device: string; edits: { name: string; value: string; section: { kind: string; index: number } }[] };
  expect(staged.edits).toEqual([
    { kind: "fc_set", section: { kind: "rate_profile", index: 0 }, name: "roll_rc_rate", value: "150" },
    { kind: "fc_set", section: { kind: "rate_profile", index: 0 }, name: "thr_mid", value: "40" },
  ]);

  // The apply sheet shows the lines; nothing was written before the click.
  await editor.getByRole("button", { name: "Review and apply…" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to Whoop FC" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("set roll_rc_rate = 150");
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("set thr_mid = 40");
  expect(await app.method("gear_apply")).toEqual([]);
  expect(await app.calls("gear_apply_click")).toEqual([]);
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
});

test("converting to another rate model fits the curve and shows the gap", async ({ app, page }) => {
  await openRates(app, page);
  await profiles(page).getByRole("button", { name: "1 RACE" }).click();
  await page.getByRole("button", { name: "Edit profile" }).click();
  const editor = page.getByRole("region", { name: "Edit rate profile" });
  await editor.getByLabel("Convert to").selectOption("betaflight");
  await expect(editor).toContainText("Fitted onto Betaflight. Largest gap from the old curve:");
  await expect(editor.getByLabel("Rates type")).toHaveValue("betaflight");
  // The three numbers are Betaflight's now: whole numbers inside its range.
  const rc = Number(await editor.getByLabel("Roll RC rate").inputValue());
  expect(Number.isInteger(rc) && rc >= 1 && rc <= 255).toBe(true);
  expect((await app.method("gear_rates_preview")).some((c) => c.to === "betaflight")).toBe(true);
  await editor.getByRole("button", { name: /^Stage/ }).click();
  const staged = (await app.method("gear_change_stage")).at(-1) as { edits: { name: string; value: string }[] };
  expect(staged.edits.find((e) => e.name === "rates_type")?.value).toBe("BETAFLIGHT");
  expect(staged.edits.some((e) => e.name === "roll_rc_rate")).toBe(true);
  await expect(editor).toContainText("Staged: Rate profile 1 RACE: Betaflight rates");
});

test("a dump file is not editable", async ({ app, page }) => {
  await openRates(app, page);
  await expect(page.getByRole("button", { name: "Edit profile" })).toBeVisible();
  await app.core(`c => { c.dialogAnswers.push(["/Users/pilot/bench/quad.dump_all.txt"]); }`);
  await page.getByRole("button", { name: "Open dump…" }).click();
  await expect(page.getByText("quad.dump_all.txt")).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit profile" })).toHaveCount(0);
});

test("syncing a sim profile plans, backs up through the sheet and reads as matching after", async ({ app, page }) => {
  await app.open();
  await app.core(`c => { c.simSync.running = []; }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Rates" }).click();
  await profiles(page).getByRole("button", { name: "1 RACE" }).click();
  const sims = page.getByRole("region", { name: "Sims" });
  const row = (name: string) => sims.getByRole("listitem").filter({ has: page.getByText(name, { exact: true }) }).first();
  await expect(row("Uncrashed")).toContainText("Differs from the quad");

  await sims.getByRole("button", { name: "Sync 1 RACE into Uncrashed FREE" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to sims" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("Uncrashed");
  await expect(sheet.getByRole("region", { name: "Checks" })).toContainText("Sim closed (Uncrashed)");
  // The quad is on Actual rates: the sheet says it was fitted.
  await expect(sheet.getByRole("region", { name: "Warnings" })).toContainText("uses the actual model");
  expect(await app.method("gear_sim_sync")).toEqual([]);
  expect((await app.method("gear_sim_sync_plan")).at(-1)).toMatchObject({ sims: [{ sim: "uncrashed", profile: "FREE" }], profile: 1 });

  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Back up Uncrashed");
  expect((await app.method("gear_sim_sync_click")).at(-1)).toMatchObject({ confirm: true, profile: 1 });
  await sheet.getByRole("button", { name: "Done" }).click();

  // The segment reads the sim again: the profile matches and Sync is off.
  await expect(row("Uncrashed")).toContainText("same as the quad");
  await expect(sims.getByRole("button", { name: "Sync 1 RACE into Uncrashed FREE" })).toBeDisabled();
});

test("a synced sim can be restored from its backup through the sheet", async ({ app, page }) => {
  await app.open();
  await app.core(`c => { c.simSync.running = []; }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Rates" }).click();
  await profiles(page).getByRole("button", { name: "1 RACE" }).click();
  const sims = page.getByRole("region", { name: "Sims" });
  // No backup yet: no Restore.
  await expect(sims.getByRole("button", { name: /^Restore Uncrashed/ })).toHaveCount(0);

  await sims.getByRole("button", { name: "Sync 1 RACE into Uncrashed FREE" }).click();
  const sync = page.getByRole("dialog", { name: "Apply to sims" });
  await sync.getByRole("button", { name: "Apply" }).click();
  await expect(sync.getByRole("region", { name: "Result" })).toContainText("Verified");
  await sync.getByRole("button", { name: "Done" }).click();

  await expect(sims.getByText(/^Backed up /).first()).toBeVisible();
  await sims.getByRole("button", { name: "Restore Uncrashed from its backup" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to sims" });
  await expect(sheet.getByRole("heading", { name: "Restore a sim backup" })).toBeVisible();
  await expect(sheet.getByRole("region", { name: "Checks" })).toContainText("Backup found");
  await expect(sheet.getByRole("region", { name: "Warnings" })).toContainText("Changes made in the game");
  expect((await app.method("gear_sim_restore_plan")).at(-1)).toMatchObject({ sim: "uncrashed" });
  expect(await app.method("gear_sim_restore")).toEqual([]);

  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  expect((await app.method("gear_sim_restore_click")).at(-1)).toMatchObject({ sim: "uncrashed", confirm: true });
  await sheet.getByRole("button", { name: "Done" }).click();
  // The file is back as it was: the profile differs from the quad again, so Sync is on.
  await expect(sims.getByRole("button", { name: "Sync 1 RACE into Uncrashed FREE" })).toBeEnabled();
});

test("a sim that runs cannot be synced, even if it starts after the plan", async ({ app, page }) => {
  await app.open();
  await app.core(`c => { c.simSync.running = []; }`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Whoop FC/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Rates" }).click();
  await profiles(page).getByRole("button", { name: "1 RACE" }).click();
  const sims = page.getByRole("region", { name: "Sims" });
  await sims.getByRole("button", { name: "Sync 1 RACE into The Zone Profile 0" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to sims" });
  await expect(sheet.getByRole("region", { name: "Warnings" })).toContainText("Unverified: QuadCam read The Zone's file shape");
  await app.core(`c => { c.simSync.running = ["The Zone"]; }`);
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet).toContainText("Quit The Zone first.");
  expect(await app.core<string[]>(`c => c.simSync.written`)).toEqual([]);
});

test("Liftoff runs in the recording: its Sync button is off and its plan refuses", async ({ app, page }) => {
  await openRates(app, page);
  await profiles(page).getByRole("button", { name: "1 RACE" }).click();
  await expect(page.getByRole("region", { name: "Sims" }).getByRole("button", { name: "Sync 1 RACE into Liftoff Race" })).toBeDisabled();
  const plan = await app.core<{ checks: { name: string; ok: boolean; refusal?: { code: string; reason: string } }[]; digest: string }>(
    `c => c.dispatch("gear_sim_sync_plan", { sims: [{ sim: "liftoff", file: null, profile: "Race" }], paths: [], device: null, backup: null, profile: 1 })`,
  );
  expect(plan.digest).toBe("");
  expect(plan.checks.find((k) => !k.ok)?.refusal).toEqual({ code: "sim_running", reason: "Quit Liftoff first." });
});

test("an agent's sim sync request waits for the person's click", async ({ app, page }) => {
  await app.open();
  await app.core(`c => { c.simSync.running = []; }`);
  const plan = await app.core(`c => c.dispatch("gear_sim_sync_plan", { sims: [{ sim: "uncrashed", file: null, profile: "FREE" }], paths: [], device: null, backup: null, profile: 1 })`);
  const change = { id: "sim-sync", device: "sims", title: "Sync sims", status: "ready", edits: [], base_backup: "", editor: "agent", note: "", order: 0, history: [] };
  await page.evaluate(([c, p]) => window.__qc!.emit("agent-apply-request", { id: 11, change: c, plan: p }), [change, plan] as [unknown, unknown]);
  const sheet = page.getByRole("dialog", { name: "Apply to sims" });
  await expect(sheet.getByText("asked to apply this change.", { exact: false })).toBeVisible();
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("Uncrashed");
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await app.calls("answer_apply_request")).map((c) => c.args)).toEqual([{ id: 11, approve: false }]);
});

test("the editor and the sim sheet pass axe", async ({ app, page }) => {
  await openRates(app, page);
  await page.getByRole("button", { name: "Edit profile" }).click();
  await expect(page.getByRole("region", { name: "Edit rate profile" })).toBeVisible();
  const r = await new AxeBuilder({ page }).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" | ")}`)).toEqual([]);
});
