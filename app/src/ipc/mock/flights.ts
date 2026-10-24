// The mock core's flights, packs, crashes, session report and "Pack up" check. The seed is
// the real core's answer on the synthetic log (e2e/fixtures/flights.json, written by
// `cargo test --test it flights::record_mock_fixture -- --ignored`); the edits
// follow `core/flights.rs` closely enough for the specs.
import flightsFixture from "../../../e2e/fixtures/flights.json";
import type { CheckRow, Connected, Crash, CrashSaveParams, Device, FlightsView, Pack, PackType, PacksView, Preflight, Profile, SessionReport } from "../types";
import { KIND_LABEL } from "../../lib/gear";
import { withAircraft } from "./gear";
import { FILES } from "./model";

/** The mock's clock for Pack up's backup ages, so the rows stay the same from run to run. */
const NOW = Date.parse("2026-10-09T12:00:00Z");
/** `preflight::BACKUP_MAX_DAYS`. */
const BACKUP_MAX_DAYS = 14;
/** The time in a snapshot id (`<device>/2026-10-06T182000-manual`), UTC. */
const backupTime = (id: string | null | undefined) => {
  const m = id?.match(/\/(\d{4}-\d{2}-\d{2})T(\d{2})(\d{2})(\d{2})/);
  return m ? Date.parse(`${m[1]}T${m[2]}:${m[3]}:${m[4]}Z`) : null;
};
const daysAgo = (t: number) => Math.max(0, Math.floor((NOW - t) / 86_400_000));

interface Seed {
  flights: FlightsView;
  packs: PacksView;
  report: SessionReport;
  preflight: Preflight;
}

const seed = () => structuredClone(flightsFixture) as unknown as Seed;

export class MockFlights {
  private s = seed();
  packs: Pack[] = this.s.packs.packs.map((p) => p.pack);
  types: PackType[] = this.s.packs.types.map((t) => t.pack_type);
  notes = this.s.packs.notes;
  folders: string[] = [];
  /** Logs that fail to read, as the core reports them. Specs push to it. */
  unreadable: { path: string; error: string }[] = [];
  crashes: Crash[] = [];
  /** Flight id -> pack label. */
  sets = new Map<string, string>(this.s.flights.flights.filter((f) => f.pack).map((f) => [f.flight.id, f.pack!]));

  flights(day: string | null): FlightsView {
    const v = structuredClone(this.s.flights);
    for (const f of v.flights) {
      f.pack = this.sets.get(f.flight.id) ?? null;
      if (f.pack) f.suggested_pack = null;
    }
    v.sources = [...v.sources, ...this.folders];
    v.unreadable = [...this.unreadable];
    if (day) v.flights = v.flights.filter((f) => f.flight.day === day);
    return v;
  }

  flightSet(flight: string, pack: string | null) {
    const f = this.s.flights.flights.find((x) => x.flight.id === flight);
    if (!f) throw `No flight "${flight}" in the radio logs.`;
    if (pack && !this.packs.some((p) => p.label === pack)) throw `No pack "${pack}". Save it first.`;
    if (pack) this.sets.set(flight, pack);
    else if (pack === "") this.sets.delete(flight);
    return this.flights(null).flights.find((x) => x.flight.id === flight)!;
  }

