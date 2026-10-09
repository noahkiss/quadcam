// The radio's model editors on the mock core (design 6.3, WP9): the Models segment edits
// timers, telemetry screens, logging, alarms and callouts; the Checklists segment edits the
// power-on checklist. Each edit stages through `gear_model_edit` into one "Model edits"
// change, shown with Review and Undo; the apply sheet shows the card diff and the apply
// verifies. Axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const RADIO_LINK = `{ kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }`;
const radio = `{ id: "${RADIO_ID}", kind: "radio", link: ${RADIO_LINK}, identity: { board: "pocket" } }`;

/** The saved radio's page on one of its segments. */
async function openSegment(app: AppFixture, page: Page, segment: "Models" | "Checklists", plugged = false) {
  await app.open();
  if (plugged) await app.core(`c => c.plug([${radio}])`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Field radio/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: segment }).click();
}

const lastEdit = async (app: AppFixture) => (await app.method("gear_model_edit")).at(-1);

test("a radio that is not plugged in reads its models from the latest backup", async ({ app, page }) => {
  await openSegment(app, page, "Models");
  await expect(page.getByText(/Read from the backup/)).toBeVisible();
  const picker = page.getByRole("combobox", { name: "Model" });
  await expect(picker).toHaveValue("model01.yml");
  await expect(picker.locator("option")).toHaveText(["ALPHA", "BRAVO 2 (selected on the radio)"]);
  await expect(page.getByRole("region", { name: "Timers" })).toContainText("Timer 2");
  await expect(page.getByRole("region", { name: "Callouts" })).toContainText("armed while L1 is on, once");
  await picker.selectOption("model00.yml");
  await expect.poll(async () => (await app.method("gear_model")).at(-1)).toMatchObject({ device: RADIO_ID, model: "model00.yml", staged: true });
});

test("a timer edit stages, and a second edit of the same timer joins the same change", async ({ app, page }) => {
  await openSegment(app, page, "Models");
  const timers = page.getByRole("region", { name: "Timers" });
  await timers.getByLabel("Timer 2 countdown").selectOption("2");
  await expect.poll(() => lastEdit(app)).toEqual({ device: RADIO_ID, model: "model01.yml", ops: [{ op: "set_timer", index: 1, fields: [{ key: "countdownBeep", value: "2" }] }], checklist: null });
  await expect(page.getByText("1 model edit is staged.")).toBeVisible();
  await timers.getByLabel("Timer 2 name").fill("FLY");
  await timers.getByLabel("Timer 2 name").press("Enter");
  await expect(page.getByText("1 model edit is staged.")).toBeVisible();
  await timers.getByRole("checkbox", { name: "Beep every minute" }).nth(1).uncheck();
  await expect(timers.getByLabel("Timer 2 name")).toHaveValue("FLY");
  expect(await app.method("gear_change_stage")).toHaveLength(0);
  const changes = await app.core<{ title: string; edits: { ops: { fields: { key: string }[] }[] }[] }[]>(`c => c.dispatch("gear_changes", { device: null, history: false })`);
  expect(changes).toHaveLength(1);
  expect(changes[0].title).toBe("Model edits");
  expect(changes[0].edits[0].ops[0].fields.map((f) => f.key).sort()).toEqual(["countdownBeep", "minuteBeep", "name"]);
  // Undo discards the change.
  await page.getByRole("button", { name: "Undo model edits" }).click();
  await expect(page.getByText(/model edits? (is|are) staged/)).toHaveCount(0);
  await expect(timers.getByLabel("Timer 2 name")).toHaveValue("FLT");
});

test("a timer can be added and removed", async ({ app, page }) => {
  await openSegment(app, page, "Models");
  const timers = page.getByRole("region", { name: "Timers" });
  await timers.getByRole("button", { name: "Add timer" }).click();
  await expect(timers.getByText("Timer 3")).toBeVisible();
  await expect(timers.getByRole("button", { name: "Add timer" })).toHaveCount(0);
  await timers.getByRole("button", { name: "Remove timer 3" }).click();
  // Back to what the radio has: the change is gone.
  await expect(page.getByText(/model edits? (is|are) staged/)).toHaveCount(0);
  await expect(timers.getByText("Timer 3")).toHaveCount(0);
});

