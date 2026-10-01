// The base components in both flavors: WCAG AA (axe, contrast included), and screenshots
// for review with QC_SHOTS=<dir>.
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "./fixtures";

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });

    test("the component gallery passes axe", async ({ app, page }) => {
      test.skip(app.ui !== "next", "the gallery is part of the new UI");
      await app.open("library", { query: "?gallery" });
      await expect(page.getByRole("heading", { name: "Components" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/gallery-${scheme}.png`, fullPage: true });
    });

    for (const [scenario, ready] of [["library", "gap-run"], ["empty", ""]] as const) {
      test(`the ${scenario} screen passes axe`, async ({ app, page }) => {
        test.skip(app.ui !== "next", "contrast is checked on the new UI");
        await app.open(scenario);
        if (ready) {
          await page.getByRole("article", { name: ready }).click();
          await page.getByRole("article", { name: "backyard-loops" }).click({ modifiers: ["Meta"] });
        } else await expect(page.getByRole("heading", { name: "Import your first flights" })).toBeVisible();
        await page.mouse.move(1, 400);
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
        expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      });
    }

    test("the open clip passes axe", async ({ app, page }) => {
      test.skip(app.ui !== "next", "contrast is checked on the new UI");
      await app.open();
      await page.getByRole("article", { name: "gap-run" }).dblclick();
      await page.getByRole("button", { name: "Roll", exact: true }).click();
      await page.mouse.move(1, 400);
      let r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      await page.getByRole("tab", { name: "Flight" }).click();
      r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });

    test("open dialog and menu pass axe", async ({ app, page }) => {
      test.skip(app.ui !== "next", "the gallery is part of the new UI");
      await app.open("library", { query: "?gallery" });
      await page.getByRole("button", { name: "Open menu" }).click();
      await expect(page.getByRole("menu", { name: "Clip actions" })).toBeVisible();
      let r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "Open dialog" }).click();
      await expect(page.getByRole("dialog", { name: "Move gap-run to the Trash?" })).toBeVisible();
      // Axe reads colors mid-fade otherwise.
      await page.waitForFunction(() => document.getAnimations().every((a) => a.playState === "finished"));
      r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
      if (process.env.QC_SHOTS) await page.screenshot({ path: `${process.env.QC_SHOTS}/gallery-dialog-${scheme}.png` });
    });
  });
}
