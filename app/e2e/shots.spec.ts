// Screenshots of each screen in both themes, for review. Run with
// QC_SHOTS=<dir>; skipped otherwise.
import { expect, test } from "./fixtures";

const dir = process.env.QC_SHOTS;

for (const scheme of ["dark", "light"] as const) {
  test.describe(`screenshots ${scheme}`, () => {
    test.skip(!dir, "set QC_SHOTS to a folder");
    test.use({ colorScheme: scheme });
    const shot = (name: string) => `${dir}/${name}-${scheme}.png`;

    test("library", async ({ app, page }) => {
      await app.open();
      await expect(page.getByRole("article").first()).toBeVisible();
      await page.getByRole("article", { name: "gap-run" }).click();
      await page.getByRole("article", { name: "backyard-loops" }).click({ modifiers: ["Meta"] });
      await page.mouse.move(10, 10);
      await page.waitForTimeout(300);
      await page.screenshot({ path: shot("library") });
      await page.getByRole("button", { name: "List", exact: true }).click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot("list") });
    });

    test("menu", async ({ app, page }) => {
      await app.open("many");
      await page.getByRole("article", { name: "gap-run" }).click({ button: "right" });
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot("menu") });
    });

    test("detail", async ({ app, page }) => {
      await app.open();
      await page.getByRole("article", { name: "gap-run" }).dblclick();
      await page.getByRole("button", { name: "Roll", exact: true }).click();
      await page.mouse.move(10, 10);
      await page.waitForTimeout(300);
      await page.screenshot({ path: shot("detail") });
      await page.getByRole("tab", { name: "Flight" }).click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot("detail-flight") });
    });

    test("import review", async ({ app, page }) => {
      await app.open("review");
      await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /Unfinished import/ }).click();
      await page.waitForTimeout(400);
      await page.screenshot({ path: shot("import-review") });
    });

    test("first run", async ({ app, page }) => {
      await app.open("empty");
      await expect(page.getByRole("heading", { name: "Import your first flights" })).toBeVisible();
      await page.waitForTimeout(200);
      await page.screenshot({ path: shot("first-run") });
    });
  });
}