test("a value screen line stages with its sources", async ({ app, page }) => {
  await openSegment(app, page, "Models");
  const screens = page.getByRole("region", { name: "Telemetry screens" });
  const line = screens.getByLabel("Screen 1 line 2");
  await line.fill("{Capa}, Tmr1");
  await line.press("Enter");
  await expect.poll(() => lastEdit(app)).toMatchObject({ ops: [{ op: "set_screen_values", index: 0, lines: [["{RxBt}", "Tmr1"], ["{Capa}", "Tmr1"]] }] });
  await expect(screens.getByLabel("Screen 1 line 2")).toHaveValue("{Capa}, Tmr1");
  // A sensor the model does not have is refused with the core's words.
  await line.fill("{Nope}");
  await line.press("Enter");
  await expect(page.getByRole("alert").filter({ hasText: "discover sensors" })).toBeVisible();
});

test("logging, the sensors in the log and the RSSI alarms", async ({ app, page }) => {
  await openSegment(app, page, "Models");
  const logging = page.getByRole("region", { name: "Logging" });
  await logging.getByRole("checkbox", { name: "Write a log" }).check();
  await expect.poll(() => lastEdit(app)).toMatchObject({ ops: [{ op: "set_logging", logging: { swtch: "ON", period_ds: 10 } }] });
  const every = logging.getByLabel("Log every (seconds)");
  await expect(every).toHaveValue("1");
  await every.fill("0.5");
  await every.press("Enter");
  await expect.poll(() => lastEdit(app)).toMatchObject({ ops: [{ op: "set_logging", logging: { swtch: "ON", period_ds: 5 } }] });
  await logging.getByRole("checkbox", { name: "Capa" }).uncheck();
  await expect.poll(() => lastEdit(app)).toMatchObject({ ops: [{ op: "set_sensor_logs", sensors: [{ label: "Capa", logs: false }] }] });
  await expect(logging.getByRole("checkbox", { name: "Capa" })).not.toBeChecked();
  const alarms = page.getByRole("region", { name: "Alarms" });
  await alarms.getByLabel("RSSI critical").fill("50");
  await alarms.getByLabel("RSSI critical").press("Enter");
  await expect(page.getByRole("alert").filter({ hasText: "critical level (50) is over the warning level (45)" })).toBeVisible();
  await alarms.getByLabel("RSSI warning").fill("60");
  await alarms.getByLabel("RSSI warning").press("Enter");
  await expect.poll(() => lastEdit(app)).toMatchObject({ ops: expect.arrayContaining([{ op: "set_rf_alarms", warning: 60, critical: 42 }]) });
});

test("a battery callout is added, edited, and removed", async ({ app, page }) => {
  await openSegment(app, page, "Models");
  const callouts = page.getByRole("region", { name: "Callouts" });
  await callouts.getByRole("button", { name: "Add callout" }).click();
  const form = callouts.getByRole("form", { name: "Callout" });
  await form.getByRole("button", { name: "Stage callout" }).click();
  await expect(form.getByRole("alert")).toHaveText("Pick a sound for the callout.");
  await form.getByLabel("Sound").fill("lowbat");
  await form.getByRole("button", { name: "Stage callout" }).click();
  await expect(form.getByRole("alert")).toHaveText("The value is a number.");
  await form.getByLabel("Value").fill("3.5");
  await form.getByLabel("Repeat", { exact: true }).selectOption("every");
  await form.getByLabel("Seconds between repeats").fill("5");
  await form.getByRole("button", { name: "Stage callout" }).click();
  await expect.poll(() => lastEdit(app)).toEqual({
    device: RADIO_ID,
    model: "model01.yml",
    ops: [{ op: "set_callout", callout: { track: "lowbat", when: "below", source: "{RxBt}", value: "3.5", delay_ds: 20, repeat: "5" } }],
    checklist: null,
  });
  await expect(callouts).toContainText("lowbat RxBt below 3.5 for 2 s, every 5 s");
  // Edit it: the same track replaces the staged callout.
  await callouts.getByRole("button", { name: "Edit callout lowbat" }).click();
  await callouts.getByLabel("Value").fill("3.4");
  await callouts.getByRole("button", { name: "Stage callout" }).click();
  await expect(callouts).toContainText("lowbat RxBt below 3.4 for 2 s, every 5 s");
  await expect(page.getByText("1 model edit is staged.")).toBeVisible();
  // A callout the editor did not write cannot be edited, but it can be removed.
  await callouts.getByRole("button", { name: "Remove callout lowbat" }).click();
  await expect(callouts).not.toContainText("lowbat");
  await expect(page.getByText(/model edits? (is|are) staged/)).toHaveCount(0);
});

