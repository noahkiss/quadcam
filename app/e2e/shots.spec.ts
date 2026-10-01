// Screenshots of each screen on both UIs, for review. Run with QC_SHOTS=<dir>.
import { expect, test } from "./fixtures";

const dir = process.env.QC_SHOTS;

test.describe("screenshots", () => {
  test.skip(!dir, "set QC_SHOTS to a folder");

  test("library", async ({ app, page }) => {
    await app.open();
    await expect(page.getByRole("article").first()).toBeVisible();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${dir}/${app.ui}-library.png` });
  });
});
