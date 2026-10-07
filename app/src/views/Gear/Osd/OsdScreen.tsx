// One FC's OSD layout: a profile picker, the profile drawn on its grid in the OSD's
// monospace cells (hover an element for its name), the elements on in the profile, and the
// check. Read only; the editor (drag to move, staged as changes) comes with staged changes.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { SegmentedControl } from "../../../components/SegmentedControl";
import type { OsdView } from "../../../ipc/types";
import styles from "./OsdScreen.module.css";

interface Props {
  view: OsdView;
}

export function OsdScreen({ view }: Props) {
  const first = view.profiles.find((p) => p.active)?.index ?? 1;
  const [index, setIndex] = useState(String(first));
  const [hover, setHover] = useState<string | null>(null);
  const profile =
    view.profiles.find((p) => String(p.index) === index) ?? view.profiles[0];
  if (!profile) return null;
  const bad = new Set(
    profile.problems.flatMap((p) => [p.element, p.other ?? ""]),
  );
  const byName = new Map(view.elements.map((e) => [e.name, e]));
  const problems = view.profiles.reduce((n, p) => n + p.problems.length, 0);
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
          {view.grid.name} {view.grid.width}×{view.grid.height}
          {profile.active ? " · in use" : ""}
        </span>
      </header>
      {!view.complete && (
        <Banner kind="warning" icon="danger-triangle" tint="yellow">
          This file is a diff. Elements it leaves out keep their firmware
          defaults and are not shown.
        </Banner>
      )}
      <div className={styles.body}>
        <figure className={styles.figure}>
          <div
            className={styles.screen}
            role="img"
            aria-label={`OSD profile ${profile.index} on a ${view.grid.name} grid, ${profile.elements.length} elements`}
          >
            <div
              className={styles.cells}
              style={{
                width: `${view.grid.width}ch`,
                height: `calc(${view.grid.height} * var(--osd-line))`,
              }}
            >
              {profile.rows.map((r, y) => (
                <div
                  key={y}
                  className={styles.row}
                  style={{ top: `calc(${y} * var(--osd-line))` }}
                >
                  {r}
                </div>
              ))}
              {profile.boxes.map((b) => (
                <span
                  key={b.element}
                  className={[
                    styles.box,
                    bad.has(b.element) ? styles.bad : "",
                    hover === b.element ? styles.hot : "",
                  ].join(" ")}
                  title={b.label}
                  data-element={b.element}
                  onMouseEnter={() => setHover(b.element)}
                  onMouseLeave={() => setHover(null)}
                  style={{
                    left: `${b.x}ch`,
                    top: `calc(${b.y} * var(--osd-line))`,
                    width: `${b.width}ch`,
                    height: `calc(${b.height} * var(--osd-line))`,
                  }}
                />
              ))}
            </div>
          </div>
          <figcaption className={styles.caption}>
            {problems === 0
              ? "Every profile passes the check: no overlaps, nothing off screen."
              : `${problems} ${problems === 1 ? "problem" : "problems"} across the profiles.`}
          </figcaption>
        </figure>
        <aside className={styles.side}>
          {profile.problems.length > 0 && (
            <ul className={styles.problems} aria-label="Problems">
              {profile.problems.map((p) => (
                <li key={p.message}>{p.message}</li>
              ))}
            </ul>
          )}
          <table
            className={styles.list}
            aria-label={`Elements in profile ${profile.index}`}
          >
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
                  <tr
                    key={name}
                    className={hover === name ? styles.hotRow : undefined}
                    onMouseEnter={() => setHover(name)}
                    onMouseLeave={() => setHover(null)}
                  >
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
