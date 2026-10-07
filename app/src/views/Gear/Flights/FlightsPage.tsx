import { useState } from "react";
import { useStore } from "../../../store";
import { api, errText, pickFolder } from "../../../ipc/api";
import type { FlightReport, SessionReport } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Select, SelectField } from "../../../components/Field";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { copyText, hhmm, mmss, num } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import g from "../Gear.module.css";
import styles from "./Flights.module.css";

/** Gear > Flights: the flights of a day from the radio logs, and the session report. */
export function FlightsPage() {
  const segment = useStore((s) => s.gearSegment);
  const setSegment = useStore((s) => s.setGearSegment);
  const seg = segment === "report" ? "report" : "flights";
  return (
    <>
      <h2 className={g.title}>Flights</h2>
      <SegmentedControl
        label="Sections"
        value={seg}
        onChange={setSegment}
        segments={[
          { value: "flights", label: "Flights" },
          { value: "report", label: "Session report" },
        ]}
      />
      {seg === "report" ? <Report /> : <Flights />}
    </>
  );
}

async function addFolder() {
  const dir = await pickFolder("Add a folder of radio logs");
  if (!dir) return;
  try {
    await api.gearFlightFolders(dir);
  } catch (e) {
    toast(errText(e), true);
  }
}

function Flights() {
  const { data, error } = useGearData(() => api.gearFlights());
  const packs = useGearData(() => api.gearPacks());
  const [day, setDay] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  if (error) return <p className={styles.muted}>{error}</p>;
  if (!data) return null;
  if (!data.flights.length)
    return (
      <>
        <p className={g.empty}>No flights in the radio logs.</p>
        <div>
          <Button icon="folder-open" onClick={addFolder}>
            Add log folder…
          </Button>
        </div>
      </>
    );
  const shown = day && data.days.includes(day) ? day : data.days[0];
  const list = data.flights.filter((f) => f.flight.day === shown);
  const labels = (packs.data?.packs || []).filter((p) => !p.pack.retired).map((p) => p.pack.label);
  const sel = list.find((f) => f.flight.id === open) || null;
  return (
    <>
      <div className={styles.bar}>
        <SelectField label="Day" value={shown} onChange={(e) => setDay(e.target.value)}>
          {data.days.map((d) => (
            <option key={d} value={d}>
              {fmtDay(d, true)}
            </option>
          ))}
        </SelectField>
        <span className={styles.grow} />
        <Button icon="folder-open" onClick={addFolder}>
          Add log folder…
        </Button>
      </div>
      <div className={styles.tableWrap}>
        <table className={styles.table} aria-label={`Flights on ${shown}`}>
          <thead>
            <tr>
              <th scope="col">Start</th>
              <th scope="col">Length</th>
              <th scope="col">Aircraft</th>
              <th scope="col">Pack</th>
              <th scope="col">Hover</th>
              <th scope="col">Sag (p5 / min)</th>
              <th scope="col">Resting</th>
              <th scope="col">Used</th>
              <th scope="col">Worst link</th>
              <th scope="col">Dropouts</th>
            </tr>
          </thead>
          <tbody>
            {[...list].reverse().map((r) => (
              <Row key={r.flight.id} r={r} labels={labels} selected={r === sel} onOpen={() => setOpen(r.flight.id === open ? null : r.flight.id)} />
            ))}
          </tbody>
        </table>
      </div>
      {sel && <FlightDetail r={sel} />}
    </>
  );
}

function Row({ r, labels, selected, onOpen }: { r: FlightReport; labels: string[]; selected: boolean; onOpen: () => void }) {
  const f = r.flight;
  const at = hhmm(f.start);
  const setPack = async (pack: string) => {
    try {
      await api.gearFlightSet(f.id, pack);
    } catch (e) {
      toast(errText(e), true);
    }
  };
  return (
    <tr aria-selected={selected}>
      <td>
        <button type="button" className={styles.rowButton} onClick={onOpen} aria-expanded={selected}>
          {at}
        </button>
      </td>
      <td>{mmss(f.secs)}</td>
      <td>{r.aircraft || f.model || "–"}</td>
      <td>
        <Select aria-label={`Pack for ${at}`} value={r.pack || ""} onChange={(e) => setPack(e.target.value)}>
          <option value="">{r.suggested_pack ? `None (next: ${r.suggested_pack})` : "None"}</option>
          {labels.map((l) => (
            <option key={l} value={l}>
              {l}
            </option>
          ))}
          {r.pack && !labels.includes(r.pack) && <option value={r.pack}>{r.pack}</option>}
        </Select>
      </td>
      <td>{num(f.hover.all, 0, "%")}</td>
      <td>
        {num(f.sag_p5_v, 2)} / {num(f.sag_min_v, 2, "V")}
      </td>
      <td>{num(f.resting_v, 2, "V")}</td>
      <td>{num(f.mah, 0, "mAh")}</td>
      <td>
        {num(f.worst_lq, 0, "%")} · {num(f.worst_rssi_db, 0, "dB")}
      </td>
      <td className={f.dropouts.length ? styles.warn : undefined}>{f.dropouts.length}</td>
    </tr>
  );
}

