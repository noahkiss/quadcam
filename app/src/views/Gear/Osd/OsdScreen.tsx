// One FC's OSD layout: a profile picker, the profile drawn on its grid in the OSD's
// monospace cells (hover an element for its name), the elements on in the profile, and the
// check. With `edit` the grid is the editor: drag an element, or focus it and press the
// arrow keys (Shift for five cells), and tick elements on or off per profile. Each edit
// goes to `edit.move`, which stages it; the screen shows what comes back.
import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Banner } from "../../../components/Banner";
import { SegmentedControl } from "../../../components/SegmentedControl";
import type { OsdElement, OsdMove, OsdView } from "../../../ipc/types";
import styles from "./OsdScreen.module.css";

export interface OsdEditing {
  /** Stage one move; the view follows when the core answers. */
  move: (m: OsdMove) => void;
  /** What the last edit did, announced to screen readers. */
  status: string;
}

interface Props {
  view: OsdView;
  edit?: OsdEditing;
}

/** Furthest cell an element can sit at (the position value's range). */
const MAX = { x: 63, y: 31 };
const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
const ARROWS: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };

interface Drag {
  name: string;
  /** Pointer position at the start. */
  x0: number;
  y0: number;
  /** Whole cells moved. */
  dx: number;
  dy: number;
}

