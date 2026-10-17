// The spot on the page where the native map shows. The app lays an Apple MapKit view over the
// web view (src-tauri/src/mapview.rs); this element holds its place and reports its rectangle
// when it changes: frame by frame while something may move it, then not at all. The map hides while anything covers the spot (a dialog, a menu, a
// toast), since a native view always draws above the page. In a browser (the mock core) the
// element shows its own placeholder.
import { useEffect, useEffectEvent, useRef } from "react";
import { api } from "../../ipc/api";
import { on } from "../../ipc/events";
import type { MapFrame, MapPin, MapStyle } from "../../bindings";
import { Icon } from "../../components/Icon";
import styles from "./MapView.module.css";

/** Overlays that can sit over the map without covering its centre. */
const OVERLAYS = 'dialog[open], [role="menu"], [role="listbox"], [role="tooltip"], [role="status"]';

function intersects(a: DOMRect, b: DOMRect) {
  return a.width > 0 && a.height > 0 && a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/** Whether something drawn by the page covers part of `el`, which the native map would hide. */
export function covered(el: HTMLElement, r: DOMRect, doc: Document = document): boolean {
  const pts: [number, number][] = [
    [r.left + r.width / 2, r.top + r.height / 2],
    [r.left + 4, r.top + 4],
    [r.right - 4, r.top + 4],
    [r.left + 4, r.bottom - 4],
    [r.right - 4, r.bottom - 4],
  ];
  for (const [x, y] of pts) {
    const hit = doc.elementFromPoint?.(x, y);
    if (hit && !el.contains(hit)) return true;
  }
  for (const o of doc.querySelectorAll<HTMLElement>(OVERLAYS)) {
    if (el.contains(o)) continue;
    const st = getComputedStyle(o);
    if (st.visibility === "hidden" || st.display === "none") continue;
    // A toast fades in from opacity 0: its text says whether it shows.
    if (o.getAttribute("role") === "status" ? !o.textContent : st.opacity === "0") continue;
    if (intersects(o.getBoundingClientRect(), r)) return true;
  }
  return false;
}

/** The map's frame in CSS pixels, or null to hide it. */
export function frameOf(el: HTMLElement | null, doc: Document = document): MapFrame | null {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width < 1 || r.height < 1 || covered(el, r, doc)) return null;
  const px = (v: number) => Math.round(v * 2) / 2;
  return { x: px(r.left), y: px(r.top), w: px(r.width), h: px(r.height), page_w: doc.documentElement.clientWidth || window.innerWidth };
}

// The native map outlives the page: it keeps its region when the page comes back.
let fitted = false;
/** The pins and look last sent: the same again is not sent (a library reload, a settings change). */
let sent = "";

/** Frames without a change before the page stops following the element. */
const SETTLE = 30;
/** A check this often catches a move that no event announced. */
const SLOW_MS = 500;
const MOVERS = ["transitionrun", "transitionend", "animationstart", "animationend"];


interface Props {
  pins: MapPin[];
  style: MapStyle;
  /** Bumped by Show all: the map zooms to every pin. */
  fitSeq: number;
  onPick: (id: string) => void;
}

export function MapHost({ pins, style, fitSeq, onPick }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const pick = useEffectEvent((id: string) => onPick(id));

  // Checks the element on every change to the page (a dialog, a menu or a toast opening),
  // which also lands while frames are paused. A change, a resize, a scroll or a transition
  // then follows it frame by frame until it has kept still for SETTLE frames.
  useEffect(() => {
    let last = "";
    let raf = 0;
    let still = 0;
    const check = () => {
      const f = frameOf(ref.current);
      const key = JSON.stringify(f);
      if (key === last) return false;
      last = key;
      api.mapFrame(f).catch(() => {});
      return true;
    };
    const tick = () => {
      raf = 0;
      still = check() ? 0 : still + 1;
      if (still < SETTLE) raf = requestAnimationFrame(tick);
    };
    const follow = () => {
      still = 0;
      if (!raf) raf = requestAnimationFrame(tick);
    };
    const changed = () => {
      check();
      follow();
    };
    const watch = new MutationObserver(changed);
    watch.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["open", "class", "style", "hidden"] });
    const sizes = new ResizeObserver(follow);
    if (ref.current) sizes.observe(ref.current);
    sizes.observe(document.documentElement);
    window.addEventListener("resize", follow);
    document.addEventListener("scroll", follow, true);
    for (const t of MOVERS) document.addEventListener(t, follow, true);
    const slow = setInterval(check, SLOW_MS);
    changed();
    return () => {
      cancelAnimationFrame(raf);
      clearInterval(slow);
      watch.disconnect();
      sizes.disconnect();
      window.removeEventListener("resize", follow);
      document.removeEventListener("scroll", follow, true);
      for (const t of MOVERS) document.removeEventListener(t, follow, true);
      api.mapFrame(null).catch(() => {});
    };
  }, []);

  const seen = useRef(fitSeq);
  useEffect(() => {
    const fit = !fitted && pins.length > 0 ? true : seen.current !== fitSeq;
    seen.current = fitSeq;
    if (fit) fitted = true;
    const key = JSON.stringify([pins, style]);
    if (key === sent && !fit) return;
    sent = key;
    api.mapPins(pins, style, fit).catch(() => {});
  }, [pins, style, fitSeq]);

  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    on("map-pick", (id) => pick(id)).then((f) => (gone ? f() : (stop = f)));
    return () => {
      gone = true;
      stop?.();
    };
  }, []);

  return (
    <div ref={ref} className={styles.host} data-map-host="">
      <div className={styles.placeholder} aria-hidden="true">
        <Icon name="map-point" size={28} tint="muted" />
      </div>
    </div>
  );
}
