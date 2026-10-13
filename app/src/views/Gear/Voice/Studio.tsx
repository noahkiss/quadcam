// The ElevenLabs section of the Voices page (design 7.4): the key (kept in the macOS
// Keychain), the account's voices and models, the credits, the line sets with what rendering
// them costs, a Sample that renders a few hard lines per voice and model into the Samples
// gallery (the section's last part), and a Render that puts the sets into a local voice pack.
// Every paid call asks first.
import { useEffect, useState, type ReactNode } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { SelectField, TextField } from "../../../components/Field";
import { BrowseVoices } from "./BrowseVoices";
import { playPreview } from "./play";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { api, errText, pickFolder } from "../../../ipc/api";
import type { ImportReport, ModelInfo, RenderPacksReport, Retake, SetInfo, StudioEstimate, VoiceEstimate, VoiceInfo, VoiceProgress } from "../../../ipc/types";
import { LIST_HEIGHT, takesOf, type UseStudio } from "./useStudio";
import { modelName } from "./names";
import styles from "./Voice.module.css";

const n = (v: number) => v.toLocaleString("en-US");

const toggle = (list: string[], id: string, on: boolean) => (on ? [...list, id] : list.filter((x) => x !== id));

/** The key's state for the card header: the hint and Replace…, Remove key. */
function KeyActions({ s, onReplace }: { s: UseStudio; onReplace: () => void }) {
  if (!s.key?.set) return null;
  return (
    <>
      <Button size="sm" variant="secondary" onClick={onReplace}>
        Replace…
      </Button>
      <Button size="sm" variant="danger-ghost" onClick={() => s.deleteKey()}>
        Remove key
      </Button>
    </>
  );
}

function KeyForm({ s, replacing, done }: { s: UseStudio; replacing: boolean; done: () => void }) {
  const [draft, setDraft] = useState("");
  return (
    <form
      className={styles.bar}
      onSubmit={(e) => {
        e.preventDefault();
        const k = draft.trim();
        setDraft("");
        done();
        if (k) void s.saveKey(k);
      }}
    >
      <TextField label="ElevenLabs API key" type="password" autoComplete="off" autoFocus={replacing} value={draft} onChange={(e) => setDraft(e.target.value)} />
      <Button type="submit" size="sm" variant="primary" disabled={!draft.trim()}>
        Save key
      </Button>
      {replacing && (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            setDraft("");
            done();
          }}
        >
          Keep current
        </Button>
      )}
      {s.key?.problem && <span className={styles.error}>{s.key.problem}</span>}
    </form>
  );
}

const fmtDay = (d: string) => new Date(`${d}T12:00:00`).toLocaleDateString(undefined, { month: "short", day: "numeric" });

const word = (x?: string | null) => (x ? x.replace(/_/g, " ") : "");
const cap = (x: string) => x.charAt(0).toUpperCase() + x.slice(1);
const uniq = (xs: string[]) => [...new Set(xs.filter(Boolean))].sort();

/** The voice table's filters: name or label text, accent, language. */
interface VoiceFilter {
  text: string;
  accent: string;
  language: string;
}

function matches(v: VoiceInfo, f: VoiceFilter) {
  const text = f.text.trim().toLowerCase();
  if (text && !`${v.name} ${v.labels} ${v.category}`.toLowerCase().includes(text)) return false;
  if (f.accent && v.accent !== f.accent) return false;
  if (f.language && !(v.languages ?? []).includes(f.language)) return false;
  return true;
}

