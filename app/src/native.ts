// Window behaviour that makes the web view feel like a Mac app.

/** The web view's own context menu (Reload, Inspect Element) shows only where it helps:
 * over a text field, or over selected text a person may copy. The app's menus call
 * `preventDefault` first. */
export function limitContextMenu(doc: Document = document) {
  const onMenu = (e: MouseEvent) => {
    if (e.defaultPrevented) return;
    const t = e.target as Element | null;
    const field = t?.closest?.('textarea, [contenteditable]:not([contenteditable="false"]), input:not([type=checkbox], [type=radio], [type=range], [type=button])');
    const copying = t?.closest?.(".selectable") && String(doc.getSelection()).trim();
    if (!field && !copying) e.preventDefault();
  };
  doc.addEventListener("contextmenu", onMenu);
  return () => doc.removeEventListener("contextmenu", onMenu);
}