  packsView(): PacksView {
    const flights = this.s.flights.flights;
    return {
      packs: [...this.packs]
        .sort((a, b) => a.label.localeCompare(b.label, undefined, { numeric: true }))
        .map((pack) => {
          const mine = flights.filter((f) => this.sets.get(f.flight.id) === pack.label).sort((a, b) => a.flight.start.localeCompare(b.flight.start));
          const last = mine.at(-1)?.flight.end ?? null;
          const charged = !!pack.charged_at && (!last || pack.charged_at > last);
          return {
            pack,
            cycles: mine.length,
            state: charged ? "charged" : mine.length ? "flown" : "unknown",
            last_flown: last,
            median_resting_v: mine[0]?.flight.resting_v ?? null,
            median_secs: mine[0]?.flight.secs ?? null,
            weak: null,
            history: mine.map((f) => ({ flight: f.flight.id, start: f.flight.start, secs: f.flight.secs, mah: f.flight.mah, sag_min_v: f.flight.sag_min_v, sag_p5_v: f.flight.sag_p5_v, resting_v: f.flight.resting_v })),
          };
        }),
      types: this.types.map((t) => {
        const seeded = this.s.packs.types.find((x) => x.pack_type.name === t.name);
        const cells = t.cells ?? 1;
        return {
          ...(seeded || { flights: 0, median_resting_v: null, median_secs: null, suggested_warn_mah: null }),
          pack_type: t,
          packs: this.packs.filter((p) => p.pack_type === t.name).length,
          full_total_v: t.full_v != null ? t.full_v * cells : null,
          storage_total_v: t.storage_v != null ? t.storage_v * cells : null,
        };
      }),
      notes: this.notes,
      target_v: this.s.packs.target_v,
    };
  }

  packSave(pack: Pack, charged: boolean | null) {
    if (!pack.label.trim()) throw "A pack needs a label.";
    if (pack.pack_type && !this.types.some((t) => t.name === pack.pack_type)) throw `No pack type "${pack.pack_type}". Save the type first.`;
    const p = { ...pack, label: pack.label.trim() };
    if (charged === true) p.charged_at = "2026-10-07T12:00:00";
    if (charged === false) p.charged_at = null;
    this.packs = [...this.packs.filter((x) => x.label !== p.label), p];
    return p;
  }

  packDelete(label: string) {
    const p = this.packs.find((x) => x.label === label);
    if (!p) throw `No pack "${label}".`;
    this.packs = this.packs.filter((x) => x !== p);
    return p;
  }

  typeSave(t: PackType) {
    if (!t.name.trim()) throw "A pack type needs a name.";
    this.types = [...this.types.filter((x) => x.name !== t.name), { ...t, name: t.name.trim() }];
    return t;
  }

  typeDelete(name: string) {
    const users = this.packs.filter((p) => p.pack_type === name).map((p) => p.label);
    if (users.length) throw `Packs ${users.join(", ")} use type "${name}".`;
    const t = this.types.find((x) => x.name === name);
    if (!t) throw `No pack type "${name}".`;
    this.types = this.types.filter((x) => x !== t);
    return t;
  }

  report(day: string | null): SessionReport {
    const r = structuredClone(this.s.report);
    if (day && !r.days.includes(day)) return { ...r, days: [day], clips: 0, flights: 0, air_s: 0, longest: null, worst_link: null, dropouts: 0, downlink_only: 0, packs: [], no_pack: 0, crashes: [], markdown: `# Session report: ${day}\n\n- Flights: 0, air time 0:00\n- Dropouts: 0\n` };
    r.crashes = this.crashes.filter((c) => r.days.includes(c.day));
    return r;
  }

