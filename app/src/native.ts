// Window behaviour that makes the web view feel like a Mac app.

declare global {
  interface Window {
    /** Set by the app on macOS 26 and later, where the window draws glass chrome. */
    __QUADCAM_GLASS__?: boolean;
  }
}

/** Whether the window has the overlay title bar and the glass sidebar (macOS 26 and later). */
export function onGlass() {
  return !!window.__QUADCAM_GLASS__;
}

/** Marks the page for the glass chrome: `html[data-glass]` makes the sidebar transparent over
 * the glass and moves the toolbar beside it. Call before the first render. */
export function markGlass(doc: Document = document) {
  if (onGlass()) doc.documentElement.dataset.glass = "";
}

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
