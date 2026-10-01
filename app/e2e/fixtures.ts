import { test as base, expect, type Page } from "@playwright/test";
import type { MockOptions, Scenario } from "../src/ipc/mock/core";
import { MOCK_BUNDLE } from "./global-setup";

export type Ui = "legacy" | "next";

/** Screens built in the new UI so far. A spec for any other area runs on legacy only. */
export const PORTED = new Set<string>(["library", "app", "detail", "import", "settings"]);

export interface AppFixture {
  ui: Ui;
  page: Page;
  /** Opens the app on the mock core. */
  open: (scenario?: Scenario, opts?: Omit<MockOptions, "scenario"> & { query?: string }) => Promise<void>;
  /** Commands the page sent, filtered by name. */
  calls: (cmd?: string, method?: string) => Promise<{ cmd: string; args: Record<string, unknown> }[]>;
  /** The params of every call to a core method, whichever way the UI sent it: the legacy
   * `core_call` and session commands, or the typed commands (see METHOD_OF). */
  method: (name: string) => Promise<Record<string, unknown>[]>;
  /** Runs `fn` against the mock core inside the page. */
  core: <T>(fn: string) => Promise<T>;
}

export const test = base.extend<{ app: AppFixture; area: string }>({
  area: ["", { option: true }],
  app: async ({ page }, use, info) => {
    const ui = (info.project.metadata.ui || "legacy") as Ui;
    const app: AppFixture = {
      ui,
      page,
      open: async (scenario = "library", { query = "", ...opts } = {}) => {
        await page.addInitScript((o) => (window.__QC_MOCK__ = o), { scenario, ...opts });
        await page.addInitScript({ path: MOCK_BUNDLE });
        await page.goto((ui === "legacy" ? "/legacy/" : "/") + query);
      },
      calls: (cmd, method) =>
        page.evaluate(([c, m]) => window.__qc!.calls.filter((x) => (!c || x.cmd === c) && (!m || x.args.method === m)), [cmd, method] as const),
      method: async (name) => {
        const all = await page.evaluate(() => window.__qc!.calls);
        return all.map(asMethod).filter((m) => m.name === name).map((m) => m.params);
      },
      core: (fn) => page.evaluate((src) => new Function("core", `return (${src})(core)`)(window.__qc!.core), fn),
    };
    await use(app);
  },
});

/** A recorded call as (core method, params). The legacy UI sends `core_call` and older
 * session commands; the new UI sends one typed command per method with `{ params }`. */
function asMethod(c: { cmd: string; args: Record<string, unknown> }): { name: string; params: Record<string, unknown> } {
  const a = c.args;
  switch (c.cmd) {
    case "core_call":
      return { name: String(a.method), params: (a.params || {}) as Record<string, unknown> };
    case "load_source":
      return { name: "load", params: { source: a.path } };
    case "edit_plan":
      return { name: "suggest", params: { patches: [a.patch], editor: "user" } };
    case "edit_plans":
      return { name: "suggest", params: { patches: a.patches, editor: "user" } };
    case "import_clips":
      return { name: "import", params: a.options as Record<string, unknown> };
    case "add_to_photos":
      return { name: "photos", params: { ids: a.ids, album: a.album } };
    case "clear_session":
      return { name: "clear", params: {} };
  }
  return { name: c.cmd, params: (a.params ?? a) as Record<string, unknown> };
}

/** Skips the spec on the new UI until its area is ported. */
export function area(name: string) {
  test.beforeEach(({ app }) => {
    test.skip(app.ui === "next" && !PORTED.has(name), `${name} is not ported to the new UI yet`);
  });
}

export { expect };

/** An open dialog by its heading. */
export const dialog = (page: Page, heading: string | RegExp) =>
  page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: heading }) });

/** The clip card or row that carries the selection: the article itself (legacy), or the
 * grid cell around it (new UI). */
export const clipCell = (page: Page, name: string) =>
  page.locator(`[aria-selected][aria-label="${name}"], [aria-selected]:has(> article[aria-label="${name}"])`).first();

/** Every selected clip card or list row. */
export const selectedClips = (page: Page) => page.locator('article[aria-selected="true"], [role="gridcell"][aria-selected="true"]');

/** The list view's table. */
export const clipTable = (page: Page) => page.locator("table");