function FlightDetail({ r }: { r: FlightReport }) {
  const f = r.flight;
  const openDetail = useStore((s) => s.openDetail);
  return (
    <section className={styles.panel} aria-label={`Flight at ${hhmm(f.start)}`}>
      <h3>Flight at {hhmm(f.start)}</h3>
      <dl className={styles.facts}>
        <div>
          <dt>Hover</dt>
          <dd>
            {num(f.hover.all, 0, "%")} · first third {num(f.hover.early, 0, "%")} · last third {num(f.hover.late, 0, "%")}
          </dd>
        </div>
        <div>
          <dt>Resting voltage</dt>
          <dd>
            {num(f.resting_v, 2, "V")}
            {f.resting_from === "next_arm" ? " (first reading of the next flight)" : ""}
          </dd>
        </div>
        {r.threshold_mah != null && (
          <div>
            <dt>Warning at {num(r.threshold_mah, 0, "mAh")}</dt>
            <dd>{r.crossed_at_s != null ? `after ${mmss(r.crossed_at_s)}` : "Not reached"}</dd>
          </div>
        )}
        <div>
          <dt>Most current</dt>
          <dd>{num(f.max_current_a, 1, "A")}</dd>
        </div>
        <div>
          <dt>Place</dt>
          <dd>{r.place || "–"}</dd>
        </div>
        <div>
          <dt>Clip</dt>
          <dd>
            {r.clip ? (
              <button type="button" className={styles.rowButton} onClick={() => openDetail(r.clip!, null, "flight")}>
                {r.clip_name || r.clip}
              </button>
            ) : (
              "None"
            )}
          </dd>
        </div>
        <div>
          <dt>Log</dt>
          <dd className="selectable">{f.file.split("/").pop()}</dd>
        </div>
      </dl>
      {f.dropouts.length > 0 && (
        <table className={styles.table} aria-label="Dropouts">
          <thead>
            <tr>
              <th scope="col">At</th>
              <th scope="col">Length</th>
              <th scope="col">Before (LQ, RSSI, power)</th>
              <th scope="col">After</th>
              <th scope="col">Kind</th>
            </tr>
          </thead>
          <tbody>
            {f.dropouts.map((d) => (
              <tr key={d.start_s}>
                <td>{mmss(d.start_s)}</td>
                <td>{num(d.secs, 1, "s")}</td>
                <td>
                  {num(d.before.lq, 0, "%")}, {num(d.before.rssi_db, 0, "dB")}, {num(d.before.tx_power_mw, 0, "mW")}
                </td>
                <td>{d.after ? `${num(d.after.lq, 0, "%")}, ${num(d.after.rssi_db, 0, "dB")}, ${num(d.after.tx_power_mw, 0, "mW")}` : "Log ended"}</td>
                <td>{d.downlink_only ? "Telemetry only" : "Link lost"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function Report() {
  const flights = useGearData(() => api.gearFlights());
  const [day, setDay] = useState<string>("");
  const { data, error } = useGearData(() => api.gearSessionReport(day || null), [day]);
  if (error) return <p className={styles.muted}>{error}</p>;
  if (!data) return null;
  return (
    <>
      <div className={styles.bar}>
        <SelectField label="Day" value={day} onChange={(e) => setDay(e.target.value)}>
          <option value="">Last import</option>
          {(flights.data?.days || []).map((d) => (
            <option key={d} value={d}>
              {fmtDay(d, true)}
            </option>
          ))}
        </SelectField>
        <span className={styles.grow} />
        <Button
          icon="share"
          onClick={async () => {
            const ok = await copyText(data.markdown);
            toast(ok ? "Copied the report as Markdown." : "Could not copy the report.", !ok);
          }}
        >
          Copy as Markdown
        </Button>
      </div>
      <ReportView r={data} />
    </>
  );
}

export function ReportView({ r }: { r: SessionReport }) {
  return (
    <section className={styles.panel} aria-label="Session report">
      <h3>{r.days.length ? r.days.map((d) => fmtDay(d, true)).join(", ") : "No flights"}</h3>
      <dl className={styles.facts}>
        {r.clips > 0 && (
          <div>
            <dt>Clips imported</dt>
            <dd>{r.clips}</dd>
          </div>
        )}
        <div>
          <dt>Flights</dt>
          <dd>{r.flights}</dd>
        </div>
        <div>
          <dt>Air time</dt>
          <dd>{mmss(r.air_s)}</dd>
        </div>
        {r.longest && (
          <div>
            <dt>Longest flight</dt>
            <dd>
              {mmss(r.longest.secs)} at {hhmm(r.longest.start)}
              {r.longest.model ? ` (${r.longest.model})` : ""}
            </dd>
          </div>
        )}
        {r.worst_link && (
          <div>
            <dt>Worst link</dt>
            <dd>
              LQ {num(r.worst_link.lq, 0, "%")}, RSSI {num(r.worst_link.rssi_db, 0, "dB")} at {hhmm(r.worst_link.start)}
            </dd>
          </div>
        )}
        <div>
          <dt>Dropouts</dt>
          <dd>
            {r.dropouts}
            {r.downlink_only ? ` (${r.downlink_only} telemetry only)` : ""}
          </dd>
        </div>
      </dl>
      {(r.packs.length > 0 || r.no_pack > 0) && (
        <table className={styles.table} aria-label="Pack use">
          <thead>
            <tr>
              <th scope="col">Pack</th>
              <th scope="col">Flights</th>
              <th scope="col">Used</th>
            </tr>
          </thead>
          <tbody>
            {r.packs.map((p) => (
              <tr key={p.pack}>
                <td>{p.pack}</td>
                <td>{p.flights}</td>
                <td>{num(p.mah, 0, "mAh")}</td>
              </tr>
            ))}
            {r.no_pack > 0 && (
              <tr>
                <td>None</td>
                <td>{r.no_pack}</td>
                <td>–</td>
              </tr>
            )}
          </tbody>
        </table>
      )}
      {r.crashes.length > 0 && (
        <ul className={styles.rows} aria-label="Crashes">
          {r.crashes.map((c) => (
            <li key={c.id}>
              <span />
              <b>{c.aircraft || "Unknown aircraft"}</b>
              <span>
                {c.broke || "Crash"}
                {c.parts?.length ? `; parts: ${c.parts.join(", ")}` : ""}
                {c.repaired ? " (repaired)" : ""}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
