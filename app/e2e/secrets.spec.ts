// Secret settings in the Settings window, on the mock core: a stored secret shows only the
// core's hint and a Replace… button, never its value.
import { dialog, expect, test } from "./fixtures";

test("the Google Places key shows its hint and a Replace… button", async ({ app, page }) => {
  await app.open();
  await page.getByRole("button", { name: "Settings" }).click();
  const dlg = dialog(page, "Settings");
  await dlg.getByRole("button", { name: "Places" }).click();
  await expect(dlg.getByLabel("Google Places API key")).toHaveAttribute("type", "password");
  await dlg.getByLabel("Google Places API key").fill("AIza-GOOGLEMIDDLE-q7Rt");
  await dlg.getByRole("button", { name: "Done" }).click();
  expect((await app.method("settings_set")).at(-1)).toMatchObject({ values: { googlePlacesKey: "AIza-GOOGLEMIDDLE-q7Rt" } });

  await page.getByRole("button", { name: "Settings" }).click();
  await dlg.getByRole("button", { name: "Places" }).click();
  await expect(dlg.getByLabel("Google Places API key")).toHaveText("AIza…q7Rt (22 chars)");
  await expect(page.locator("body")).not.toContainText("GOOGLEMIDDLE");
  await dlg.getByRole("button", { name: "Replace…" }).click();
  await expect(dlg.getByLabel("Google Places API key")).toHaveValue("");
  await dlg.getByRole("button", { name: "Keep Current" }).click();
  await expect(dlg.getByLabel("Google Places API key")).toHaveText("AIza…q7Rt (22 chars)");
});
