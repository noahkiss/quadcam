import axe from "axe-core";

/** Axe violations in `node`. Contrast is checked in the browser (e2e/theme.spec.ts); jsdom
 * has no layout. */
export async function axeViolations(node: Element) {
  const r = await axe.run(node, { rules: { "color-contrast": { enabled: false }, region: { enabled: false } } });
  return r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.html).join(" | ")}`);
}
