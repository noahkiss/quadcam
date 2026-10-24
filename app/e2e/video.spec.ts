import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { axeSchemes, dialog, expect, test } from "./fixtures";

const card = (page: Page, name: string) => page.getByRole("article", { name });

async function findHighlights(page: Page, name: string) {
  await card(page, name).click({ button: "right" });
  await page.getByRole("menu", { name: "Clip actions" }).getByRole("menuitem", { name: "Find Highlights Again" }).click();
}

test("Find highlights again marks highlights on a clip without a radio log", async ({ app, page }) => {
  await app.open("library", { video: true });
  // The last import's clips were analysed at launch.
  await expect.poll(async () => (await app.method("library_analyze_video")).length).toBe(1);
  expect((await app.method("library_analyze_video"))[0]).toMatchObject({ again: false });
  await expect(card(page, "river-dive").getByRole("img", { name: "Highlight" })).toBeVisible();
  // A clip with radio-log moments keeps them.
  await expect(card(page, "gap-run").getByRole("img", { name: "Highlight" })).toHaveCount(0);
  await expect(card(page, "gap-run").getByRole("img", { name: "Roll" })).toBeVisible();

  await findHighlights(page, "river-dive");
  await expect(page.getByText("1 highlight found.")).toBeVisible();
  const calls = await app.method("library_analyze_video");
  expect(calls.at(-1)).toMatchObject({ again: true, ids: [expect.any(String)] });

  await card(page, "river-dive").dblclick();
  const moments = page.locator("li").filter({ has: page.getByRole("button", { name: "Highlight" }) });
  await expect(moments).toHaveCount(1);
  await expect(moments.getByText("Video analysis")).toBeVisible();
  await expect(page.getByRole("button", { name: /^Highlight at / })).toHaveAttribute("title", /Video analysis/);
});

test("on an older macOS nothing runs and the menu says why", async ({ app, page }) => {
  await app.open();
  await expect(card(page, "river-dive")).toBeVisible();
  await expect(card(page, "river-dive").getByRole("img", { name: "Highlight" })).toHaveCount(0);
  await findHighlights(page, "river-dive");
  await expect(page.getByText("Video analysis on this Mac needs macOS 27 or later.")).toBeVisible();
});

test("the setting turns the background analysis off", async ({ app, page }) => {
  await app.open("library", { video: true, settings: { videoAnalysis: false } });
  await expect(card(page, "river-dive")).toBeVisible();
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Import" }).click();
  const box = dlg.getByRole("checkbox", { name: /Find highlights in new clips/ });
  await expect(box).not.toBeChecked();
  expect(await app.method("library_analyze_video")).toEqual([]);
  await box.check();
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect.poll(async () => (await app.method("settings_set")).map((p) => (p as { values: Record<string, unknown> }).values)).toContainEqual(expect.objectContaining({ videoAnalysis: true }));
});

test("the setting says when this Mac cannot run it", async ({ app, page }) => {
  await app.open();
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Import" }).click();
  await expect(dlg.getByText("Video analysis on this Mac needs macOS 27 or later.")).toBeVisible();
});

for (const scheme of axeSchemes) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("highlights in the open clip pass axe", async ({ app, page }) => {
      await app.open("library", { video: true });
      await expect(card(page, "river-dive").getByRole("img", { name: "Highlight" })).toBeVisible();
      await card(page, "river-dive").dblclick();
      await expect(page.getByText("Video analysis").first()).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
