import { useEffect, useState } from "react";
import { useStore } from "../../../store";
import { api, errText, pickFolder, pickSavePath } from "../../../ipc/api";
import type { Crash, FlightReport, SessionReport } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Select } from "../../../components/Field";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { Disclosure } from "../../../components/dash/Disclosure";
import { EmptyState } from "../../../components/dash/EmptyState";
import { FactList } from "../../../components/dash/FactList";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
import { DetailDrawer, WithDrawer } from "../../../components/dash/DetailDrawer";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { checkOnDevice } from "../../../actions/session";
import { sel } from "../../../store/settings";
import { copyText, hhmm, mmss, num } from "../../../lib/flights";
import { PathText } from "../../../components/PathText";
import { RangeTrend } from "./RangeTrend";
import { useGearData } from "./useGearData";
import styles from "./Flights.module.css";

/** Gear > Flights: the flights of a day from the radio logs, and the session report. The
 *  header holds the section switch and the day. */
export function FlightsPage() {
  const segment = useStore((s) => s.gearSegment);
  const setSegment = useStore((s) => s.setGearSegment);
  const seg = segment === "report" ? "report" : "flights";
  const flights = useGearData(() => api.gearFlights());
  const days = flights.data?.days ?? [];
  const [day, setDay] = useState<string | null>(null);
  const [reportDay, setReportDay] = useState<string>("");
  const shown = day && days.includes(day) ? day : (days[0] ?? null);
  const dayPicker =
    seg === "report" ? (
      <Select aria-label="Day" value={reportDay} onChange={(e) => setReportDay(e.target.value)}>
        <option value="">Last import</option>
        {days.map((d) => (
          <option key={d} value={d}>
            {fmtDay(d, true)}
          </option>
        ))}
      </Select>
    ) : (
      shown && (
        <Select aria-label="Day" value={shown} onChange={(e) => setDay(e.target.value)}>
          {days.map((d) => (
            <option key={d} value={d}>
              {fmtDay(d, true)}
            </option>
          ))}
        </Select>
      )
    );
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Flights"
        actions={
          <>
            <SegmentedControl
              label="Sections"
              value={seg}
              onChange={setSegment}
              segments={[
                { value: "flights", label: "Flights" },
                { value: "report", label: "Session report" },
              ]}
            />
            {dayPicker}
          </>
        }
        more={[{ label: "Add log folder…", icon: "folder-open", run: () => void addFolder() }]}
        moreLabel="Flights actions"
      />
      {seg === "report" ? <Report day={reportDay} /> : <Flights data={flights.data} error={flights.error} shown={shown} />}
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

/** The day's numbers as tiles: "—" for a value the day does not have. */
function DayStats({ r }: { r: SessionReport | null }) {
  return (
    <StatRow label="Day summary">
      <Stat label="Flights" value={r ? r.flights : 0} />
      <Stat label="Air time" value={mmss(r?.air_s)} />
      <Stat label="Longest" value={mmss(r?.longest?.secs)} sub={r?.longest ? `at ${hhmm(r.longest.start)}` : undefined} />
      <Stat label="Worst link" value={num(r?.worst_link?.lq, 0, "%")} sub={r?.worst_link ? `${num(r.worst_link.rssi_db, 0, "dB")} at ${hhmm(r.worst_link.start)}` : undefined} />
      <Stat label="Dropouts" value={r ? r.dropouts : 0} tone={r?.dropouts ? "warn" : undefined} sub={r?.downlink_only ? `${r.downlink_only} telemetry only` : undefined} />
    </StatRow>
  );
}

function Flights({ data, error, shown }: { data: Awaited<ReturnType<typeof api.gearFlights>> | null; error: string | null; shown: string | null }) {
  const packs = useGearData(() => api.gearPacks());
  const [open, setOpen] = useState<string | null>(null);
  const report = useGearData(() => (shown ? api.gearSessionReport(shown) : Promise.resolve(null)), [shown]);
  if (error) return <p className={styles.muted}>{error}</p>;
  if (!data) return null;
  if (!data.flights.length || !shown)
    return (
      <>
        <DayStats r={null} />
        <Unreadable data={data} />
        <EmptyState icon="stopwatch">No flights in the radio logs.</EmptyState>
      </>
    );
  const list = [...data.flights.filter((f) => f.flight.day === shown)].reverse();
  const labels = (packs.data?.packs || []).filter((p) => !p.pack.retired).map((p) => p.pack.label);
  const sel = list.find((f) => f.flight.id === open) || null;
  const columns: Column<FlightReport>[] = [
    {
      key: "start",
      header: "Start",
      rowHeader: true,
      cell: (r) => (
        <button type="button" className={styles.rowButton} onClick={() => setOpen(r.flight.id === open ? null : r.flight.id)} aria-expanded={r === sel}>
          {hhmm(r.flight.start)}
        </button>
      ),
    },
    { key: "length", header: "Length", numeric: true, stack: "top", cell: (r) => mmss(r.flight.secs) },
    { key: "aircraft", header: "Aircraft", optional: true, cell: (r) => r.aircraft || r.flight.model || "–" },
    { key: "pack", header: "Pack", stack: "top", cell: (r) => <PackPicker r={r} labels={labels} /> },
    { key: "used", header: "Used", numeric: true, cell: (r) => num(r.flight.mah, 0, "mAh") },
    { key: "resting", header: "Resting", numeric: true, cell: (r) => num(r.flight.resting_v, 2, "V") },
    { key: "link", header: "Worst link", numeric: true, cell: (r) => `${num(r.flight.worst_lq, 0, "%")} · ${num(r.flight.worst_rssi_db, 0, "dB")}` },
    { key: "dropouts", header: "Dropouts", numeric: true, cell: (r) => <span className={r.flight.dropouts.length ? styles.warn : undefined}>{r.flight.dropouts.length}</span> },
  ];
  return (
    <>
      <DayStats r={report.data && report.data.days.includes(shown) ? report.data : null} />
      <Unreadable data={data} />
      <WithDrawer drawer={sel && <FlightDetail r={sel} onClose={() => setOpen(null)} />}>
        <DataTable label={`Flights on ${shown}`} columns={columns} rows={list} rowKey={(r) => r.flight.id} />
        <RangeTrend places={data.places} />
      </WithDrawer>
    </>
  );
}

/** The radio logs that failed to read. The next read of the page tries them again. */
function Unreadable({ data }: { data: Awaited<ReturnType<typeof api.gearFlights>> }) {
  const bad = data.unreadable ?? [];
  if (!bad.length) return null;
  return (
    <Disclosure summary={<span className={styles.warn}>{`${bad.length} log${bad.length === 1 ? "" : "s"} could not be read`}</span>}>
      <ul className={styles.unreadable}>
        {bad.map((u) => (
          <li key={u.path}>
            <PathText path={u.path} />
            <span className={`${styles.muted} selectable`}>{u.error}</span>
          </li>
        ))}
      </ul>
    </Disclosure>
  );
}

/** The flight's pack as a compact pop-up button; "None" names the pack QuadCam suggests. */
function PackPicker({ r, labels }: { r: FlightReport; labels: string[] }) {
  const at = hhmm(r.flight.start);
  const setPack = async (pack: string) => {
    try {
      await api.gearFlightSet(r.flight.id, pack);
    } catch (e) {
      toast(errText(e), true);
    }
  };
  return (
    <Select className={styles.popup} aria-label={`Pack for ${at}`} value={r.pack || ""} onChange={(e) => setPack(e.target.value)}>
      <option value="">{r.suggested_pack ? `None (next: ${r.suggested_pack})` : "None"}</option>
      {labels.map((l) => (
        <option key={l} value={l}>
          {l}
        </option>
      ))}
      {r.pack && !labels.includes(r.pack) && <option value={r.pack}>{r.pack}</option>}
    </Select>
  );
}

/** The selected flight in the drawer: hover, sag and the other facts, the clip, and its
 *  dropouts. */
function FlightDetail({ r, onClose }: { r: FlightReport; onClose: () => void }) {
  const f = r.flight;
  const openDetail = useStore((s) => s.openDetail);
  const facts = [
    { label: "Hover", value: `${num(f.hover.all, 0, "%")} · first third ${num(f.hover.early, 0, "%")} · last third ${num(f.hover.late, 0, "%")}` },
    { label: "Sag (p5 / min)", value: `${num(f.sag_p5_v, 2)} / ${num(f.sag_min_v, 2, "V")}` },
    { label: "Resting voltage", value: `${num(f.resting_v, 2, "V")}${f.resting_from === "next_arm" ? " (first reading of the next flight)" : ""}` },
    ...(r.threshold_mah != null ? [{ label: `Warning at ${num(r.threshold_mah, 0, "mAh")}`, value: r.crossed_at_s != null ? `after ${mmss(r.crossed_at_s)}` : "Not reached" }] : []),
    { label: "Most current", value: num(f.max_current_a, 1, "A") },
    { label: "Place", value: r.place || "–" },
    {
      label: "Clip",
      value: r.clip ? (
        <button type="button" className={styles.rowButton} onClick={() => openDetail(r.clip!, null, "flight")}>
          {r.clip_name || r.clip}
        </button>
      ) : (
        "None"
      ),
    },
    { label: "Log", value: f.file.split("/").pop(), selectable: true },
  ];
  type Drop = (typeof f.dropouts)[number];
  const link = (l: Drop["before"]) => `${num(l.lq, 0, "%")}, ${num(l.rssi_db, 0, "dB")}, ${num(l.tx_power_mw, 0, "mW")}`;
  const drops: Column<Drop>[] = [
    { key: "at", header: "At", numeric: true, cell: (d) => mmss(d.start_s) },
    { key: "len", header: "Length", numeric: true, cell: (d) => num(d.secs, 1, "s") },
    { key: "before", header: "Before (LQ, RSSI, power)", cell: (d) => link(d.before) },
    { key: "after", header: "After", cell: (d) => (d.after ? link(d.after) : "Log ended") },
    { key: "kind", header: "Kind", cell: (d) => (d.downlink_only ? "Telemetry only" : "Link lost") },
  ];
  return (
    <DetailDrawer title={`Flight at ${hhmm(f.start)}`} subtitle={[mmss(f.secs), r.aircraft || f.model, r.pack].filter(Boolean).join(" · ")} onClose={onClose}>
      <FactList facts={facts} />
      {f.dropouts.length > 0 && <DataTable label="Dropouts" compact columns={drops} rows={f.dropouts} rowKey={(d) => String(d.start_s)} />}
    </DetailDrawer>
  );
}

function Report({ day }: { day: string }) {
  const { data, error } = useGearData(() => api.gearSessionReport(day || null), [day]);
  const onDeviceOn = useStore(sel.onDevice);
  const onDeviceReady = useStore((x) => x.onDeviceReady);
  // The on-device summary of the day shown, with its report's Markdown.
  const [summary, setSummary] = useState<{ day: string; text: string; markdown: string } | null>(null);
  const [summarizing, setSummarizing] = useState(false);
  useEffect(() => {
    if (onDeviceOn) checkOnDevice();
  }, [onDeviceOn]);
  if (error) return <p className={styles.muted}>{error}</p>;
  if (!data) return null;
  const shown = summary && summary.day === day ? summary : null;
  const summarize = async () => {
    setSummarizing(true);
    try {
      const r = await api.gearSessionReport(day || null, true);
      if (r.summary) setSummary({ day, text: r.summary, markdown: r.markdown });
      else toast(r.summary_unavailable || "No summary.", true);
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setSummarizing(false);
    }
  };
  return (
    <>
      <div className={styles.bar}>
        <span className={styles.grow} />
        {onDeviceOn && onDeviceReady && !shown && (
          <Button icon="sparkle" disabled={summarizing} onClick={summarize}>
            {summarizing ? "Summarizing…" : "Summarize day"}
          </Button>
        )}
        <Button
          icon="share"
          onClick={async () => {
            const ok = await copyText(shown ? shown.markdown : data.markdown);
            toast(ok ? "Copied the report as Markdown." : "Could not copy the report.", !ok);
          }}
        >
          Copy as Markdown
        </Button>
        <Button
          icon="import"
          onClick={async () => {
            const path = await pickSavePath("Save the session report", `session-report-${data.days[0] ?? "last-import"}.md`, [
              { name: "Markdown", extensions: ["md"] },
            ]);
            if (!path) return;
            try {
              await api.gearSessionReportSave(day || null, path, !!shown);
              toast("Saved the report.");
            } catch (e) {
              toast(errText(e), true);
            }
          }}
        >
          Save…
        </Button>
      </div>
      <ReportView r={data} summary={shown?.text} />
    </>
  );
}

type PackRow = { pack: string; flights: number; mah: number | null };

export function ReportView({ r, summary }: { r: SessionReport; summary?: string }) {
  const packRows: PackRow[] = [...r.packs.map((p) => ({ pack: p.pack, flights: p.flights, mah: p.mah })), ...(r.no_pack > 0 ? [{ pack: "None", flights: r.no_pack, mah: null }] : [])];
  const packCols: Column<PackRow>[] = [
    { key: "pack", header: "Pack", rowHeader: true, cell: (p) => p.pack },
    { key: "flights", header: "Flights", numeric: true, cell: (p) => p.flights },
    { key: "used", header: "Used", numeric: true, cell: (p) => num(p.mah, 0, "mAh") },
  ];
  const crashCols: Column<Crash>[] = [
    { key: "aircraft", header: "Aircraft", rowHeader: true, cell: (c) => c.aircraft || "Unknown aircraft" },
    { key: "broke", header: "Broke", cell: (c) => c.broke || "Crash" },
    { key: "parts", header: "Parts", cell: (c) => (c.parts?.length ? c.parts.join(", ") : "–") },
    { key: "repaired", header: "Repaired", cell: (c) => (c.repaired ? "Yes" : "No") },
  ];
  return (
    <section className={styles.report} aria-label="Session report">
      <h3 className={styles.reportTitle}>{r.days.length ? r.days.map((d) => fmtDay(d, true)).join(", ") : "No flights"}</h3>
      {summary && (
        <p className={`${styles.summary} selectable`} aria-label="Summary of the day">
          {summary}
        </p>
      )}
      <StatRow label="Report summary">
        {r.clips > 0 && <Stat label="Clips imported" value={r.clips} />}
        <Stat label="Flights" value={r.flights} />
        <Stat label="Air time" value={mmss(r.air_s)} />
        <Stat label="Longest" value={mmss(r.longest?.secs)} sub={r.longest ? `at ${hhmm(r.longest.start)}${r.longest.model ? ` (${r.longest.model})` : ""}` : undefined} />
        <Stat label="Worst link" value={num(r.worst_link?.lq, 0, "%")} sub={r.worst_link ? `RSSI ${num(r.worst_link.rssi_db, 0, "dB")} at ${hhmm(r.worst_link.start)}` : undefined} />
        <Stat label="Dropouts" value={r.dropouts} tone={r.dropouts ? "warn" : undefined} sub={r.downlink_only ? `${r.downlink_only} telemetry only` : undefined} />
      </StatRow>
      {packRows.length > 0 && (
        <SectionCard title="Packs" label="Packs used">
          <DataTable label="Pack use" columns={packCols} rows={packRows} rowKey={(p) => p.pack} />
        </SectionCard>
      )}
      {r.crashes.length > 0 && (
        <SectionCard title="Crashes" label="Crashes in the report">
          <DataTable label="Crashes" columns={crashCols} rows={r.crashes} rowKey={(c) => c.id} />
        </SectionCard>
      )}
    </section>
  );
}
