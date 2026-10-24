// The switch map as a table: one group of rows per control, one row per position, with its
// channel values, FC modes and radio effects. The position a control is in now is marked.
import { Fragment } from "react";
import { Badge } from "../../../components/dash/Badge";
import { Disclosure } from "../../../components/dash/Disclosure";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { SectionCard } from "../../../components/dash/SectionCard";
import { StatusDot } from "../../../components/dash/StatusDot";
import type { AuxMode, Live, SwitchMap, Unmapped } from "../../../ipc/types";
import type { MapSources } from "./hooks";
import styles from "./Switches.module.css";

const base = (p: string) => p.split("/").pop() || p;

/** The FC's modes: name, channel and range; with live input, which are on. */
function modeColumns(live: Live | null): Column<AuxMode>[] {
  const cols: Column<AuxMode>[] = [
    { key: "mode", header: "Mode", rowHeader: true, cell: (m) => m.name },
    { key: "ch", header: "Channel", cell: (m) => (m.linked ? "—" : `AUX${m.ch - 4} (CH${m.ch})`) },
    { key: "range", header: "Range", numeric: true, cell: (m) => (m.linked ? `Linked to ${m.linked}` : `${m.start}–${m.end}`) },
  ];
  if (live) cols.push({ key: "now", header: "Now", width: "6rem", cell: (m) => (live.modes.includes(m.name) ? <StatusDot tone="ok">On</StatusDot> : <StatusDot tone="idle">Off</StatusDot>) });
  return cols;
}

const UNMAPPED: Column<Unmapped>[] = [
  { key: "switch", header: "Switch", rowHeader: true, width: "5rem", cell: (u) => u.switch },
  { key: "kind", header: "Kind", width: "6rem", cell: (u) => u.kind },
  { key: "condition", header: "Condition", cell: (u) => <span className={styles.mono}>{u.condition}</span> },
  { key: "reads", header: "Reads", optional: true, cell: (u) => u.reads.join(", ") || "—" },
  { key: "used", header: "Used by", cell: (u) => u.used_by.join(", ") || "—" },
];

const CONFLICT: Column<string>[] = [{ key: "conflict", header: "Conflict", cell: (c) => c }];

/** The source pop-up (a card, a model file, a dump) and what was read. */
export function MapSourceBar({ src, map, openCard, openModel, openDump }: { src: MapSources; map: SwitchMap | null; openCard: () => void; openModel: () => void; openDump: () => void }) {
  return (
    <div className={styles.bar}>
      <MoreMenu
        text="Open…"
        label="Open"
        items={[
          { label: "Open Card…", icon: "sd-card", run: openCard },
          { label: "Open Model…", icon: "folder-open", run: openModel },
          { label: "Open Dump…", icon: "folder-open", run: openDump },
        ]}
      />
      {(map || src.radio || src.fc.length > 0) && (
        <span className={styles.source}>{map ? [map.model, map.sources.join(", ")].filter(Boolean).join(" · ") : [src.radio, ...src.fc].filter(Boolean).map((p) => base(p!)).join(", ")}</span>
      )}
    </div>
  );
}

export function SwitchMapView({ map, live }: { map: SwitchMap; live: Live | null }) {
  return (
    <div className={styles.map}>
      {map.conflicts.length > 0 && (
        <SectionCard title="Conflicts" status={<Badge tone="warn">{map.conflicts.length}</Badge>}>
          <DataTable label="Conflicts" columns={CONFLICT} rows={map.conflicts} rowKey={(c) => c} compact />
        </SectionCard>
      )}
      {live && (
        <p className={styles.live} aria-live="polite">
          <strong>{live.source === "fc" ? "FC" : "Radio"}</strong>
          {" · Modes on: "}
          {live.modes.length ? live.modes.join(", ") : "none"}
          {live.adjustments.length > 0 && ` · ${live.adjustments.join(", ")}`}
        </p>
      )}
      {map.rows.length > 0 && (
        <table className={styles.table} aria-label="Switch map">
          <thead>
            <tr>
              <th scope="col">Control</th>
              <th scope="col">Position</th>
              <th scope="col">Channels</th>
              <th scope="col">FC</th>
              <th scope="col">Radio</th>
            </tr>
          </thead>
          {map.rows.map((r) => {
            const now = live?.positions[r.id] ?? null;
            const span = r.positions.reduce((n, p) => n + 1 + p.combos.length, 0);
            return (
              <tbody key={r.id} className={styles.group} aria-label={r.label}>
                {r.positions.map((p, i) => (
                  <Fragment key={p.name}>
                    <tr aria-current={now === i ? "true" : undefined} className={now === i ? styles.now : undefined}>
                      {i === 0 && (
                        <th scope="rowgroup" rowSpan={span} className={styles.control}>
                          {r.label}
                          {r.switch_type && <span className={styles.type}>{r.switch_type}</span>}
                        </th>
                      )}
                      <td>{p.name}</td>
                      <td className={styles.num}>{p.channels.map((c) => `CH${c.ch} ${c.us}`).join(", ")}</td>
                      <td>{p.fc.join(", ")}</td>
                      <td>{p.radio.join("; ")}</td>
                    </tr>
                    {p.combos.map((c) => (
                      <tr key={c.with.join("+")} className={styles.combo}>
                        <td>
                          {p.name} with {c.with.join(" and ")}
                        </td>
                        <td className={styles.num}>{c.channels.map((v) => `CH${v.ch} ${v.us}`).join(", ")}</td>
                        <td>{c.fc.join(", ")}</td>
                        <td>{c.radio.join("; ")}</td>
                      </tr>
                    ))}
                  </Fragment>
                ))}
              </tbody>
            );
          })}
        </table>
      )}
      {map.modes.length > 0 && (
        <section aria-label="Modes">
          <Disclosure summary="Modes" count={map.modes.length}>
            <DataTable label="Modes" columns={modeColumns(live)} rows={map.modes} rowKey={(m) => String(m.slot)} compact />
          </Disclosure>
        </section>
      )}
      {map.unmapped.length > 0 && (
        <section aria-label="Not mapped">
          <Disclosure summary="Not mapped" count={map.unmapped.length}>
            <DataTable label="Not mapped" columns={UNMAPPED} rows={map.unmapped} rowKey={(u) => u.switch} compact />
            <p className={styles.muted}>Logical switches on telemetry, timers or sticky state. The map cannot say when they are on.</p>
          </Disclosure>
        </section>
      )}
      {map.notes.length > 0 && (
        <section aria-label="Notes">
          <Disclosure summary="Notes" count={map.notes.length}>
            <ul className={[styles.notes, styles.muted].join(" ")}>
              {map.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          </Disclosure>
        </section>
      )}
    </div>
  );
}
