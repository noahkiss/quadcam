// Gear > Voices (design 7.4): the summary; the voice packs on this Mac, rendered or installed,
// with a sample, Apply to radios…, Publish… and Delete; the ElevenLabs section (key, voices,
// models, line sets, cost, render, and the Samples gallery); Mac and other voices (Render my
// voice with the provider from the settings); and the index's packs with their preview lines.
// Packs are not per radio: a radio's Voice segment picks one of these.
import { useState } from "react";
import { modelName, setsSummary, shortName } from "./names";
import { useStoredFlag } from "../../../lib/stored";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Input } from "../../../components/Field";
import { Badge } from "../../../components/dash/Badge";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { api, errText } from "../../../ipc/api";
import type { ModelInfo, PreviewTake, RenderReport, SetInfo, VoicePack, VoiceView } from "../../../ipc/types";
import { fmtBytes } from "../../../lib/format";
import { deviceName } from "../../../lib/gear";
import { useStore } from "../../../store";
import { ApplyToRadios } from "./ApplyToRadios";
import { DeletePack } from "./DeletePack";
import { playFile } from "./play";
import { Publish } from "./Publish";
import { SpeechBadge, SpeechCheck } from "./SpeechCheck";
import { Gallery, reasonText } from "./Gallery";
import { Studio } from "./Studio";
import { useGallery } from "./useGallery";
import { usePlayer } from "./usePlayer";
import { takesOf, useStudio, type UseStudio } from "./useStudio";
import { useVoice, type UseVoice } from "./useVoice";
import styles from "./Voice.module.css";

/** The line a pack's Play button plays: Armed when the pack has it. */
const SAMPLE_LINE = "SOUNDS/en/armed.wav";


const fmtShort = (t: string | null | undefined) => (t ? new Date(t).toLocaleDateString(undefined, { month: "short", day: "numeric" }) : "");

/** A pack's voice cell: the short name and the rest of the name; the model, where it came
 *  from, its sets and its radios; and the lines that need a re-take or carry a flag behind a
 *  disclosure. */
