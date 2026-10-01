// Screenshots of each screen on both UIs and both themes, for review. Run with
// QC_SHOTS=<dir>; skipped otherwise.
import { expect, test } from "./fixtures";

const dir = process.env.QC_SHOTS;

for (const scheme of ["dark", "light"] as const) {
  test.describe(`screenshots ${scheme}`, () => {
    test.skip(!dir, "set QC_SHOTS to a folder");
    test.use({ colorScheme: scheme });
    const shot = (ui: string, name: string) => `${dir}/${ui}-${name}-${scheme}.png`;

    test("library", async ({ app, page }) => {
      await app.open();
      await expect(page.getByRole("article").first()).toBeVisible();
      await page.getByRole("article", { name: "gap-run" }).click();
      await page.getByRole("article", { name: "backyard-loops" }).click({ modifiers: ["Meta"] });
      await page.mouse.move(10, 10);
      await page.waitForTimeout(300);
      await page.screenshot({ path: shot(app.ui, "library") });
      await page.getByRole("button", { name: "List", exact: true }).click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot(app.ui, "list") });
    });

    test("menu", async ({ app, page }) => {
      await app.open("many");
      await page.getByRole("article", { name: "gap-run" }).click({ button: "right" });
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot(app.ui, "menu") });
    });

    test("first run", async ({ app, page }) => {
      await app.open("empty");
      await expect(page.getByRole("heading", { name: "Import your first flights" })).toBeVisible();
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot(app.ui, "first-run") });
    });
  });
}
