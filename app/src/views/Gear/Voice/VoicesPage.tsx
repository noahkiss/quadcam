// Gear > Voices (design 7.4): the voice packs on this Mac, rendered or installed, with a sample,
// Apply to radios… and Delete; the index's packs and Render my voice; and the Voice studio.
// Packs are not per radio: a radio's Voice segment picks one of these.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Input } from "../../../components/Field";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { api, errText, fileSrc } from "../../../ipc/api";
import type { RenderReport, SetInfo, VoicePack, VoiceView } from "../../../ipc/types";
import { fmtBytes } from "../../../lib/format";
import { deviceName } from "../../../lib/gear";
import { useStore } from "../../../store";
import { ApplyToRadios } from "./ApplyToRadios";
import { DeletePack } from "./DeletePack";
import { Studio } from "./Studio";
import { takesOf, useStudio, type UseStudio } from "./useStudio";
import { useVoice, type UseVoice } from "./useVoice";
import styles from "./Voice.module.css";

/** The line a pack's Play button plays: Armed when the pack has it. */
const SAMPLE_LINE = "SOUNDS/en/armed.wav";

const fmtDay = (t: string | null | undefined) => (t ? new Date(t).toLocaleDateString(undefined, { dateStyle: "medium" }) : "");

function play(path: string) {
  const a = new Audio(fileSrc(path));
  void a.play().catch(() => undefined);
}

