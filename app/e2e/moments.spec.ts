import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test } from "./fixtures";

const card = (page: Page, name: string) => page.getByRole("article", { name });

test.beforeEach(async ({ app }) => {
  await app.open();
  await expect(card(app.page, "gap-run")).toBeVisible();
});

test("Shift time moves the selected clips by one clip's real time; a radio log wins", async ({ app, page }) => {
  await card(page, "backyard-loops").click();
  await card(page, "gap-run").click({ modifiers: ["Meta"] });
  await page.getByRole("toolbar", { name: "Selected clips" }).getByRole("button", { name: "Shift Time…" }).click();
  const dlg = dialog(page, "Shift time");
  await dlg.getByLabel("Clip").selectOption({ label: "backyard-loops" });
  await dlg.getByLabel("Real time").fill("18:30");
  await expect(dlg.getByText("+8:15")).toBeVisible();
  const table = dlg.getByRole("table", { name: "New times" });
  const row = (name: string) => table.getByRole("row").filter({ has: page.getByRole("rowheader", { name }) });
  await expect(row("backyard-loops")).toContainText("18:30");
  await expect(row("backyard-loops")).toContainText("Set");
  await expect(row("gap-run")).toContainText("18:56");
  await expect(row("gap-run")).toContainText("Radio log");
  expect((await app.method("library_time_shift")).every((p) => p.apply === false)).toBe(true);

  await dlg.getByRole("button", { name: "Shift 2 Clips" }).click();
  await expect(dlg).toBeHidden();
  expect((await app.method("library_time_shift")).at(-1)).toMatchObject({ from: expect.any(String), time: "18:30", apply: true });
  await expect(card(page, "backyard-loops").getByText("18:30")).toBeVisible();
  await expect(card(page, "gap-run").getByText("18:56")).toBeVisible();
});

test("Shift time needs a clip with a time of day", async ({ app, page }) => {
  await app.core("(core) => { for (const c of core.lib.clips) c.time = null; core.libraryChanged(); }");
  await card(page, "backyard-loops").click();
  await card(page, "gap-run").click({ modifiers: ["Meta"] });
  await page.getByRole("button", { name: "Shift Time…" }).click();
  const dlg = dialog(page, "Shift time");
  await expect(dlg.getByText("None of the selected clips has a time of day.")).toBeVisible();
  await expect(dlg.getByRole("button", { name: /^Shift/ })).toBeDisabled();
});

test("a clip with moments shows a heat strip on its card and its timeline", async ({ page }) => {
  await expect(card(page, "gap-run").locator('[data-component="heat-strip"]')).toHaveCount(1);
  await expect(card(page, "backyard-loops").locator('[data-component="heat-strip"]')).toHaveCount(0);
  await card(page, "gap-run").dblclick();
  await expect(page.getByRole("group", { name: "Clip timeline" }).locator('[data-component="heat-strip"]')).toHaveCount(1);
});

test("Best moments lists every moment, best first, and opens the clip at one", async ({ app, page }) => {
  const side = page.getByRole("navigation", { name: "Library" });
  await side.getByRole("button", { name: /Best moments/ }).click();
  const list = page.getByRole("list", { name: "Best moments" });
  await expect(list.getByRole("button")).toHaveText([/Roll.*gap-run.*0:20.*90 %/, /Flip.*gap-run.*0:45.*80 %/, /Punch-out.*gap-run.*1:10.*40 %/]);
  await list.getByRole("button", { name: "Flip in gap-run at 0:45" }).click();
  await expect(page.getByRole("region", { name: "Clip" })).toBeVisible();
  await expect.poll(async () => (await app.method("library_preview")).length).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(list).toBeVisible();
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("Best moments and Shift time pass axe", async ({ app, page }) => {
      await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /Best moments/ }).click();
      await expect(page.getByRole("list", { name: "Best moments" })).toBeVisible();
      const check = async () => {
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      };
      await check();
      await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /All clips/ }).click();
      // A clip without a time, so the table shows every source: Set, Radio log, No time.
      await app.core("(core) => { core.lib.clips.find((c) => c.name === 'river-dive').time = null; core.libraryChanged(); }");
      await card(page, "river-dive").click();
      await card(page, "backyard-loops").click({ modifiers: ["Meta"] });
      await card(page, "gap-run").click({ modifiers: ["Meta"] });
      await page.getByRole("button", { name: "Shift Time…" }).click();
      const table = dialog(page, "Shift time").getByRole("table", { name: "New times" });
      await dialog(page, "Shift time").getByLabel("Clip").selectOption({ label: "backyard-loops" });
      for (const t of ["Set", "Radio log", "No time"]) await expect(table.getByText(t, { exact: true })).toBeVisible();
      await check();
    });
  });
}
