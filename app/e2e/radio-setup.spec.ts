// The radio's Setup segment on the mock core: one row per job with its channel, control,
// direction and what each position sends; fitted parts per slot; a problem shown at its job
// and the setup left unsaved; Review staging the one "Radio setup" change into the apply
// sheet; Copy from another radio; Learn from the radio in USB Joystick mode; axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const RADIO_LINK = `{ kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }`;
const radio = `{ id: "${RADIO_ID}", kind: "radio", link: ${RADIO_LINK}, identity: { board: "gx12" } }`;
const POCKET = `{ id: "radio-0a0b0c0d0e0f1011", kind: "radio", name: "Old radio", aircraft: null, identity: { board: "pocket", firmware: "EdgeTX", version: "2.12.4" }, last_seen: "2026-10-01T10:00:00Z", last_backup: "radio-0a0b0c0d0e0f1011/2026-10-01T100000-manual" }`;

/** CH1-CH8 at rest as raw axes: sticks centred, throttle low, every switch up. */
const REST = [1024, 1024, 0, 1024, 0, 0, 0, 0];

/** The plugged-in GX12's Setup segment. */
async function openSetup(app: AppFixture, page: Page) {
  await app.open();
  await app.core(`c => c.plug([${radio}])`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("table", { name: "Devices" }).getByRole("button", { name: "Field radio", exact: true }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Setup" }).click();
  await expect(page.getByRole("table", { name: "Jobs" })).toBeVisible();
}

const jobs = (page: Page) => page.getByRole("table", { name: "Jobs" });
const jobRow = (page: Page, label: string) => jobs(page).getByRole("row").filter({ has: page.getByRole("rowheader", { name: label, exact: true }) });
const lastEdit = async (app: AppFixture) => (await app.method("gear_radio_setup_edit")).at(-1) as { setup?: { assignments: { job: string; control: string; high: number | null }[]; fitted: { control: string; radio_type: string }[] } } | undefined;

test("each job shows its channel, control, direction and what it sends", async ({ app, page }) => {
  await openSetup(app, page);
  await expect(page.getByText("GX12 · From the card · The board's default setup")).toBeVisible();
  const arm = jobRow(page, "Arm");
  await expect(arm.getByRole("cell", { name: "CH5" })).toBeVisible();
  await expect(arm.getByLabel("Arm: control")).toHaveValue("SA");
  await expect(arm.getByLabel("Arm: 2000 µs at").locator("option:checked")).toHaveText("SA down");
  const sends = arm.getByRole("list", { name: "Arm: what it sends" });
  await expect(sends.getByRole("listitem").nth(0)).toHaveText(/SA up\s*1000 µs\s*disarmed/);
  await expect(sends.getByRole("listitem").nth(1)).toHaveText(/SA down\s*2000 µs\s*armed/);
  // A 3-position job lists the middle; a pot follows the control.
  await expect(jobRow(page, "Rates").getByRole("list", { name: "Rates: what it sends" })).toContainText("SC middle");
  await expect(jobRow(page, "VTX power").getByText("Follows the control")).toBeVisible();
  // The turtle's momentary switch, and only controls that fit are offered.
  await expect(jobRow(page, "Turtle").getByLabel("Turtle: 2000 µs at").locator("option:checked")).toHaveText("SH pressed");
  await expect(jobRow(page, "Turtle").getByLabel("Turtle: control").locator("option")).toHaveText(["None", "SH", "SG"]);
  // The fitted controls, the factory part beside what is fitted.
  const slots = page.getByRole("table", { name: "Fitted controls" });
  const sa = slots.getByRole("row").filter({ has: page.getByRole("rowheader", { name: "SA", exact: true }) });
  await expect(sa.locator('td[data-label="Factory part"]')).toHaveText("2-position switch");
  await expect(sa.getByLabel("SA: fitted")).toHaveValue("2POS");
  await expect(page.getByText("1 file on the radio differs from this setup.")).toBeVisible();
});

test("a change saves; a part that does not fit shows at its job and nothing saves", async ({ app, page }) => {
  await openSetup(app, page);
  await jobRow(page, "VTX power").getByLabel("VTX power: control").selectOption("P2");
  await expect.poll(async () => (await lastEdit(app))?.setup?.assignments.find((a) => a.job === "vtx")).toEqual({ job: "vtx", control: "P2", high: null });
  await jobRow(page, "Rates").getByLabel("Rates: 2000 µs at").selectOption("0");
  await expect.poll(async () => (await lastEdit(app))?.setup?.assignments.find((a) => a.job === "rates")).toEqual({ job: "rates", control: "SC", high: 0 });
  await expect(jobRow(page, "Rates").getByRole("list", { name: "Rates: what it sends" }).getByRole("listitem").nth(2)).toHaveText(/SC up\s*2000 µs/);

  // A momentary module in SA, which arms: the problem shows at Arm and nothing saves.
  const edits = (await app.method("gear_radio_setup_edit")).length;
  const slots = page.getByRole("table", { name: "Fitted controls" });
  await slots.getByLabel("SA: fitted").selectOption("TOGGLE");
  const arm = jobRow(page, "Arm");
  await expect(arm.getByRole("alert")).toHaveText("SA cannot do Arm: it is a momentary switch, and Arm takes 2-position switch.");
  await expect(arm.getByLabel("Arm: control")).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByText("Not saved: fix the problems below.")).toBeVisible();
  expect((await app.method("gear_radio_setup_edit")).length).toBe(edits);
  // Arm on no control: the setup checks, and saves with the module recorded.
  await arm.getByLabel("Arm: control").selectOption("");
  await expect.poll(async () => (await lastEdit(app))?.setup?.fitted).toEqual([{ control: "SA", radio_type: "TOGGLE" }]);
  expect((await lastEdit(app))?.setup?.assignments.some((a) => a.job === "arm")).toBe(false);
  await expect(arm.getByRole("alert")).toHaveCount(0);
  // A blanking plug in P1: VTX power moved to P2 already, so it saves.
  await slots.getByLabel("P1: fitted").selectOption("none");
  await expect.poll(async () => (await lastEdit(app))?.setup?.fitted).toEqual([
    { control: "SA", radio_type: "TOGGLE" },
    { control: "P1", radio_type: "none" },
  ]);
});

test("Review stages one Radio setup change and opens the apply sheet", async ({ app, page }) => {
  await openSetup(app, page);
  await page.getByRole("table", { name: "Fitted controls" }).getByLabel("SG: fitted").selectOption("2POS");
  await expect.poll(async () => (await lastEdit(app))?.setup?.fitted).toEqual([{ control: "SG", radio_type: "2POS" }]);
  await page.getByRole("button", { name: "Review…" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  const diff = sheet.getByRole("region", { name: "Changes" });
  await expect(diff).toContainText("RADIO/radio.yml");
  await expect(diff).toContainText("SG: type: 2POS");
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByText(/The setup is staged: 2 files change when you apply it\./)).toBeVisible();
  // A later change keeps the staged change up to date.
  await jobRow(page, "VTX power").getByLabel("VTX power: control").selectOption("SL1");
  await expect.poll(async () => ((await app.method("gear_radio_setup_edit")).at(-1) as { stage?: boolean }).stage).toBe(true);
  await page.getByRole("button", { name: "Discard Staged Setup" }).click();
  await dialog(page, "Discard the staged setup?").getByRole("button", { name: "Discard" }).click();
  await expect(page.getByText(/The setup is staged/)).toHaveCount(0);
});

test("Copy from another radio stages its models first", async ({ app, page }) => {
  await openSetup(app, page);
  await app.core(`c => { c.gear.devices.push(${POCKET}); c.emit("gear-changed"); }`);
  await page.getByRole("button", { name: "Copy From…" }).click();
  const dialog = page.getByRole("dialog", { name: "Copy from another radio" });
  await dialog.getByLabel("Copy from").selectOption({ label: "Old radio" });
  await expect(dialog.getByRole("checkbox", { name: /ALPHA/ })).toBeChecked();
  await dialog.getByRole("checkbox", { name: /BRAVO 2/ }).uncheck();
  await dialog.getByRole("button", { name: "Copy", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect((await app.method("gear_radio_setup_edit")).at(-1)).toMatchObject({ device: RADIO_ID, copy_from: { from: "radio-0a0b0c0d0e0f1011", models: ["model00.yml"], links: [] } });
  await expect(page.getByText("Copies ALPHA to model02.yml from the Old radio (pocket) first.")).toBeVisible();
  await expect(page.getByText(/The setup is staged: .* First it copies ALPHA to model02\.yml from the POCKET\./)).toBeVisible();
});

test("Learn takes the control that moves on the radio", async ({ app, page }) => {
  await openSetup(app, page);
  await app.core(`c => c.radioFrame(${JSON.stringify(REST)})`);
  // Arm: SD moves, but Finder has it.
  await jobRow(page, "Arm").getByRole("button", { name: "Learn Arm" }).click();
  const learn = page.getByRole("status", { name: "Learn" });
  await expect(learn).toContainText("Move the control for Arm to “armed”.");
  await app.core(`c => c.radioFrame(${JSON.stringify(REST)})`);
  await app.core(`c => c.radioFrame([1024, 1024, 0, 1024, 0, 0, 0, 2048])`);
  await expect(page.getByText("SD moved, but it cannot do Arm")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  // Rates: SC from down to up gives SC with up sending 2000 µs.
  const down = [1024, 1024, 0, 1024, 0, 0, 2048, 0];
  await app.core(`c => c.radioFrame(${JSON.stringify(down)})`);
  await jobRow(page, "Rates").getByRole("button", { name: "Learn Rates" }).click();
  await expect(learn).toContainText("Move the control for Rates");
  await app.core(`c => c.radioFrame(${JSON.stringify(down)})`);
  await app.core(`c => c.radioFrame(${JSON.stringify(REST)})`);
  await expect.poll(async () => (await lastEdit(app))?.setup?.assignments.find((a) => a.job === "rates")).toEqual({ job: "rates", control: "SC", high: 0 });
  await expect(learn).toHaveCount(0);
});

test("a radio whose board QuadCam has no control list for says so", async ({ app, page }) => {
  await app.open();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("table", { name: "Devices" }).getByRole("button", { name: "Field radio", exact: true }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Setup" }).click();
  await expect(page.getByRole("alert")).toContainText('QuadCam has no control list for the board "tx16s"');
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Setup segment passes axe", async ({ app, page }) => {
      await openSetup(app, page);
      await page.getByRole("table", { name: "Fitted controls" }).getByLabel("SA: fitted").selectOption("TOGGLE");
      await expect(jobRow(page, "Arm").getByRole("alert")).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
