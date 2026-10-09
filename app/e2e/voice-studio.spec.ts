// The Voice studio on the mock core (design 7.4): the key (kept out of every answer), the
// voice and model pickers, the credits, the line sets with a cost, a Sample that waits for
// the person's go-ahead and then shows an A/B grid, a Render into a local pack, and axe.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

async function openStudio(app: AppFixture, page: Page) {
  await app.open();
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Field radio/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Voice" }).click();
  return page.getByRole("region", { name: "Voice studio" });
}

async function saveKey(studio: ReturnType<Page["getByRole"]>) {
  await studio.getByLabel("ElevenLabs API key").fill("fake-key-0123456789abcdef");
  await studio.getByRole("button", { name: "Save key" }).click();
}

test("the key goes to the core once, is cleared from the field, and is never shown again", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveAttribute("type", "password");
  await expect(studio.getByRole("button", { name: "Save key" })).toBeDisabled();
  await saveKey(studio);
  expect((await app.method("gear_voice_key")).at(-1)).toEqual({ action: "set", key: "fake-key-0123456789abcdef" });
  await expect(studio).toContainText("ElevenLabs key stored (ends in 3f9a).");
  await expect(studio.getByLabel("ElevenLabs API key")).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("fake-key-0123456789abcdef");
  await expect(studio.getByRole("status").first()).toContainText("98,500 credits left of 100,000.");
  await studio.getByRole("button", { name: "Remove key" }).click();
  expect((await app.method("gear_voice_key")).at(-1)).toMatchObject({ action: "delete" });
  await expect(studio.getByLabel("ElevenLabs API key")).toBeVisible();
});

test("the pickers list the account's voices and models, and the cost follows the picks", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const voices = studio.getByRole("group", { name: "Voices" });
  await expect(voices.getByRole("checkbox")).toHaveCount(8);
  await expect(voices.getByRole("checkbox", { name: /Callum/ })).toBeChecked();
  const models = studio.getByRole("group", { name: "Models" });
  await expect(models.getByRole("checkbox", { name: /eleven_turbo_v2_5,/ })).toBeVisible();
  const cost = studio.getByRole("status", { name: "Cost" });
  await expect(cost).toContainText("Callum, eleven_v4: 168 lines");
  const full = await cost.textContent();
  // The price shows in USD and in estimated credits.
  expect(full).toMatch(/\$\d+\.\d\d at \$0\.08 per 1K, [\d,]+ credits \(estimated\)/);
  await expect(models.getByRole("checkbox", { name: /eleven_v4,/ })).toContainText("$0.08 per 1K characters");
  // The half-price model costs half the credits for the same sets.
  await models.getByRole("checkbox", { name: /eleven_v4,/ }).uncheck();
  await models.getByRole("checkbox", { name: /eleven_turbo_v2_5,/ }).check();
  await expect(cost).toContainText("eleven_turbo_v2_5");
  expect(await cost.textContent()).not.toEqual(full);
  // A second set adds its lines.
  await studio.getByRole("group", { name: "Line sets" }).getByRole("checkbox", { name: /Easter eggs/ }).check();
  await expect(cost).toContainText("180 lines");
  expect((await app.method("gear_voice_estimate")).at(-1)).toMatchObject({ sets: ["quad", "easter"], voice: "voice-callum", model: "eleven_turbo_v2_5" });
});

test("Sample asks before it pays, then shows the A/B grid", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  await studio.getByRole("group", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await studio.getByRole("group", { name: "Models" }).getByRole("checkbox", { name: /eleven_turbo_v2_5,/ }).check();
  await studio.getByRole("button", { name: "Sample", exact: true }).click();
  expect((await app.method("gear_voice_sample")).at(-1)).toMatchObject({ voices: ["voice-callum", "voice-daniel"], models: ["eleven_v4", "eleven_turbo_v2_5"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  await expect(studio.getByRole("table", { name: "Sample voices" })).toHaveCount(0);
  await studio.getByRole("button", { name: "Sample and pay" }).click();
  expect((await app.method("gear_voice_sample")).at(-1)).toMatchObject({ confirm: true });
  const grid = studio.getByRole("table", { name: "Sample voices" });
  await expect(grid.getByRole("columnheader")).toHaveCount(1 + 4);
  await expect(grid.getByRole("row", { name: /^Six/ })).toBeVisible();
  await grid.getByRole("button", { name: "Play Six in Daniel with eleven_turbo_v2_5" }).click();
  // The credits went down by what the sample cost.
  await expect(studio.getByRole("status").first()).not.toContainText("98,500");
});

test("Render pack needs one voice and one model, asks, then adds a local pack", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const render = studio.getByRole("button", { name: "Render pack" });
  await studio.getByRole("group", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).check();
  await expect(render).toBeDisabled();
  await studio.getByRole("group", { name: "Voices" }).getByRole("checkbox", { name: /Daniel/ }).uncheck();
  await expect(render).toBeEnabled();
  await render.click();
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ voice: "voice-callum", model: "eleven_v4", sets: ["quad"], confirm: false });
  await expect(studio).toContainText("This sends text to ElevenLabs, which bills it.");
  await studio.getByRole("button", { name: "Render and pay" }).click();
  expect((await app.method("gear_voice_render")).at(-1)).toMatchObject({ confirm: true });
  await expect(studio).toContainText("Rendered into local-elevenlabs-callum-eleven-v4");
  await expect(page.getByRole("region", { name: "Voice packs" })).toContainText("Callum (local-elevenlabs-callum-eleven-v4)");
});

test("a render the credits do not cover cannot start", async ({ app, page }) => {
  const studio = await openStudio(app, page);
  await saveKey(studio);
  const group = studio.getByRole("group", { name: "Line sets" });
  await group.getByRole("checkbox", { name: /Full EdgeTX English/ }).check();
  for (const t of ["FPV quad", "Helicopter", "Plane", "Glider", "FPV extras"]) await group.getByRole("checkbox", { name: new RegExp(t) }).check();
  await app.core(`c => { c.gear.voice.spent = 98000; }`);
  await studio.getByRole("group", { name: "Models" }).getByRole("checkbox", { name: /eleven_v4,/ }).uncheck();
  await studio.getByRole("group", { name: "Models" }).getByRole("checkbox", { name: /eleven_v4_turbo,/ }).check();
  await expect(studio.getByRole("status", { name: "Cost" })).toContainText("The account does not have enough.");
  await expect(studio.getByRole("button", { name: "Render pack" })).toBeDisabled();
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Voice studio passes axe", async ({ app, page }) => {
      const studio = await openStudio(app, page);
      await saveKey(studio);
      await studio.getByRole("button", { name: "Sample", exact: true }).click();
      await studio.getByRole("button", { name: "Sample and pay" }).click();
      await expect(studio.getByRole("table", { name: "Sample voices" })).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
