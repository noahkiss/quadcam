// On-device suggestions (Apple's model, `core/ondevice.rs`) on the mock core: the Review
// step's Suggest names and its on-device marks, the setting, and the report's Summarize day.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test, type AppFixture } from "./fixtures";

const sheet = (page: Page) => dialog(page, "Import");
const row = (page: Page, name: string) => sheet(page).locator('[role="row"]').filter({ has: page.getByRole("textbox", { name: `Short name for ${name}` }) });
const nameOf = (page: Page, name: string) => page.getByRole("textbox", { name: `Short name for ${name}` });

async function openReview(app: AppFixture, onDevice = true) {
  await app.open("review");
  if (!onDevice) await app.core("c => { c.onDevice = false; }");
  await app.page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /Unfinished import/ }).click();
  await expect(sheet(app.page)).toBeVisible();
}

test("Suggest names fills empty names with one outing name and pack numbers, marked on-device", async ({ app, page }) => {
  await openReview(app);
  await nameOf(page, "PICT0002.AVI").fill("my own name");
  await nameOf(page, "PICT0002.AVI").press("Tab");
  await sheet(page).getByRole("button", { name: "Suggest names" }).click();
  await expect.poll(() => app.method("suggest_on_device")).toEqual([{ ids: null }]);
  await expect(nameOf(page, "PICT0001.AVI")).toHaveValue("morning loops pack 1");
  await expect(row(page, "PICT0001.AVI").getByText("on-device", { exact: true })).toBeVisible();
  await expect(row(page, "PICT0001.AVI").getByText("agent", { exact: true })).toHaveCount(0);
  // The typed name stays; the suggestion waits beside it.
  await expect(nameOf(page, "PICT0002.AVI")).toHaveValue("my own name");
  const hint = row(page, "PICT0002.AVI");
  await expect(hint).toContainText("On-device: morning loops pack 2");
  await hint.getByRole("button", { name: "Use the on-device name for PICT0002.AVI" }).click();
  await expect(nameOf(page, "PICT0002.AVI")).toHaveValue("morning loops pack 2");
  // An edit makes the name the person's: its mark goes (the note keeps the model's).
  await expect(nameOf(page, "PICT0001.AVI")).toHaveAttribute("title", "On-device suggestion");
  await nameOf(page, "PICT0001.AVI").fill("backyard loops");
  await nameOf(page, "PICT0001.AVI").press("Tab");
  await expect(nameOf(page, "PICT0001.AVI")).not.toHaveAttribute("title", "On-device suggestion");
  await expect(row(page, "PICT0001.AVI").getByText("on-device", { exact: true })).toBeVisible();
});

test("no Suggest names without the model or with the setting off", async ({ app, page }) => {
  await openReview(app, false);
  await expect(nameOf(page, "PICT0001.AVI")).toBeVisible();
  await expect(sheet(page).getByRole("button", { name: "Suggest names" })).toHaveCount(0);
  expect(await app.calls("on_device_status")).toHaveLength(1);

  await app.open("review", { settings: { onDeviceSuggestions: false } });
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /Unfinished import/ }).click();
  await expect(nameOf(page, "PICT0001.AVI")).toBeVisible();
  await expect(sheet(page).getByRole("button", { name: "Suggest names" })).toHaveCount(0);
  expect(await app.calls("on_device_status")).toHaveLength(0);
});

test("the session report's Summarize day adds the model's paragraph", async ({ app, page }) => {
  await app.open();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Flights/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Session report" }).click();
  await page.getByRole("button", { name: "Summarize day" }).click();
  await expect(page.getByLabel("Summary of the day")).toContainText("A calm morning at the field");
  expect((await app.method("gear_session_report")).at(-1)).toEqual({ day: null, summarize: true });
  await expect(page.getByRole("button", { name: "Summarize day" })).toHaveCount(0);
  await app.core(`c => c.dialogAnswers.push("/Users/pilot/report.md")`);
  await page.getByRole("button", { name: "Save…" }).click();
  await expect(page.getByRole("status")).toContainText("Saved the report");
  expect(await app.method("gear_session_report_save")).toEqual([{ day: null, path: "/Users/pilot/report.md", overwrite: true, summarize: true }]);
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Review step with on-device suggestions passes axe", async ({ app, page }) => {
      await openReview(app);
      await nameOf(page, "PICT0002.AVI").fill("my own name");
      await nameOf(page, "PICT0002.AVI").press("Tab");
      await sheet(page).getByRole("button", { name: "Suggest names" }).click();
      await expect(row(page, "PICT0002.AVI")).toContainText("On-device:");
      await page.mouse.move(0, 0);
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