export function OsdScreen({ view, edit }: Props) {
  const first = view.profiles.find((p) => p.active)?.index ?? 1;
  const [index, setIndex] = useState(String(first));
  const [hover, setHover] = useState<string | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const cells = useRef<HTMLDivElement>(null);
  const profile = view.profiles.find((p) => String(p.index) === index) ?? view.profiles[0];
  if (!profile) return null;
  const bad = new Set(profile.problems.flatMap((p) => [p.element, p.other ?? ""]));
  const byName = new Map(view.elements.map((e) => [e.name, e]));
  const problems = view.profiles.reduce((n, p) => n + p.problems.length, 0);
  const { width: gw, height: gh } = view.grid;

  /** Where the element would sit after `dx`, `dy` cells, within the grid (or the value's range for keys). */
  const target = (e: OsdElement, dx: number, dy: number, keys: boolean) => ({
    x: clamp(e.x + dx, 0, keys ? MAX.x : Math.min(MAX.x, gw - 1)),
    y: clamp(e.y + dy, 0, keys ? MAX.y : Math.min(MAX.y, gh - 1)),
  });

  const onKey = (e: OsdElement, ev: KeyboardEvent) => {
    const d = ARROWS[ev.key];
    if (!d || !edit) return;
    ev.preventDefault();
    const step = ev.shiftKey ? 5 : 1;
    const t = target(e, d[0] * step, d[1] * step, true);
    if (t.x !== e.x || t.y !== e.y) edit.move({ element: e.name, x: t.x, y: t.y });
  };
  const onDown = (e: OsdElement, ev: PointerEvent<HTMLDivElement>) => {
    if (!edit || ev.button !== 0) return;
    ev.currentTarget.setPointerCapture(ev.pointerId);
    ev.currentTarget.focus();
    setDrag({ name: e.name, x0: ev.clientX, y0: ev.clientY, dx: 0, dy: 0 });
  };
  const onMove = (e: OsdElement, ev: PointerEvent<HTMLDivElement>) => {
    const box = cells.current?.getBoundingClientRect();
    if (!drag || drag.name !== e.name || !box) return;
    const t = target(e, Math.round((ev.clientX - drag.x0) / (box.width / gw)), Math.round((ev.clientY - drag.y0) / (box.height / gh)), false);
    setDrag({ ...drag, dx: t.x - e.x, dy: t.y - e.y });
  };
  const onUp = (e: OsdElement) => {
    if (!drag || drag.name !== e.name) return;
    setDrag(null);
    if (drag.dx || drag.dy) edit?.move({ element: e.name, x: e.x + drag.dx, y: e.y + drag.dy });
  };

  const onList = edit ? view.elements.filter((e) => e.profiles.includes(profile.index)) : [];
  return (
    <section className={styles.osd} aria-label="OSD">
      <header className={styles.head}>
        <SegmentedControl
          label="OSD profile"
          value={index}
          onChange={setIndex}
          segments={view.profiles.map((p) => ({
            value: String(p.index),
            label: `${p.index}${p.name ? ` ${p.name}` : ""}${p.problems.length ? ` (${p.problems.length})` : ""}`,
          }))}
        />
        <span className={styles.meta}>
          {view.grid.name} {gw}×{gh}
          {profile.active ? " · in use" : ""}
        </span>
      </header>
      {!view.complete && (
        <Banner kind="warning" icon="danger-triangle" tint="yellow">
          This file is a diff. Elements it leaves out keep their firmware defaults and are not shown.
        </Banner>
      )}
      <div className={styles.body}>
        <figure className={styles.figure}>
          <div className={styles.screen} role={edit ? "group" : "img"} aria-label={`OSD profile ${profile.index} on a ${view.grid.name} grid, ${profile.elements.length} elements`}>
            <div
              ref={cells}
              className={styles.cells}
              style={{
                width: `${gw}ch`,
                height: `calc(${gh} * var(--osd-line))`,
              }}
            >
              {!edit &&
                profile.rows.map((r, y) => (
                  <div key={y} className={styles.row} style={{ top: `calc(${y} * var(--osd-line))` }}>
                    {r}
                  </div>
                ))}
              {profile.boxes.map((b) => {
                const el = byName.get(b.element);
                const moving = drag?.name === b.element ? drag : null;
                const cls = [styles.box, edit ? styles.edit : "", bad.has(b.element) ? styles.bad : "", hover === b.element ? styles.hot : "", moving ? styles.moving : ""].join(" ");
                const pos = {
                  left: `${b.x + (moving?.dx ?? 0)}ch`,
                  top: `calc(${b.y + (moving?.dy ?? 0)} * var(--osd-line))`,
                  width: `${b.width}ch`,
                  height: `calc(${b.height} * var(--osd-line))`,
                };
                if (!edit || !el) {
                  return <span key={b.element} className={cls} title={b.label} data-element={b.element} onMouseEnter={() => setHover(b.element)} onMouseLeave={() => setHover(null)} style={pos} />;
                }
                return (
                  <div
                    key={b.element}
                    role="button"
                    tabIndex={0}
                    className={cls}
                    title={b.label}
                    data-element={b.element}
                    aria-label={`${b.label}, column ${el.x}, row ${el.y}. Arrow keys move it.`}
                    onKeyDown={(ev) => onKey(el, ev)}
                    onPointerDown={(ev) => onDown(el, ev)}
                    onPointerMove={(ev) => onMove(el, ev)}
                    onPointerUp={() => onUp(el)}
                    onPointerCancel={() => setDrag(null)}
                    onMouseEnter={() => setHover(b.element)}
                    onMouseLeave={() => setHover(null)}
                    style={pos}
                  >
                    {el.height === 1 ? el.sample : ""}
                  </div>
                );
              })}
            </div>
          </div>
          <figcaption className={styles.caption}>
            {problems === 0 ? "Every profile passes the check: no overlaps, nothing off screen." : `${problems} ${problems === 1 ? "problem" : "problems"} across the profiles.`}
          </figcaption>
          {edit && (
            <p role="status" className={styles.caption}>
              {edit.status}
            </p>
          )}
        </figure>
        <aside className={styles.side}>
          {profile.problems.length > 0 && (
            <ul className={styles.problems} aria-label="Problems">
              {profile.problems.map((p) => (
                <li key={p.message}>{p.message}</li>
              ))}
            </ul>
          )}
          {edit ? (
            <Elements view={view} index={profile.index} hover={hover} setHover={setHover} edit={edit} on={onList} />
          ) : (
            <table className={styles.list} aria-label={`Elements in profile ${profile.index}`}>
              <thead>
                <tr>
                  <th scope="col">Element</th>
                  <th scope="col">X</th>
                  <th scope="col">Y</th>
                </tr>
              </thead>
              <tbody>
                {profile.elements.map((name) => {
                  const e = byName.get(name);
                  return (
                    <tr key={name} className={hover === name ? styles.hotRow : undefined} onMouseEnter={() => setHover(name)} onMouseLeave={() => setHover(null)}>
                      <td>
                        {e?.label ?? name}
                        {e && !e.known ? " (unknown width)" : ""}
                      </td>
                      <td>{e?.x}</td>
                      <td>{e?.y}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
          {[...view.notes, ...profile.notes].map((n) => (
            <p key={n} className={styles.note}>
              {n}
            </p>
          ))}
        </aside>
      </div>
    </section>
  );
}

/** The editor's element list: every element, on ones first, with its profile toggle and position. */
function Elements({ view, index, hover, setHover, edit, on }: { view: OsdView; index: number; hover: string | null; setHover: (n: string | null) => void; edit: OsdEditing; on: OsdElement[] }) {
  const onNames = new Set(on.map((e) => e.name));
  const rows = [...view.elements].sort((a, b) => Number(onNames.has(b.name)) - Number(onNames.has(a.name)) || a.label.localeCompare(b.label));
  const toggle = (e: OsdElement, checked: boolean) => edit.move({ element: e.name, profiles: checked ? [...e.profiles, index].sort() : e.profiles.filter((p) => p !== index) });
  const place = (e: OsdElement, axis: "x" | "y", raw: string) => {
    const n = Number(raw);
    if (!Number.isInteger(n) || n === e[axis]) return;
    edit.move({ element: e.name, [axis]: clamp(n, 0, MAX[axis]) });
  };
  return (
    <div className={styles.scroll} role="region" aria-label={`Elements for profile ${index}`} tabIndex={0}>
      <table className={styles.list} aria-label={`Elements in profile ${index}`}>
        <thead>
          <tr>
            <th scope="col">On</th>
            <th scope="col">Element</th>
            <th scope="col">X</th>
            <th scope="col">Y</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((e) => (
            <tr key={e.name} className={hover === e.name ? styles.hotRow : undefined} onMouseEnter={() => setHover(e.name)} onMouseLeave={() => setHover(null)}>
              <td>
                <input type="checkbox" className={styles.tick} checked={onNames.has(e.name)} aria-label={`Show ${e.label} in profile ${index}`} onChange={(ev) => toggle(e, ev.target.checked)} />
              </td>
              <td>
                {e.label}
                {!e.known ? " (unknown width)" : ""}
              </td>
              {(["x", "y"] as const).map((axis) => (
                <td key={axis}>
                  <input
                    key={`${e.name}-${axis}-${e[axis]}`}
                    type="number"
                    className={styles.num}
                    min={0}
                    max={MAX[axis]}
                    defaultValue={e[axis]}
                    aria-label={`${e.label} ${axis.toUpperCase()}`}
                    onBlur={(ev) => place(e, axis, ev.target.value)}
                    onKeyDown={(ev) => ev.key === "Enter" && place(e, axis, ev.currentTarget.value)}
                  />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
