// The Voice studio (design 7.4): the ElevenLabs key (kept in the macOS Keychain), the
// account's voices and models, the credits, the line sets with what rendering them costs,
// a Sample that renders a few hard lines per voice and model for A/B listening, and a
// Render that puts the sets into a local voice pack. Every paid call asks first.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { SelectField, TextField } from "../../../components/Field";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { fileSrc, pickFolder } from "../../../ipc/api";
import type { ImportReport, ModelInfo, RenderPacksReport, Retake, SampleItem, SetInfo, StudioEstimate, VoiceEstimate, VoiceInfo, VoiceProgress } from "../../../ipc/types";
import { takesOf, type UseStudio } from "./useStudio";
import styles from "./Voice.module.css";

const n = (v: number) => v.toLocaleString("en-US");

const toggle = (list: string[], id: string, on: boolean) => (on ? [...list, id] : list.filter((x) => x !== id));

function play(path: string) {
  const a = new Audio(fileSrc(path));
  void a.play().catch(() => undefined);
}

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

function Pickers({ s }: { s: UseStudio }) {
  const cat = s.catalog!;
  const v0 = s.voices[0];
  const m0 = s.models.length === 1 ? s.models[0] : undefined;
  const voiceCols: Column<VoiceInfo>[] = [
    { key: "name", header: "Voice", rowHeader: true, cell: (v) => v.name },
    { key: "tags", header: "Tags", optional: true, cell: (v) => <span className={styles.muted}>{[v.category, v.labels].filter(Boolean).join(", ")}</span> },
    { key: "done", header: "Rendered", numeric: true, cell: (v) => n(takesOf(s.takes, v.id, m0)) },
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
      <DataTable
        label="Voices"
        compact
        columns={voiceCols}
        rows={cat.voices}
        rowKey={(v) => v.id}
        selection={{ selected: (v) => s.voices.includes(v.id), onChange: (v, on) => s.setVoices(toggle(s.voices, v.id, on)), label: (v) => v.name }}
      />
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

function costText(e: VoiceEstimate) {
  const left = e.remaining === null ? "" : ` of ${n(e.remaining)} left`;
  const rate = (e.usd_per_1k ?? 0) > 0 ? ` at $${e.usd_per_1k} per 1K${e.promo_until ? `, promo until ${e.promo_until}` : ""}` : "";
  const basis = e.credits_basis === "recorded" ? "recorded rate" : "estimated";
  return `${n(e.chars)} characters in ${e.batches} ${e.batches === 1 ? "batch" : "batches"} (${e.cached_batches} cached): $${(e.usd ?? 0).toFixed(2)}${rate}, ${n(e.credits)} credits (${basis})${left}.`;
}

/** One line per row; with several voices or models, one play button per pair, numbered. */
function Samples({ items }: { items: SampleItem[] }) {
  const combos = [...new Map(items.map((i) => [`${i.voice}|${i.model}`, i])).values()];
  const rows = [...new Map(items.map((i) => [i.line, i.text])).entries()];
  const cell = (line: string, c: SampleItem) => items.find((i) => i.line === line && i.voice === c.voice && i.model === c.model);
  const many = combos.length > 1;
  const wrong = items.filter((i) => i.heard_ok === false);
  return (
    <div className={styles.samples} data-component="voice-samples">
      {many && (
        <ol className={styles.legend} aria-label="Voices and models">
          {combos.map((c) => (
            <li key={`${c.voice}|${c.model}`}>
              {c.voice_name}, {c.model}
            </li>
          ))}
        </ol>
      )}
      <ul className={styles.chips} aria-label="Sample voices" style={many ? { gridTemplateColumns: `repeat(auto-fill, minmax(${140 + 52 * combos.length}px, 1fr))` } : undefined}>
        {rows.map(([line, text]) => (
          <li key={line} className={styles.chip} aria-label={text}>
            {many && <span className={styles.chipText}>{text}</span>}
            {combos.map((c, n) => {
              const it = cell(line, c);
              if (!it) return null;
              const label = `Play ${text} in ${c.voice_name} with ${c.model}`;
              return many ? (
                <Button key={`${c.voice}|${c.model}`} size="sm" variant="ghost" icon="play" aria-label={label} title={`${c.voice_name}, ${c.model}`} onClick={() => play(it.file)}>
                  {n + 1}
                </Button>
              ) : (
                <Button key={`${c.voice}|${c.model}`} size="sm" variant="ghost" icon="play" aria-label={label} onClick={() => play(it.file)}>
                  {text}
                </Button>
              );
            })}
          </li>
        ))}
      </ul>
      {wrong.length > 0 && (
        <ul className={styles.retakes} aria-label="Heard wrong">
          {wrong.map((i) => (
            <li key={`${i.voice}|${i.model}|${i.line}`}>
              {i.text} in {i.voice_name}, {i.model}: the speech check heard {i.heard ? `“${i.heard}”` : "nothing"}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const PLURAL: Record<string, string> = { quad: "quads", heli: "helicopters", plane: "planes", glider: "gliders", car: "cars" };

function andList(words: string[]) {
  return words.length < 2 ? words.join("") : `${words.slice(0, -1).join(", ")} and ${words.at(-1)}`;
}

/** Where a set goes: made for the aircraft flown (0), for any aircraft (1), for others (2). */
function rank(x: SetInfo, aircraft: string[]) {
  const k = x.kinds ?? [];
  if (k.length === 0) return 1;
  return k.every((a) => aircraft.includes(a)) ? 0 : 2;
}

function LineSets({ s }: { s: UseStudio }) {
  const mine = `For your ${andList(s.aircraft.map((k) => PLURAL[k] ?? k))}`;
  const titles = [mine, "Any aircraft", "Other aircraft"];
  const rows = s.sets
    .filter((x) => x.id !== "sample" && !(x.id === "custom" && x.lines === 0))
    .map((x, i) => ({ x, r: rank(x, s.aircraft), i }))
    .sort((a, b) => a.r - b.r || a.i - b.i);
  const cols: Column<(typeof rows)[number]>[] = [
    {
      key: "set",
      header: "Set",
      rowHeader: true,
      cell: ({ x }) => (
        <>
          {x.title}
          <span className={styles.about}>{x.about}</span>
        </>
      ),
    },
    { key: "lines", header: "Lines", numeric: true, width: "5rem", cell: ({ x }) => n(x.lines) },
    { key: "done", header: "Rendered", numeric: true, width: "6rem", cell: ({ x }) => (x.rendered == null ? "—" : n(x.rendered)) },
  ];
  return (
    <DataTable
      label="Line sets"
      columns={cols}
      rows={rows}
      rowKey={({ x }) => x.id}
      group={({ r }) => titles[r]}
      selection={{ selected: ({ x }) => s.picked.includes(x.id), onChange: ({ x }, on) => s.setPicked(toggle(s.picked, x.id, on)), label: ({ x }) => x.title }}
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
function CostByPack({ e }: { e: RenderPacksReport }) {
  const cols: Column<StudioEstimate>[] = [
    { key: "voice", header: "Voice", rowHeader: true, cell: (p) => p.voice_name },
    { key: "model", header: "Model", cell: (p) => <span className={styles.muted}>{p.model}</span> },
    { key: "left", header: "To render", numeric: true, cell: (p) => n(p.to_render ?? p.estimate.lines) },
    { key: "chars", header: "Characters", numeric: true, cell: (p) => n(p.estimate.chars) },
    { key: "usd", header: "Cost", numeric: true, cell: (p) => `$${(p.estimate.usd ?? 0).toFixed(2)}` },
    { key: "credits", header: "Credits", numeric: true, optional: true, cell: (p) => n(p.estimate.credits) },
  ];
  return <DataTable label="Cost by pack" compact columns={cols} rows={e.pairs} rowKey={(p) => `${p.voice}|${p.model}`} />;
}

/** A paid run going on: batches done of total, the pack it is on, and Stop. */
function RunProgress({ p, onStop }: { p: VoiceProgress; onStop: () => void }) {
  const what = p.kind === "sample" ? "Sampling" : "Rendering";
  const on = p.voice_name ? ` · ${p.voice_name}, ${p.model}` : "";
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

/** The Voice studio card. `s` comes from the page, which also shows its summary. */
export function Studio({ s }: { s: UseStudio }) {
  const [replacing, setReplacing] = useState(false);
  const keySet = s.key?.set ?? false;
  const e = s.estimate;
  const ready = s.voices.length > 0 && s.models.length > 0 && s.picked.length > 0;
  const pairs = s.voices.length * s.models.length;
  const ask = s.asking === "sample" ? s.sample?.estimate : s.asking === "render" ? s.render?.estimate : null;
  const retakes = (s.render?.packs ?? []).flatMap((p) => p.retakes ?? []);
  return (
    <SectionCard
      title="Voice studio"
      label="Voice studio"
      status={keySet && !replacing ? <Badge tone="ok">ElevenLabs key {s.key?.hint}</Badge> : undefined}
      actions={replacing ? undefined : <KeyActions s={s} onReplace={() => setReplacing(true)} />}
    >
      {(!keySet || replacing) && <KeyForm s={s} replacing={replacing} done={() => setReplacing(false)} />}
      {s.error && (
        <Banner kind="error" icon="danger-triangle">
          {s.error}
        </Banner>
      )}
      {keySet && s.catalog && (
        <>
          <Pickers s={s} />
          <LineSets s={s} />
          {e ? (
            <EstimateStats e={e} />
          ) : (
            <p className={styles.status} role="status" aria-label="Cost">
              {ready ? "Working out the cost…" : "Pick a voice, a model and a line set to see what a render costs."}
            </p>
          )}
          {e && e.pairs.length > 1 && <CostByPack e={e} />}
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
          {s.busy && s.progress && <RunProgress p={s.progress} onStop={() => void s.stopRun()} />}
          {s.render && !s.render.needs_confirm && !s.asking && <RenderResult r={s.render} />}
          {s.render && !s.render.needs_confirm && !s.asking && retakes.length > 0 && <Retakes s={s} retakes={retakes} />}
          {s.sample?.stopped && !s.asking && (
            <p className={styles.status} role="status">
              Stopped: the samples below are the pairs finished before the stop.
            </p>
          )}
          {s.sample && s.sample.items.length > 0 && <Samples items={s.sample.items} />}
        </>
      )}
    </SectionCard>
  );
}
