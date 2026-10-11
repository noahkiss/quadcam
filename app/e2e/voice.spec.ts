// The radio's Voice segment on the mock core (design 7.4, WP9): Choose voice from the packs on
// the Voices page (one staged change, Keep my overrides), per-line overrides, the apply
// sheet's card diff, and axe in both themes.
import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test, type AppFixture } from "./fixtures";

const RADIO_ID = "radio-1f2e3d4c5b6a7980";
const RADIO_LINK = `{ kind: "volume", mount: "/Volumes/RADIO", volume_uuid: null, bus_protocol: "USB", whole_disk: "disk4" }`;
const radio = `{ id: "${RADIO_ID}", kind: "radio", link: ${RADIO_LINK}, identity: { board: "pocket" } }`;

async function openVoice(app: AppFixture, page: Page, plugged = false) {
  await app.open();
  if (plugged) await app.core(`c => c.plug([${radio}])`);
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Field radio/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Voice" }).click();
}

/** Installs the index's Demo pack on the Voices page, then opens the radio's Voice segment. */
async function installDemo(page: Page) {
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Voices/ }).click();
  await page.getByRole("button", { name: "Refresh packs" }).click();
  await page.getByRole("button", { name: "Install Demo" }).click();
  await expect(page.getByRole("table", { name: "Voice packs on this Mac" })).toContainText("Demo");
  await page.getByRole("navigation", { name: "Library" }).getByRole("button", { name: /^Devices/ }).click();
  await page.getByRole("list", { name: "Devices" }).getByRole("button", { name: /Field radio/ }).click();
  await page.getByRole("group", { name: "Sections" }).getByRole("button", { name: "Voice" }).click();
}

test("the lines show the spoken text, and Choose voice lists the packs on the Voices page", async ({ app, page }) => {
  await openVoice(app, page);
  const lines = page.getByRole("table", { name: "Voice lines" });
  await expect(lines.getByRole("row", { name: /armed\.wav Armed/ })).toBeVisible();
  await expect(lines.getByRole("row", { name: /gpsfix\.wav GPS fix \(spoken: G\.P\.S\. fix\)/ })).toBeVisible();
  const choose = page.getByRole("region", { name: "Choose voice" });
  await expect(choose).toContainText("No voice pack on this Mac.");
  // The studio and the pack list live on the Voices page now.
  await expect(page.getByRole("region", { name: "Voice studio" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Refresh packs" })).toHaveCount(0);
  await choose.getByRole("button", { name: "Open Voices" }).click();
  await expect(page.getByRole("heading", { name: "Voices", level: 2 })).toBeVisible();
  await installDemo(page);
  await expect(choose.getByRole("combobox", { name: "Voice" })).toHaveValue("en-demo-v1");
  await expect(lines.getByRole("button", { name: "Play Armed in Demo" })).toBeVisible();
});

test("Choose voice stages one change and keeps the overrides when asked", async ({ app, page }) => {
  await openVoice(app, page);
  await installDemo(page);
  const choose = page.getByRole("region", { name: "Choose voice" });
  const keep = choose.getByRole("checkbox", { name: "Keep my overrides" });
  await expect(keep).toBeChecked();
  await choose.getByRole("button", { name: "Choose voice" }).click();
  expect((await app.method("gear_voice_choose")).at(-1)).toEqual({ radio: RADIO_ID, pack: "en-demo-v1", keep_overrides: true, editor: null });
  await expect(page.getByText(/Voice: Demo is staged: 7 sounds to put on the card/)).toBeVisible();
  await expect(choose).toContainText("Chosen for this radio: en-demo-v1.");
  // Again with Keep my overrides off: the same change, updated.
  await keep.uncheck();
  await choose.getByRole("button", { name: "Choose voice" }).click();
  expect((await app.method("gear_voice_choose")).at(-1)).toMatchObject({ keep_overrides: false });
  const changes = await app.core<{ title: string }[]>(`c => c.dispatch("gear_changes", { device: null, history: false })`);
  expect(changes.map((c) => c.title)).toEqual(["Voice: Demo"]);
  await page.getByRole("button", { name: "Undo voice change" }).click();
  await expect(page.getByText(/Voice: Demo is staged/)).toHaveCount(0);
});

test("one line takes another voice's take or the person's own text, and Reset clears it", async ({ app, page }) => {
  await openVoice(app, page);
  await installDemo(page);
  const row = page.getByRole("row", { name: /armed\.wav/ });
  await row.getByRole("combobox", { name: "Take for Armed" }).selectOption("en-demo-v1");
  expect((await app.method("gear_voice_edit")).at(-1)).toEqual({ radio: RADIO_ID, line: "SOUNDS/en/armed.wav", text: null, pack: "en-demo-v1", confirm: false });
  await expect(row).toContainText("take from en-demo-v1");
  await row.getByRole("button", { name: "My text…" }).click();
  await row.getByLabel("My text for Armed").fill("Motors live");
  await row.getByRole("button", { name: "Render" }).click();
  expect((await app.method("gear_voice_edit")).at(-1)).toMatchObject({ text: "Motors live", pack: null });
  await expect(row).toContainText("my text: Motors live");
  await expect(row.getByRole("button", { name: "Play Armed in my text" })).toBeVisible();
  await row.getByRole("button", { name: "Play Armed in my text" }).click();
  expect((await app.method("gear_voice_preview")).at(-1)).toMatchObject({ line: "SOUNDS/en/armed.wav", pack: null, radio: RADIO_ID });
  await row.getByRole("button", { name: "Reset" }).click();
  await expect(row).not.toContainText("my text");
  // The group filter hides the rest.
  await page.getByRole("combobox", { name: "Group" }).selectOption("numbers");
  await expect(page.getByRole("row", { name: /armed\.wav/ })).toHaveCount(0);
  await expect(page.getByRole("row", { name: /0001\.wav one/ })).toBeVisible();
});

test("Review shows the sounds that go on the card, and the apply verifies", async ({ app, page }) => {
  await openVoice(app, page, true);
  await installDemo(page);
  await page.getByRole("region", { name: "Choose voice" }).getByRole("button", { name: "Choose voice" }).click();
  await page.getByRole("button", { name: "Review…" }).click();
  const sheet = page.getByRole("dialog", { name: "Apply to Field radio" });
  await expect(sheet.getByRole("region", { name: "Changes" })).toContainText("SOUNDS/en/armed.wav");
  await sheet.getByRole("button", { name: "Apply" }).click();
  await expect(sheet.getByRole("region", { name: "Result" })).toContainText("Verified");
  await sheet.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText(/Voice: Demo is staged/)).toHaveCount(0);
});

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test("the Voice segment passes axe", async ({ app, page }) => {
      await openVoice(app, page);
      await installDemo(page);
      await page.getByRole("row", { name: /armed\.wav/ }).getByRole("button", { name: "My text…" }).click();
      await page.getByRole("region", { name: "Choose voice" }).getByRole("button", { name: "Choose voice" }).click();
      await expect(page.getByText(/Voice: Demo is staged/)).toBeVisible();
      const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze();
      expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ") + " " + n.failureSummary).join(", ")}`)).toEqual([]);
    });
  });
}
