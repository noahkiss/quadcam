// The Sim page's Rates section and the sidebar badge on the mock core (WP8): the badge says "Out of date"
// while a sim's rates differ from the quad's, the page lists each sim against the quad's
// profile in use, Velocidrone says it is off, and axe passes.
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "./fixtures";

test("the Sims item says Out of date while a sim differs, and the page lists every sim", async ({ app, page }) => {
  await app.open();
  const side = page.getByRole("navigation", { name: "Library" });
  const item = side.getByRole("button", { name: /^Sim( |$)/ });
  await expect(item).toContainText("Out of date");
  await item.click();

  await expect(page.getByRole("heading", { name: "Sim", exact: true })).toBeVisible();
  await expect(page.getByRole("group", { name: "Sim sections" }).getByRole("button", { name: "Rates" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Compared with Whoop FC · 0 FREE")).toBeVisible();
  await expect(page.locator("[data-component=page-header]")).toContainText("1 out of date");
  const sims = page.getByRole("table", { name: "Sims" });
  const row = (name: string) => sims.getByRole("row").filter({ has: page.getByText(name, { exact: true }) }).first();
  await expect(row("Liftoff: Micro Drones")).toContainText("Out of date");
  await expect(row("Liftoff")).toContainText("Matches the quad");
  await expect(row("Velocidrone")).toContainText("Off");
  await expect(row("Velocidrone")).toContainText("Not supported yet. QuadCam needs a sample Velocidrone save to read its rate file, so this sim stays off.");
  expect((await app.method("gear_sims")).at(-1)).toEqual({ paths: [], device: "fc-0a1b2c3d4e5f6071", profile: 0 });

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("without a quad with a backup the page says what to do and the badge stays off", async ({ app, page }) => {
  await app.open();
  await app.core(`c => { c.gear.devices = c.gear.devices.filter(d => d.kind !== "fc"); c.emit("gear-changed"); }`);
  const side = page.getByRole("navigation", { name: "Library" });
  await side.getByRole("button", { name: /^Devices/ }).click();
  await side.getByRole("button", { name: /^Sim( |$)/ }).click();
  await expect(side.getByRole("button", { name: /^Sim( |$)/ })).not.toContainText("Out of date");
  await expect(page.getByText("No quad to compare with.", { exact: false })).toBeVisible();
});