test("the checklist editor stages text, checks the width, and turns the checklist on", async ({ app, page }) => {
  await openSegment(app, page, "Checklists");
  const sec = page.getByRole("region", { name: "Checklist" });
  await expect(sec.getByText("This model has no checklist items.")).toBeVisible();
  await expect(sec.getByRole("checkbox", { name: "Show the checklist when the model loads" })).not.toBeChecked();
  await sec.getByRole("button", { name: "Add item" }).click();
  await sec.getByLabel("Item 1", { exact: true }).fill("Props tight");
  await sec.getByRole("button", { name: "Add item" }).click();
  await sec.getByLabel("Item 2", { exact: true }).fill("Battery strapped, antenna clear of props");
  await expect(sec.getByRole("alert")).toContainText("One item is over 20 characters");
  await expect(sec.getByRole("button", { name: "Stage checklist" })).toBeDisabled();
  await sec.getByLabel("Item 2", { exact: true }).fill("Battery strapped");
  await sec.getByRole("checkbox", { name: "Tick box" }).nth(1).uncheck();
  await sec.getByRole("button", { name: "Move item 2 up" }).click();
  await expect(sec.getByLabel("Item 1", { exact: true })).toHaveValue("Battery strapped");
  await sec.getByRole("button", { name: "Stage checklist" }).click();
  await expect.poll(() => lastEdit(app)).toMatchObject({ ops: [], checklist: "Battery strapped\n=Props tight\n" });
  await expect(page.getByText("2 model edits are staged.")).toBeVisible();
  await expect(sec.getByRole("checkbox", { name: "Show the checklist when the model loads" })).toBeChecked();
  await expect(sec.getByRole("button", { name: "Stage checklist" })).toBeDisabled();
});

test("Review shows the card diff, and the apply verifies; the next read shows the applied model", async ({ app, page }) => {
  await openSegment(app, page, "Models", true);
  const callouts = page.getByRole("region", { name: "Callouts" });
  await callouts.getByRole("button", { name: "Add callout" }).click();
  await callouts.getByLabel("Sound").fill("lowbat");
  await callouts.getByLabel("Value").fill("3.5");
  await callouts.getByRole("button", { name: "Stage callout" }).click();
  await page.getByRole("button", { name: "Review…" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("callout lowbat");
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("MODELS/model01.yml");
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText(/model edits? (is|are) staged/)).toHaveCount(0);
  await expect(callouts).toContainText("lowbat RxBt below 3.5");
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    for (const segment of ["Models", "Checklists"] as const) {
      test(`the ${segment} segment passes axe`, async ({ app, page }) => {
        await openSegment(app, page, segment);
        if (segment === "Models") {
          await page.getByRole("region", { name: "Timers" }).getByLabel("Timer 2 countdown").selectOption("1");
          await page.getByRole("region", { name: "Callouts" }).getByRole("button", { name: "Add callout" }).click();
        } else {
          await page.getByRole("button", { name: "Add item" }).click();
        }
        await expect(page.getByRole("group", { name: "Sections" })).toBeVisible();
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      });
    }
  });
}