function Pickers({ s, onError }: { s: UseStudio; onError: (e: string) => void }) {
  const cat = s.catalog!;
  const v0 = s.voices[0];
  const m0 = s.models.length === 1 ? s.models[0] : undefined;
  const [f, setF] = useState<VoiceFilter>({ text: "", accent: "", language: "" });
  const [browsing, setBrowsing] = useState(false);
  const accents = uniq(cat.voices.map((v) => v.accent ?? ""));
  const langs = uniq(cat.voices.flatMap((v) => v.languages ?? []));
  const shown = cat.voices.filter((v) => matches(v, f));
  const voiceCols: Column<VoiceInfo>[] = [
    {
      key: "name",
      header: "Voice",
      rowHeader: true,
      cell: (v) => (
        <span className={styles.cell}>
          {v.name}
          {v.rate != null && <Badge tone="warn">{v.rate}× credits</Badge>}
        </span>
      ),
    },
    { key: "accent", header: "Accent", cell: (v) => word(v.accent) },
    { key: "language", header: "Language", cell: (v) => (v.languages ?? []).join(", ") },
    { key: "tags", header: "Tags", optional: true, cell: (v) => <span className={styles.muted}>{[v.category, v.gender, word(v.age), word(v.use_case)].filter(Boolean).join(", ")}</span> },
    { key: "done", header: "Rendered", numeric: true, cell: (v) => n(takesOf(s.takes, v.id, m0)) },
    {
      key: "play",
      header: "",
      cell: (v) =>
        v.preview_url ? (
          <Button size="sm" variant="ghost" icon="play" aria-label={`Play a preview of ${v.name}`} onClick={() => void playPreview(v.preview_url!).catch((e) => onError(errText(e)))} />
        ) : null,
    },
  ];
  const modelCols: Column<ModelInfo>[] = [
    {
      key: "name",
      header: "Model",
      rowHeader: true,
      cell: (m) => (
        <>
          {m.name}
          <span className={styles.sub}>{m.id}</span>
        </>
      ),
    },
    {
      key: "price",
      header: "Per 1K",
      numeric: true,
      cell: (m) => (
        <span className={styles.price}>
          {m.promo_until && <Badge tone="accent">Promo to {fmtDay(m.promo_until)}</Badge>}${m.usd_per_1k ?? "?"}
        </span>
      ),
    },
    { key: "credits", header: "Credits/char", numeric: true, optional: true, cell: (m) => `${m.cost_per_char ?? "?"}` },
    { key: "done", header: "Rendered", numeric: true, cell: (m) => (v0 ? n(takesOf(s.takes, v0, m.id)) : "—") },
  ];
  return (
    <div className={styles.pickers}>
      <h4 className={styles.subtitle}>Voices</h4>
      <div className={styles.voices}>
        <div className={styles.bar} role="search" aria-label="Filter voices">
          <TextField label="Filter" inline value={f.text} onChange={(e) => setF({ ...f, text: e.target.value })} />
          <SelectField label="Accent" inline value={f.accent} onChange={(e) => setF({ ...f, accent: e.target.value })}>
            <option value="">Any</option>
            {accents.map((a) => (
              <option key={a} value={a}>
                {cap(a)}
              </option>
            ))}
          </SelectField>
          <SelectField label="Language" inline value={f.language} onChange={(e) => setF({ ...f, language: e.target.value })}>
            <option value="">Any</option>
            {langs.map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </SelectField>
          <span className={styles.grow} />
          <Button size="sm" variant="secondary" onClick={() => setBrowsing(true)}>
            Browse voices…
          </Button>
        </div>
      <DataTable
        label="Voices"
        compact
        columns={voiceCols}
        rows={shown}
        rowKey={(v) => v.id}
        selection={{ selected: (v) => s.voices.includes(v.id), onChange: (v, on) => s.setVoices(toggle(s.voices, v.id, on)), label: (v) => v.name }}
        empty="No voice matches the filter."
        maxHeight={LIST_HEIGHT}
      />
      </div>
      {browsing && <BrowseVoices onClose={() => setBrowsing(false)} onAdded={s.refreshCatalog} />}
      <h4 className={styles.subtitle}>Models</h4>
      <DataTable
        label="Models"
        compact
        columns={modelCols}
        rows={cat.models}
        rowKey={(m) => m.id}
        selection={{ selected: (m) => s.models.includes(m.id), onChange: (m, on) => s.setModels(toggle(s.models, m.id, on)), label: (m) => `${m.name} (${m.id})` }}
      />
    </div>
  );
}

export function costText(e: VoiceEstimate) {
  const left = e.remaining === null ? "" : ` of ${n(e.remaining)} left`;
  const rate = (e.usd_per_1k ?? 0) > 0 ? ` at $${e.usd_per_1k} per 1K${e.promo_until ? `, promo until ${e.promo_until}` : ""}` : "";
  const basis = e.credits_basis === "recorded" ? "recorded rate" : "estimated";
  return `${n(e.chars)} characters in ${e.batches} ${e.batches === 1 ? "batch" : "batches"} (${e.cached_batches} cached): $${(e.usd ?? 0).toFixed(2)}${rate}, ${n(e.credits)} credits (${basis})${left}.`;
}

/** Where a set goes: made for the aircraft flown (0), for any aircraft (1), for others (2). */
function rank(x: SetInfo, aircraft: string[]) {
  const k = x.kinds ?? [];
  if (k.length === 0) return 1;
  return k.every((a) => aircraft.includes(a)) ? 0 : 2;
}

/** The set the others sit under. */
const FULL = "edgetx";

/** Distinct card paths of some sets. */
function distinct(paths: Record<string, string[]> | null, ids: string[]) {
  if (!paths) return null;
  const all = new Set<string>();
  for (const id of ids) for (const p of paths[id] ?? []) all.add(p);
  return all.size;
}

interface SetRow {
  x: SetInfo;
  /** The rows under Full EdgeTX English. */
  child: boolean;
}

/** The line sets: Full EdgeTX English with the sets it holds under it (the aircraft flown
 *  first), then the sets of their own. The parent's box is mixed while only some sets under
 *  it are ticked, and its Lines column counts the distinct lines ticked of all it holds. */
function LineSets({ s }: { s: UseStudio }) {
  const [paths, setPaths] = useState<Record<string, string[]> | null>(null);
  useEffect(() => {
    let live = true;
    api
      .gearVoiceSetPaths()
      .then((p) => live && setPaths(p))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  const shown = s.sets.filter((x) => x.id !== "sample" && !(x.id === "custom" && x.lines === 0));
  const full = shown.find((x) => x.id === FULL);
  const kids = shown
    .map((x, i) => ({ x, i }))
    .filter(({ x }) => full && x.part_of === FULL)
    .sort((a, b) => rank(a.x, s.aircraft) - rank(b.x, s.aircraft) || a.i - b.i)
    .map(({ x }) => x);
  const kidIds = kids.map((x) => x.id);
  const own = shown.filter((x) => x.id !== FULL && !kidIds.includes(x.id));
  const rows: SetRow[] = [...(full ? [{ x: full, child: false }] : []), ...kids.map((x) => ({ x, child: true })), ...own.map((x) => ({ x, child: false }))];
  const fullOn = s.picked.includes(FULL);
  const group = [FULL, ...kidIds];
  const isOn = (r: SetRow) => s.picked.includes(r.x.id) || (r.child && fullOn);
  const mixed = (r: SetRow) => r.x.id === FULL && !fullOn && kidIds.some((id) => s.picked.includes(id));
  const change = (r: SetRow, on: boolean) => {
    if (r.x.id === FULL) return s.setPicked(on ? [...new Set([...s.picked, ...group])] : s.picked.filter((id) => !group.includes(id)));
    if (r.child && fullOn && !on) return s.setPicked([...new Set([...s.picked.filter((id) => id !== FULL), ...kidIds])].filter((id) => id !== r.x.id));
    s.setPicked(toggle(s.picked, r.x.id, on));
  };
  const ticked = distinct(
    paths,
    group.filter((id) => s.picked.includes(id) || (fullOn && id !== FULL)),
  );
  const total = distinct(paths, group);
  const cols: Column<SetRow>[] = [
    {
      key: "set",
      header: "Set",
      rowHeader: true,
      cell: ({ x, child }) => (
        <span className={child ? styles.childSet : undefined}>
          {x.title}
          <span className={styles.about}>{x.about}</span>
        </span>
      ),
    },
    {
      key: "lines",
      header: "Lines",
      numeric: true,
      width: "8rem",
      cell: ({ x }) => (x.id === FULL && ticked !== null && total !== null ? `${n(ticked)} of ${n(total)}` : n(x.lines)),
    },
    { key: "done", header: "Rendered", numeric: true, width: "6rem", cell: ({ x }) => (x.rendered == null ? "—" : n(x.rendered)) },
  ];
  return (
    <DataTable
      label="Line sets"
      columns={cols}
      rows={rows}
      rowKey={({ x }) => x.id}
      selection={{ selected: isOn, mixed, onChange: change, label: ({ x }) => x.title }}
    />
  );
}

/** The estimate as tiles: lines to render, characters, cost, credits after. */
function EstimateStats({ e }: { e: RenderPacksReport }) {
  const x = e.estimate;
  const sum = (f: (p: StudioEstimate) => number) => e.pairs.reduce((n, p) => n + f(p), 0);
  const left = sum((p) => p.to_render ?? p.estimate.lines);
  const all = sum((p) => p.lines);
  const retakes = sum((p) => p.retakes ?? 0);
  const after = x.remaining === null ? null : x.remaining - x.credits;
  const basis = x.credits_basis === "recorded" ? "recorded rate" : "estimated";
  const many = e.pairs.length > 1;
  return (
    <StatRow label="Cost" role="status">
      <Stat label="To render" value={n(left)} sub={`of ${n(all)} lines${many ? ` in ${e.pairs.length} packs` : ""} · ${n(sum((p) => p.rendered ?? 0))} rendered${retakes ? ` · ${n(retakes)} re-takes` : ""}`} tone={left === 0 ? "ok" : undefined} />
      <Stat label="Characters" value={n(x.chars)} sub={`${x.batches} ${x.batches === 1 ? "batch" : "batches"}, ${x.cached_batches} cached`} />
      <Stat label="Cost" value={`$${(x.usd ?? 0).toFixed(2)}`} sub={(x.usd_per_1k ?? 0) > 0 ? `$${x.usd_per_1k} per 1K characters` : many ? "Models at different rates" : undefined} badge={x.promo_until ? <Badge tone="accent">Promo to {fmtDay(x.promo_until)}</Badge> : undefined} />
      <Stat label="Credits after" value={after === null ? "—" : n(after)} sub={`${n(x.credits)} credits (${basis})`} tone={x.affordable ? undefined : "error"} />
    </StatRow>
  );
}

/** With several voices or models: what each pack costs. */
function CostByPack({ e, models }: { e: RenderPacksReport; models?: ModelInfo[] | null }) {
  const cols: Column<StudioEstimate>[] = [
    {
      key: "voice",
      header: "Voice",
      rowHeader: true,
      cell: (p) => (
        <span className={styles.cell}>
          {p.voice_name}
          {p.rate != null && <Badge tone="warn">{p.rate}× credits</Badge>}
        </span>
      ),
    },
    { key: "model", header: "Model", cell: (p) => <span className={styles.muted}>{modelName(p.model, models)}</span> },
    { key: "left", header: "To render", numeric: true, cell: (p) => n(p.to_render ?? p.estimate.lines) },
    { key: "chars", header: "Characters", numeric: true, cell: (p) => n(p.estimate.chars) },
    { key: "usd", header: "Cost", numeric: true, cell: (p) => `$${(p.estimate.usd ?? 0).toFixed(2)}` },
    { key: "credits", header: "Credits", numeric: true, optional: true, cell: (p) => n(p.estimate.credits) },
  ];
  return <DataTable label="Cost by pack" compact columns={cols} rows={e.pairs} rowKey={(p) => `${p.voice}|${p.model}`} />;
}

/** A paid run going on: batches done of total, the pack it is on, and Stop. */
function RunProgress({ p, models, onStop }: { p: VoiceProgress; models?: ModelInfo[] | null; onStop: () => void }) {
  const what = p.kind === "sample" ? "Sampling" : "Rendering";
  const on = p.voice_name ? ` · ${p.voice_name}, ${modelName(p.model, models)}` : "";
  return (
    <div className={styles.bar} role="status" aria-label="Progress">
      <progress className={styles.progress} aria-label={`${what} batches`} max={Math.max(1, p.total)} value={p.done} />
      <span className={styles.status}>
        {p.step === "Rendering" ? `Batch ${n(p.done)} of ${n(p.total)}` : p.step}
        {on}
      </span>
      <Button size="sm" disabled={p.stopping} onClick={onStop}>
        {p.stopping ? "Stopping…" : "Stop"}
      </Button>
    </div>
  );
}

/** What a render made: a line per pack, and why it ended early. */
function RenderResult({ r }: { r: RenderPacksReport }) {
  const packs = r.packs ?? [];
  return (
    <>
      {packs.length > 0 && (
        <ul className={styles.packs} aria-label="Rendered packs">
          {packs.map((p) => (
            <li key={p.pack} className={styles.status}>
              {p.voice}: {p.rendered} {p.rendered === 1 ? "batch" : "batches"} made, {p.from_cache} from the cache, into {p.pack}.
            </li>
          ))}
        </ul>
      )}
      {r.stopped && (
        <p className={styles.status} role="status">
          Stopped after {packs.length} of {r.pairs.length} {r.pairs.length === 1 ? "pack" : "packs"}. The batches already made are not billed again.
        </p>
      )}
      {r.error && (
        <Banner kind="error" icon="danger-triangle">
          Stopped after {packs.length} of {r.pairs.length} {r.pairs.length === 1 ? "pack" : "packs"}: {r.error}
        </Banner>
      )}
    </>
  );
}

function Retakes({ s, retakes }: { s: UseStudio; retakes: Retake[] }) {
  return (
    <Banner
      kind="warning"
      icon="danger-triangle"
      action={
        <Button size="sm" disabled={s.busy} onClick={() => s.retake()}>
          Re-take
        </Button>
      }
    >
      {retakes.length === 1 ? "1 line needs" : `${retakes.length} lines need`} a re-take and {retakes.length === 1 ? "is" : "are"} not in the pack:
      <ul aria-label="Needs a re-take">
        {retakes.map((t, i) => (
          <li key={`${i}|${t.path}`}>
            {t.text} ({t.path}): {t.reason}
          </li>
        ))}
      </ul>
    </Banner>
  );
}

const SPEEDS = [0.8, 0.9, 1, 1.1, 1.2];

function importText(r: ImportReport) {
  const left = r.skipped.length ? `, ${r.skipped.length} left out` : "";
  return `${r.imported} ${r.imported === 1 ? "take" : "takes"} imported, ${r.already} in the cache already${left}.`;
}

/** The ElevenLabs section. `s` comes from the page, which also shows its summary; `children`
 *  (the Samples) close the section. */
export function Studio({ s, collapse, children }: { s: UseStudio; collapse?: { open: boolean; onToggle: (open: boolean) => void }; children?: ReactNode }) {
  const [replacing, setReplacing] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const keySet = s.key?.set ?? false;
  const e = s.estimate;
  const ready = s.voices.length > 0 && s.models.length > 0 && s.picked.length > 0;
  const pairs = s.voices.length * s.models.length;
  const ask = s.asking === "sample" ? s.sample?.estimate : s.asking === "render" ? s.render?.estimate : null;
  const retakes = (s.render?.packs ?? []).flatMap((p) => p.retakes ?? []);
  return (
    <SectionCard
      title="ElevenLabs"
      label="ElevenLabs"
      status={keySet && !replacing ? <Badge tone="ok">Key {s.key?.hint}</Badge> : undefined}
      actions={replacing ? undefined : <KeyActions s={s} onReplace={() => setReplacing(true)} />}
      collapse={collapse}
    >
      {(!keySet || replacing) && <KeyForm s={s} replacing={replacing} done={() => setReplacing(false)} />}
      {(s.error || previewError) && (
        <Banner kind="error" icon="danger-triangle">
          {s.error ?? previewError}
        </Banner>
      )}
      {keySet && s.catalog && (
        <>
          <Pickers s={s} onError={setPreviewError} />
          <LineSets s={s} />
          {e ? (
            <EstimateStats e={e} />
          ) : (
            <p className={styles.status} role="status" aria-label="Cost">
              {ready ? "Working out the cost…" : "Pick a voice, a model and a line set to see what a render costs."}
            </p>
          )}
          {e && e.pairs.length > 1 && <CostByPack e={e} models={s.catalog?.models} />}
          {e && !e.estimate.affordable && (
            <p className={styles.error} role="alert">
              The account does not have enough credits.
            </p>
          )}
          <div className={styles.bar}>
            <SelectField label="Speed" inline value={String(s.speed)} onChange={(ev) => s.setSpeed(Number(ev.target.value))}>
              {SPEEDS.map((x) => (
                <option key={x} value={String(x)}>
                  {x.toFixed(1)}
                </option>
              ))}
            </SelectField>
            <Button size="sm" disabled={s.busy || s.voices.length === 0 || s.models.length === 0} onClick={() => s.run("sample", false)}>
              Sample
            </Button>
            <Button size="sm" variant="primary" disabled={s.busy || !ready || !(e?.estimate.affordable ?? true)} onClick={() => s.run("render", false)}>
              {pairs > 1 ? "Render packs" : "Render pack"}
            </Button>
            {ready && !s.picked.includes("quadcam") && (
              <span className={styles.muted} role="note">
                QuadCam's own cues need the QuadCam default set. Without it, they use the Mac voice.
              </span>
            )}
            <span className={styles.grow} />
            <Button
              size="sm"
              variant="ghost"
              icon="import"
              disabled={s.busy}
              onClick={async () => {
                const f = await pickFolder("Folder of earlier takes");
                if (f) await s.importTakes(f);
              }}
            >
              Import takes…
            </Button>
          </div>
          {s.imported && (
            <p className={styles.status} role="status" aria-label="Import">
              {importText(s.imported)}
            </p>
          )}
          {s.asking && ask && (
            <Banner
              kind="warning"
              icon="danger-triangle"
              action={
                <>
                  <Button size="sm" variant="danger" disabled={s.busy} onClick={() => s.run(s.asking!, true)}>
                    {s.asking === "sample" ? "Sample and pay" : "Render and pay"}
                  </Button>
                  <Button size="sm" onClick={s.cancel}>
                    Cancel
                  </Button>
                </>
              }
            >
              This sends text to ElevenLabs, which bills it.{s.asking === "render" && s.render && s.render.pairs.length > 1 ? ` ${s.render.pairs.length} packs.` : ""} {costText(ask)}
            </Banner>
          )}
          {s.busy && s.progress && <RunProgress p={s.progress} models={s.catalog?.models} onStop={() => void s.stopRun()} />}
          {s.render && !s.render.needs_confirm && !s.asking && <RenderResult r={s.render} />}
          {s.render && !s.render.needs_confirm && !s.asking && retakes.length > 0 && <Retakes s={s} retakes={retakes} />}
          {s.sample?.stopped && !s.asking && (
            <p className={styles.status} role="status">
              Stopped: Samples holds the pairs finished before the stop.
            </p>
          )}
        </>
      )}
      {children}
    </SectionCard>
  );
}
