// The switch map as a table: one group of rows per control, one row per position, with its
// channel values, FC modes and radio effects. The position a control is in now is marked.
import { Button } from "../../../components/Button";
import type { Live, SwitchMap } from "../../../ipc/types";
import type { MapSources } from "./hooks";
import styles from "./Switches.module.css";

const base = (p: string) => p.split("/").pop() || p;

/** The source pickers and what was read. */
export function MapSourceBar({ src, map, openCard, openModel, openDump }: { src: MapSources; map: SwitchMap | null; openCard: () => void; openModel: () => void; openDump: () => void }) {
  return (
    <div className={styles.bar}>
      <Button icon="sd-card" onClick={openCard}>
        Open card…
      </Button>
      <Button icon="folder-open" onClick={openModel}>
        Open model…
      </Button>
      <Button icon="folder-open" onClick={openDump}>
        Open dump…
      </Button>
      {(map || src.radio || src.fc.length > 0) && (
        <span className={styles.source}>{map ? [map.model, map.sources.join(", ")].filter(Boolean).join(" · ") : [src.radio, ...src.fc].filter(Boolean).map((p) => base(p!)).join(", ")}</span>
      )}
    </div>
  );
}

export function SwitchMapView({ map, live }: { map: SwitchMap; live: Live | null }) {
  return (
    <div className={styles.map}>
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
            return (
              <tbody key={r.id} className={styles.group}>
                {r.positions.map((p, i) => (
                  <tr key={p.name} aria-current={now === i ? "true" : undefined} className={now === i ? styles.now : undefined}>
                    {i === 0 && (
                      <th scope="rowgroup" rowSpan={r.positions.length} className={styles.control}>
                        {r.label}
                        {r.switch_type && <span className={styles.type}>{r.switch_type}</span>}
                      </th>
                    )}
                    <td>{p.name}</td>
                    <td className={styles.num}>{p.channels.map((c) => `CH${c.ch} ${c.us}`).join(", ")}</td>
                    <td>{p.fc.join(", ")}</td>
                    <td>{p.radio.join("; ")}</td>
                  </tr>
                ))}
              </tbody>
            );
          })}
        </table>
      )}
      {map.modes.length > 0 && (
        <section aria-label="Modes">
          <h3 className={styles.h3}>Modes</h3>
          <ul className={styles.modes}>
            {map.modes.map((m) => (
              <li key={m.slot} data-on={live?.modes.includes(m.name) ? "" : undefined}>
                <span>{m.name}</span>
                <span className={styles.num}>{m.linked ? `linked to ${m.linked}` : `AUX${m.ch - 4} (CH${m.ch}) ${m.start}-${m.end}`}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
      {map.conflicts.length > 0 && (
        <section aria-label="Conflicts">
          <h3 className={styles.h3}>Conflicts</h3>
          <ul className={styles.notes}>
            {map.conflicts.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        </section>
      )}
      {map.notes.length > 0 && (
        <ul className={[styles.notes, styles.muted].join(" ")} aria-label="Notes">
          {map.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
