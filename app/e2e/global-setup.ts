import { buildSync } from "esbuild";
import { fileURLToPath } from "node:url";

/** Bundles the mock runtime once; each page gets it as an init script. */
export const MOCK_BUNDLE = fileURLToPath(new URL("../.e2e/mock.js", import.meta.url));

export default function globalSetup() {
  buildSync({
    entryPoints: [fileURLToPath(new URL("./mock-entry.ts", import.meta.url))],
    bundle: true,
    format: "iife",
    target: "safari16",
    outfile: MOCK_BUNDLE,
    logLevel: "warning",
  });
}
