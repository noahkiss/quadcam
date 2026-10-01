import { test as base, expect, type Page } from "@playwright/test";
import type { MockOptions, Scenario } from "../src/ipc/mock/core";
import { MOCK_BUNDLE } from "./global-setup";

export type Ui = "legacy" | "next";

/** Screens built in the new UI so far. A spec for any other area runs on legacy only. */
export const PORTED = new Set<string>([]);

export interface AppFixture {
  ui: Ui;
  page: Page;
  /** Opens the app on the mock core. */
  open: (scenario?: Scenario, opts?: Omit<MockOptions, "scenario">) => Promise<void>;
  /** Commands the page sent, filtered by name. */
  calls: (cmd?: string, method?: string) => Promise<{ cmd: string; args: Record<string, unknown> }[]>;
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
      open: async (scenario = "library", opts = {}) => {
        await page.addInitScript((o) => (window.__QC_MOCK__ = o), { scenario, ...opts });
        await page.addInitScript({ path: MOCK_BUNDLE });
        await page.goto(ui === "legacy" ? "/legacy/" : "/");
      },
      calls: (cmd, method) =>
        page.evaluate(([c, m]) => window.__qc!.calls.filter((x) => (!c || x.cmd === c) && (!m || x.args.method === m)), [cmd, method] as const),
      core: (fn) => page.evaluate((src) => new Function("core", `return (${src})(core)`)(window.__qc!.core), fn),
    };
    await use(app);
  },
});

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