function PackVoice({ k, sets, models, radios }: { k: VoicePack; sets: SetInfo[]; models?: ModelInfo[] | null; radios: string[] }) {
  const retakes = k.retakes ?? [];
  const flags = k.flags ?? [];
  const count = [retakes.length ? `${n(retakes.length)} ${retakes.length === 1 ? "needs" : "need"} a re-take` : "", flags.length ? `${n(flags.length)} flagged` : ""].filter(Boolean).join(", ");
  const short = shortName(k.voice);
  const rest = k.voice.slice(short.length).replace(/^\s*-\s*/, "");
  const all = (k.sets ?? []).map((id) => sets.find((x) => x.id === id)?.title ?? id).join(", ");
  return (
    <div className={styles.packCell}>
      <span className={styles.packName}>
        <b>{short}</b>
        {rest && <span className={styles.muted}> {rest}</span>}
      </span>
      <span className={styles.packMeta}>
        <Badge tone={k.local ? "accent" : "neutral"}>{k.local ? "Rendered here" : "Installed"}</Badge>
        <span>{k.model ? modelName(k.model, models) : k.provider}</span>
        <span title={all || undefined}>{setsSummary(k.sets ?? [], sets)}</span>
        {!k.local && <span className="selectable">{k.id}</span>}
        {k.stale && <span>From older lines</span>}
        {radios.length > 0 && <span>On {radios.join(", ")}</span>}
      </span>
      {count && (
        <details className={styles.lineList}>
          <summary>{count}</summary>
          <ul aria-label={`Re-takes and flags of ${k.voice}`}>
            {retakes.map((t) => (
              <li key={`r|${t.path}`}>
                <span className={styles.lineText}>{t.text}</span>
                <span className={styles.lineWhy}>{t.reason}</span>
              </li>
            ))}
            {flags.map((f) => (
              <li key={`f|${f.line}`}>
                <span className={styles.lineText}>{f.line}</span>
                <span className={styles.lineWhy}>
                  Flagged: {reasonText(f.reason)}
                  {f.note ? ` (${f.note})` : ""}
                </span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function Library({ v, view, sets, models, onCheck, onPublish }: { v: UseVoice; view: VoiceView; sets: SetInfo[]; models?: ModelInfo[] | null; onCheck: (k: VoicePack) => void; onPublish: (k: VoicePack) => void }) {
  const devices = useStore((s) => s.devices);
  const [applying, setApplying] = useState<VoicePack | null>(null);
  const [deleting, setDeleting] = useState<VoicePack | null>(null);
  const packs = view.packs.filter((k) => k.installed);
  const radioName = (id: string) => {
    const d = devices.find((x) => x.id === id);
    return d ? deviceName(d) : id;
  };
  const sample = async (k: VoicePack) => {
    const lines = view.lines.filter((l) => l.packs.includes(k.id));
    const line = lines.find((l) => l.path === SAMPLE_LINE) ?? lines[0];
    if (!line) return v.setError(`${k.voice} holds no line QuadCam knows.`);
    try {
      playFile(await api.gearVoicePreview({ line: line.path, pack: k.id, radio: null }));
    } catch (e) {
      v.setError(errText(e));
    }
  };
  const cols: Column<VoicePack>[] = [
    {
      key: "voice",
      header: "Voice",
      rowHeader: true,
      cell: (k) => <PackVoice k={k} sets={sets} models={models} radios={(k.radios ?? []).map(radioName)} />,
    },
    { key: "lines", header: "Lines", numeric: true, cell: (k) => n(k.lines) },
    {
      key: "speech",
      header: "Speech check",
      cell: (k) => (
        <button type="button" className={styles.badgeButton} aria-label={`Open the speech check of ${k.voice}`} onClick={() => onCheck(k)}>
          <SpeechBadge pack={k} />
        </button>
      ),
    },
    { key: "size", header: "Size", numeric: true, optional: true, cell: (k) => fmtBytes(k.bytes) },
    { key: "date", header: "Date", optional: true, cell: (k) => <span className={styles.nowrap}>{fmtShort(k.made)}</span> },
    {
      key: "actions",
      header: <span className={styles.hidden}>Actions</span>,
      cell: (k) => (
        <div className={styles.rowActions}>
          <Button size="sm" variant="ghost" icon="play" aria-label={`Play a sample of ${k.voice}`} onClick={() => sample(k)} />
          <MoreMenu
            label={`More actions for ${k.voice}`}
            items={[
              { label: "Apply to radios…", run: () => setApplying(k) },
              { label: "Check speech", run: () => onCheck(k) },
              k.local ? { label: "Publish…", run: () => onPublish(k) } : null,
              { label: "Delete…", icon: "trash-bin-trash", danger: true, run: () => setDeleting(k) },
            ]}
          />
        </div>
      ),
    },
  ];
  return (
    <SectionCard title="Voice packs on this Mac" label="Library">
      <DataTable label="Voice packs on this Mac" columns={cols} rows={packs} rowKey={(k) => k.id} empty={<p className={styles.muted}>No voice pack on this Mac. Render one with ElevenLabs or a voice below, or install one.</p>} />
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

/** An index pack's preview lines: fetched on the first Preview click, then played from the
 *  cache. */
function PreviewLines({ v, pack }: { v: UseVoice; pack: VoicePack }) {
  const [takes, setTakes] = useState<PreviewTake[] | null>(null);
  const [open, setOpen] = useState(false);
  const toggle = async () => {
    if (!open && !takes) {
      try {
        setTakes(await api.gearVoicePackPreview({ pack: pack.id, source: null }));
      } catch (e) {
        return v.setError(errText(e));
      }
    }
    setOpen(!open);
  };
  return (
    <>
      <Button size="sm" variant="ghost" icon="volume-loud" aria-expanded={open} aria-label={`Preview ${pack.voice}`} onClick={toggle}>
        Preview
      </Button>
      {open && takes && (
        <ul className={styles.previewLines} aria-label={`Preview lines of ${pack.voice}`}>
          {takes.map((t) => (
            <li key={t.path}>
              <Button size="sm" icon="play" onClick={() => playFile(t.file)}>
                {t.text}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

const PROVIDERS: Record<string, string> = { say: "macOS voices", openai: "OpenAI-compatible server", elevenlabs: "ElevenLabs" };

/** Mac and other voices: a render of QuadCam's lines with the provider from the settings. */
function MyVoice({ v, view }: { v: UseVoice; view: VoiceView }) {
  const [voice, setVoice] = useState("");
  const [report, setReport] = useState<RenderReport | null>(null);
  const p = view.provider;
  const render = async (dry: boolean, confirm = false) => {
    const r = await v.act(() => api.gearVoiceRender({ voice, lines: [], dry_run: dry, confirm, settings: null, digest: confirm ? (report?.digest ?? null) : null }));
    if (r) setReport(r);
  };
  const where = [PROVIDERS[p.provider] ?? p.provider, p.provider === "openai" ? p.base_url : "", p.model].filter(Boolean).join(" · ");
  return (
    <SectionCard title="Mac and other voices" status={where}>
      <div className={styles.bar} role="group" aria-label="Render my voice">
        <label className={styles.field}>
          My voice
          <Input aria-label="My voice" placeholder={p.voice || "the provider's default"} value={voice} onChange={(e) => setVoice(e.target.value)} />
        </label>
        <Button size="sm" onClick={() => render(true)}>
          Check cost
        </Button>
        <Button size="sm" variant="primary" disabled={!p.ready} onClick={() => render(false)}>
          Render my voice
        </Button>
        {report?.needs_confirm && (
          <Button size="sm" variant="danger" onClick={() => render(false, true)}>
            Render and pay
          </Button>
        )}
      </div>
      {!p.ready && (
        <p className={styles.error} role="alert">
          The voice provider is not ready: {p.problem ?? "unknown"}
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

/** The index's packs to install. */
function GetPacks({ v, view }: { v: UseVoice; view: VoiceView }) {
  const available = view.packs.filter((k) => !k.installed);
  return (
    <SectionCard
      title="Get voice packs"
      actions={
        <Button size="sm" icon="refresh" onClick={() => v.act(v.refresh)}>
          Refresh packs
        </Button>
      }
    >
      {available.length > 0 ? (
        <ul className={styles.packs} aria-label="Packs in the index">
          {available.map((k) => (
            <li key={k.id} className={styles.pack}>
              <span className={styles.grow}>
                <strong>{k.voice}</strong> ({k.id}) · {k.model ? modelName(k.model) : k.provider} · {k.lines} lines · {fmtBytes(k.zip_bytes || k.bytes)}
                {k.stale ? " · from older lines" : ""}
                {k.license ? ` · ${k.license}` : ""}
              </span>
              {(k.preview?.length ?? 0) > 0 && <PreviewLines v={v} pack={k} />}
              <Button size="sm" icon="import" aria-label={`Install ${k.voice}`} onClick={() => v.act(() => api.gearVoicePackInstall({ pack: k.id, source: null }))}>
                Install
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className={styles.muted}>Every pack in the index is on this Mac.</p>
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
        <Stat label="ElevenLabs credits" value="No key" sub="Add a key under ElevenLabs" />
      )}
      <Stat label="Packs on this Mac" value={n(packs.length)} sub={`${local} rendered here · ${packs.length - local} installed`} />
      <Stat label="Radios with a pack" value={n(radios)} />
      <Stat
        label="Lines rendered"
        value={n(voices.reduce((t, x) => t + x.lines, 0))}
        sub={
          voices.length ? (
            <span className={styles.topVoices}>
              {voices.slice(0, 3).map((x) => (
                <span key={x.v} className={styles.topVoice}>
                  <span className={styles.topName}>{shortName(name(x.v))}</span>
                  <span className={styles.topCount}>{n(x.lines)}</span>
                </span>
              ))}
            </span>
          ) : (
            "No takes yet"
          )
        }
      />
    </StatRow>
  );
}

const n = (v: number) => v.toLocaleString("en-US");

export function VoicesPage() {
  const v = useVoice(null, true);
  const g = useGallery();
  const player = usePlayer();
  const s = useStudio(
    async () => {
      await v.refresh();
      await g.reload();
    },
    () => g.reload(),
  );
  const [checking, setChecking] = useState<string | null>(null);
  const [publishing, setPublishing] = useState<string | null>(null);
  const [elevenOpen, setElevenOpen] = useStoredFlag("quadcam.voices.elevenlabs.open", true);
  const checked = v.view?.packs.find((k) => k.id === checking && k.installed) ?? null;
  const published = v.view?.packs.find((k) => k.id === publishing && k.local) ?? null;
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
      {v.view && <Library v={v} view={v.view} sets={s.sets} models={s.catalog?.models} onCheck={(k) => setChecking(k.id)} onPublish={(k) => setPublishing(k.id)} />}
      {v.view && checked && <SpeechCheck key={checked.id} v={v} view={v.view} pack={checked} onClose={() => setChecking(null)} onFlagged={g.reload} />}
      {published && <Publish key={published.id} pack={published} onClose={() => setPublishing(null)} />}
      <Studio
        s={s}
        collapse={{
          open: elevenOpen,
          onToggle: setElevenOpen,
        }}
      >
        <Gallery g={g} player={player} voices={s.catalog?.voices} models={s.catalog?.models} />
      </Studio>
      {v.view && <MyVoice v={v} view={v.view} />}
      {v.view && <GetPacks v={v} view={v.view} />}
    </div>
  );
}