  /** `core/flights.rs` `gear_preflight` over the mock gear: the packs row from the packs, the
   *  radio, card space, backups and cards-out rows from the saved and plugged-in devices. */
  preflight(devices: Device[], connected: Connected[], profiles: Profile[]): Preflight {
    const p = structuredClone(this.s.preflight);
    const live = this.packsView().packs.filter((x) => !x.pack.retired);
    const not = live.filter((x) => x.state !== "charged").map((x) => x.pack.label);
    const charged = live.length - not.length;
    const set = (id: string, r: Partial<CheckRow>) => (p.rows = p.rows.map((x) => (x.id === id ? { ...x, ...r } : x)));
    set(
      "packs",
      live.length
        ? { state: not.length ? "warn" : "pass", detail: not.length ? `${charged} of ${live.length} charged. Not charged: ${not.join(", ")}.` : `${charged} of ${live.length} charged.` }
        : { state: "unknown", detail: "No packs saved." },
    );
    const name = (id: string | null | undefined, kind: Device["kind"]) => devices.find((d) => d.id === id)?.name?.trim() || KIND_LABEL[kind];
    const model = FILES.find((f) => f.file === "model01.yml")!.name;
    const plugged = connected.find((c) => c.kind === "radio");
    const saved = devices.find((d) => d.kind === "radio" && d.id === plugged?.id) ?? devices.find((d) => d.kind === "radio" && d.last_backup);
    if (plugged || saved) {
      const n = name(plugged?.id ?? saved!.id, "radio");
      const aircraft = saved ? withAircraft(saved, profiles, !!plugged).aircraft : null;
      const t = backupTime(saved?.last_backup);
      const age = plugged || t == null ? "" : `, from backup, ${daysAgo(t) === 1 ? "1 day" : `${daysAgo(t)} days`} ago`;
      set("radio", { state: aircraft ? "pass" : "warn", detail: aircraft ? `${n}: model ${model} selected (${aircraft})${age}.` : `${n}: model ${model} selected, not linked to an aircraft${age}.` });
    } else set("radio", { state: "unknown", detail: "No radio saved." });
    const vols = connected.filter((c) => c.link.kind === "volume");
    const cards = vols.filter((c) => c.kind === "dvr_card" || c.kind === "goggles");
    if (cards.length) set("cards_space", { state: "pass", detail: `${cards.map((c) => `${name(c.id, c.kind)}: free space unknown`).join("; ")}.` });
    const backed = devices.filter((d) => d.kind === "radio" || d.kind === "fc");
    if (backed.length) {
      const ages = backed.map((d) => [name(d.id, d.kind), backupTime(d.last_backup)] as const);
      const old = ages.some(([, t]) => t == null || daysAgo(t) > BACKUP_MAX_DAYS);
      set("backups", { state: old ? "warn" : "pass", detail: `${ages.map(([n, t]) => (t == null ? `${n}: never` : `${n}: ${daysAgo(t)} days ago`)).join("; ")}.` });
    }
    if (vols.length) set("cards_in", { state: "warn", detail: `Still in the Mac: ${vols.map((c) => name(c.id, c.kind)).join(", ")}.` });
    return p;
  }

  crashList(clip: string | null, aircraft: string | null) {
    return this.crashes.filter((c) => (!clip || c.clip === clip) && (!aircraft || c.aircraft === aircraft));
  }

  crashSave(p: CrashSaveParams, clipInfo: (id: string) => { date: string; aircraft: string | null; duration: number } | null) {
    const old = p.id ? this.crashes.find((c) => c.id === p.id) : undefined;
    if (p.id && !old) throw `No crash "${p.id}".`;
    const lib = p.clip ? clipInfo(p.clip) : null;
    if (p.clip && !lib && !old) throw `No library clip "${p.clip}".`;
    const c: Crash = old
      ? { ...old }
      : { id: `crash-${this.crashes.length + 1}`, clip: p.clip ?? null, time_s: null, aircraft: lib?.aircraft ?? null, day: lib?.date ?? "2026-10-07", broke: "", parts: [], note: "", repaired: false };
    if (p.time_s != null) c.time_s = p.time_s;
    if (p.aircraft != null) c.aircraft = p.aircraft || null;
    if (p.day) c.day = p.day;
    if (p.broke != null) c.broke = p.broke;
    if (p.parts != null) c.parts = p.parts;
    if (p.note != null) c.note = p.note;
    if (p.repaired != null) c.repaired = p.repaired;
    this.crashes = [...this.crashes.filter((x) => x.id !== c.id), c];
    return c;
  }

  crashDelete(id: string) {
    const c = this.crashes.find((x) => x.id === id);
    if (!c) throw `No crash "${id}".`;
    this.crashes = this.crashes.filter((x) => x !== c);
    return c;
  }
}