function Library({ v, view, sets }: { v: UseVoice; view: VoiceView; sets: SetInfo[] }) {
  const devices = useStore((s) => s.devices);
  const [applying, setApplying] = useState<VoicePack | null>(null);
  const [deleting, setDeleting] = useState<VoicePack | null>(null);
  const packs = view.packs.filter((k) => k.installed);
  const setTitle = (id: string) => sets.find((s) => s.id === id)?.title ?? id;
  const radioName = (id: string) => {
    const d = devices.find((x) => x.id === id);
    return d ? deviceName(d) : id;
  };
  const sample = async (k: VoicePack) => {
    const lines = view.lines.filter((l) => l.packs.includes(k.id));
    const line = lines.find((l) => l.path === SAMPLE_LINE) ?? lines[0];
    if (!line) return v.setError(`${k.voice} holds no line QuadCam knows.`);
    try {
      play(await api.gearVoicePreview({ line: line.path, pack: k.id, radio: null }));
    } catch (e) {
      v.setError(errText(e));
    }
  };
  const cols: Column<VoicePack>[] = [
    {
      key: "voice",
      header: "Voice",
      rowHeader: true,
      cell: (k) => (
        <>
          {k.voice}
          <span className={styles.sub}>
            {k.id}
            {k.stale ? " · from older lines" : ""}
          </span>
          {(k.retakes?.length ?? 0) > 0 && (
            <ul className={styles.retakes} aria-label={`${k.voice} needs a re-take`}>
              {(k.retakes ?? []).map((t) => (
                <li key={t.path}>
                  Needs a re-take: {t.text} ({t.path}): {t.reason}
                </li>
              ))}
            </ul>
          )}
        </>
      ),
    },
    { key: "source", header: "Source", cell: (k) => <Badge tone={k.local ? "accent" : "neutral"}>{k.local ? "Rendered here" : "Installed"}</Badge> },
    { key: "model", header: "Model", optional: true, cell: (k) => k.model || k.provider },
    { key: "sets", header: "Line sets", optional: true, cell: (k) => (k.sets ?? []).map(setTitle).join(", ") || "Some lines" },
    { key: "lines", header: "Lines", numeric: true, cell: (k) => k.lines },
    { key: "size", header: "Size", numeric: true, optional: true, cell: (k) => fmtBytes(k.bytes) },
    { key: "date", header: "Date", optional: true, cell: (k) => fmtDay(k.made) },
    { key: "radios", header: "Radios", cell: (k) => (k.radios ?? []).map(radioName).join(", ") || <span className={styles.muted}>None</span> },
    {
      key: "actions",
      header: <span className={styles.hidden}>Actions</span>,
      cell: (k) => (
        <div className={styles.rowActions}>
          <Button size="sm" variant="ghost" icon="play" aria-label={`Play a sample of ${k.voice}`} onClick={() => sample(k)} />
          <Button size="sm" onClick={() => setApplying(k)}>
            Apply to radios…
          </Button>
          <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Delete ${k.voice}`} onClick={() => setDeleting(k)} />
        </div>
      ),
    },
  ];
  return (
    <SectionCard title="Voice packs on this Mac" label="Library">
      <DataTable label="Voice packs on this Mac" columns={cols} rows={packs} rowKey={(k) => k.id} empty={<p className={styles.muted}>No voice pack on this Mac. Render one in the Voice studio, or install one below.</p>} />
      {applying && <ApplyToRadios pack={applying} onClose={() => setApplying(null)} onStaged={v.refresh} />}
      {deleting && (
        <DeletePack
          pack={deleting}
          radios={(deleting.radios ?? []).map(radioName)}
          onClose={async (takes) => {
            const k = deleting;
            setDeleting(null);
            if (takes !== null) await v.act(() => api.gearVoicePackDelete({ pack: k.id, takes }));
          }}
        />
      )}
    </SectionCard>
  );
}

const reportText = (r: RenderReport) => {
  const p = r.plan;
  if (r.needs_confirm) return `Not rendered: ${p.chars} characters would go to ${r.provider}, which may charge for them.`;
  if (r.dry_run) return `Would render ${p.to_render} of ${p.lines} lines with ${r.provider}; ${p.cached} come from the cache; ${p.chars} characters${r.paid ? ", which may be charged" : ""}.`;
  return `Rendered ${r.rendered} of ${p.lines} lines into ${r.pack}; ${r.from_cache} came from the cache.`;
};

/** The index's packs to install, and a render of QuadCam's lines with the provider from the
 *  settings. */
function GetPacks({ v, view }: { v: UseVoice; view: VoiceView }) {
  const [voice, setVoice] = useState("");
  const [report, setReport] = useState<RenderReport | null>(null);
  const available = view.packs.filter((k) => !k.installed);
  const render = async (dry: boolean, confirm = false) => {
    const r = await v.act(() => api.gearVoiceRender({ voice, lines: [], dry_run: dry, confirm, settings: null }));
    if (r) setReport(r);
  };
  return (
    <SectionCard title="Get voice packs">
      {available.length > 0 && (
        <ul className={styles.packs} aria-label="Packs in the index">
          {available.map((k) => (
            <li key={k.id} className={styles.pack}>
              <span className={styles.grow}>
                <strong>{k.voice}</strong> ({k.id}) · {k.lines} lines · {fmtBytes(k.bytes)}
                {k.stale ? " · from older lines" : ""}
                {k.license ? ` · ${k.license}` : ""}
              </span>
              <Button size="sm" icon="import" aria-label={`Install ${k.voice}`} onClick={() => v.act(() => api.gearVoicePackInstall({ pack: k.id, source: null }))}>
                Install
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className={styles.bar}>
        <Button size="sm" icon="refresh" onClick={() => v.act(v.refresh)}>
          Refresh packs
        </Button>
      </div>
      <div className={styles.bar} role="group" aria-label="Render my voice">
        <label className={styles.field}>
          My voice
          <Input aria-label="My voice" placeholder={view.provider.voice || "the provider's default"} value={voice} onChange={(e) => setVoice(e.target.value)} />
        </label>
        <Button size="sm" onClick={() => render(true)}>
          Check cost
        </Button>
        <Button size="sm" variant="primary" disabled={!view.provider.ready} onClick={() => render(false)}>
          Render my voice
        </Button>
        {report?.needs_confirm && (
          <Button size="sm" variant="danger" onClick={() => render(false, true)}>
            Render and pay
          </Button>
        )}
      </div>
      {!view.provider.ready && (
        <p className={styles.error} role="alert">
          The voice provider is not ready: {view.provider.problem ?? "unknown"}
        </p>
      )}
      {report && (
        <p className={styles.status} role="status">
          {reportText(report)}
        </p>
      )}
    </SectionCard>
  );
}

/** The page's summary: the key and credits, the packs on this Mac, the lines each voice has. */
function Summary({ s, view }: { s: UseStudio; view: VoiceView | null }) {
  const c = s.catalog?.credits;
  const packs = view?.packs.filter((k) => k.installed) ?? [];
  const local = packs.filter((k) => k.local).length;
  const radios = new Set(packs.flatMap((k) => k.radios ?? [])).size;
  const name = (id: string) => s.catalog?.voices.find((v) => v.id === id)?.name ?? id;
  const voices = [...new Set(s.takes.map((t) => t.voice))].map((v) => ({ v, lines: takesOf(s.takes, v) })).sort((a, b) => b.lines - a.lines);
  const left = c && c.limit > 0 ? c.remaining / c.limit : undefined;
  return (
    <StatRow label="Summary">
      {s.key?.set ? (
        <Stat label="ElevenLabs credits" value={c ? n(c.remaining) : "—"} sub={c ? `of ${n(c.limit)} · ${c.tier} plan` : "Key stored"} meter={left} tone={left !== undefined && left < 0.1 ? "warn" : undefined} />
      ) : (
        <Stat label="ElevenLabs credits" value="No key" sub="Add a key in the Voice studio" />
      )}
      <Stat label="Packs on this Mac" value={n(packs.length)} sub={`${local} rendered here · ${packs.length - local} installed`} />
      <Stat label="Radios with a pack" value={n(radios)} />
      <Stat
        label="Lines rendered"
        value={voices.length ? n(voices[0].lines) : "0"}
        sub={voices.length ? voices.slice(0, 3).map((x) => `${name(x.v)} · ${n(x.lines)}`).join("  ") : "No takes yet"}
      />
    </StatRow>
  );
}

const n = (v: number) => v.toLocaleString("en-US");

export function VoicesPage() {
  const v = useVoice(null, true);
  const s = useStudio(() => v.refresh());
  return (
    <div className={styles.voices} aria-labelledby="voices-title">
      <h2 id="voices-title" className={styles.title}>
        Voices
      </h2>
      <Summary s={s} view={v.view} />
      {v.error && (
        <Banner kind="error" icon="danger-triangle">
          {v.error}
        </Banner>
      )}
      {v.view && <Library v={v} view={v.view} sets={s.sets} />}
      <Studio s={s} />
      {v.view && <GetPacks v={v} view={v.view} />}
    </div>
  );
}
