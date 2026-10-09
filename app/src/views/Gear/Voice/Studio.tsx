// The Voice studio (design 7.4): the ElevenLabs key (kept in the macOS Keychain), the
// account's voices and models, the credits, the line sets with what rendering them costs,
// a Sample that renders a few hard lines per voice and model for A/B listening, and a
// Render that puts the sets into a local voice pack. Every paid call asks first.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Checkbox, TextField } from "../../../components/Field";
import { fileSrc } from "../../../ipc/api";
import type { SampleItem, VoiceEstimate } from "../../../ipc/types";
import { useStudio, type UseStudio } from "./useStudio";
import styles from "./Voice.module.css";

const n = (v: number) => v.toLocaleString("en-US");

const toggle = (list: string[], id: string, on: boolean) => (on ? [...list, id] : list.filter((x) => x !== id));

function play(path: string) {
  const a = new Audio(fileSrc(path));
  void a.play().catch(() => undefined);
}

function KeyRow({ s }: { s: UseStudio }) {
  const [draft, setDraft] = useState("");
  if (s.key?.set) {
    const c = s.catalog?.credits;
    return (
      <div className={styles.bar}>
        <span>ElevenLabs key stored ({s.key.hint}).</span>
        {c && (
          <span role="status">
            {n(c.remaining)} credits left of {n(c.limit)}.
          </span>
        )}
        <Button size="sm" variant="danger-ghost" onClick={() => s.deleteKey()}>
          Remove key
        </Button>
      </div>
    );
  }
  return (
    <form
      className={styles.bar}
      onSubmit={(e) => {
        e.preventDefault();
        const k = draft.trim();
        setDraft("");
        if (k) void s.saveKey(k);
      }}
    >
      <TextField label="ElevenLabs API key" type="password" autoComplete="off" value={draft} onChange={(e) => setDraft(e.target.value)} />
      <Button type="submit" size="sm" variant="primary" disabled={!draft.trim()}>
        Save key
      </Button>
      {s.key?.problem && <span className={styles.error}>{s.key.problem}</span>}
    </form>
  );
}

function costText(e: VoiceEstimate) {
  const left = e.remaining === null ? "" : ` of ${n(e.remaining)} left`;
  return `${n(e.chars)} characters in ${e.batches} ${e.batches === 1 ? "batch" : "batches"} (${e.cached_batches} cached): ${n(e.credits)} credits${left}.`;
}

function Grid({ items }: { items: SampleItem[] }) {
  const cols = [...new Map(items.map((i) => [`${i.voice}|${i.model}`, i])).values()];
  const rows = [...new Map(items.map((i) => [i.line, i.text])).entries()];
  const cell = (line: string, c: SampleItem) => items.find((i) => i.line === line && i.voice === c.voice && i.model === c.model);
  return (
    <div className={styles.tableWrap}>
      <table className={styles.table} aria-label="Sample voices">
        <thead>
          <tr>
            <th scope="col">Line</th>
            {cols.map((c) => (
              <th scope="col" key={`${c.voice}|${c.model}`}>
                {c.voice_name}
                <br />
                <span className={styles.muted}>{c.model}</span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map(([line, text]) => (
            <tr key={line}>
              <th scope="row">{text}</th>
              {cols.map((c) => {
                const it = cell(line, c);
                return (
                  <td key={`${c.voice}|${c.model}`}>
                    {it && <Button size="sm" icon="play" aria-label={`Play ${text} in ${c.voice_name} with ${c.model}`} onClick={() => play(it.file)} />}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Studio({ onRendered }: { onRendered: () => Promise<void> }) {
  const s = useStudio(onRendered);
  const keySet = s.key?.set ?? false;
  const e = s.estimate;
  const one = s.voices.length === 1 && s.models.length === 1 && s.picked.length > 0;
  const ask = s.asking === "sample" ? s.sample?.estimate : s.asking === "render" ? s.render?.estimate : null;
  return (
    <section className={styles.section} aria-label="Voice studio">
      <h3>Voice studio</h3>
      <KeyRow s={s} />
      {s.error && (
        <Banner kind="error" icon="danger-triangle">
          {s.error}
        </Banner>
      )}
      {keySet && s.catalog && (
        <>
          <div className={styles.picks}>
            <fieldset className={styles.pick}>
              <legend>Voices</legend>
              {s.catalog.voices.map((v) => (
                <Checkbox key={v.id} label={v.name} detail={[v.category, v.labels].filter(Boolean).join(", ")} checked={s.voices.includes(v.id)} onChange={(ev) => s.setVoices(toggle(s.voices, v.id, ev.target.checked))} />
              ))}
            </fieldset>
            <fieldset className={styles.pick}>
              <legend>Models</legend>
              {s.catalog.models.map((m) => (
                <Checkbox key={m.id} label={m.name} detail={`${m.id}, ${m.cost_per_char ?? "?"} ${m.cost_per_char === 1 ? "credit" : "credits"} a character`} checked={s.models.includes(m.id)} onChange={(ev) => s.setModels(toggle(s.models, m.id, ev.target.checked))} />
              ))}
            </fieldset>
            <fieldset className={styles.pick}>
              <legend>Line sets</legend>
              {s.sets
                .filter((x) => x.id !== "sample" && !(x.id === "custom" && x.lines === 0))
                .map((x) => (
                  <Checkbox key={x.id} label={x.title} detail={`${x.lines} lines`} checked={s.picked.includes(x.id)} onChange={(ev) => s.setPicked(toggle(s.picked, x.id, ev.target.checked))} />
                ))}
            </fieldset>
          </div>
          <p className={e && !e.estimate.affordable ? styles.error : styles.status} role="status" aria-label="Cost">
            {e ? `${e.voice_name}, ${e.model}: ${e.lines} lines, ${costText(e.estimate)}${e.estimate.affordable ? "" : " The account does not have enough."}` : one ? "Working out the cost…" : "Pick one voice, one model and a line set to see what a render costs."}
          </p>
          <div className={styles.bar}>
            <Button size="sm" disabled={s.busy || s.voices.length === 0 || s.models.length === 0} onClick={() => s.run("sample", false)}>
              Sample
            </Button>
            <Button size="sm" variant="primary" disabled={s.busy || !one || !(e?.estimate.affordable ?? true)} onClick={() => s.run("render", false)}>
              Render pack
            </Button>
          </div>
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
              This sends text to ElevenLabs, which bills it. {costText(ask)}
            </Banner>
          )}
          {s.render && !s.render.needs_confirm && !s.asking && (
            <p className={styles.status} role="status">
              Rendered into {s.render.pack}: {s.render.rendered} {s.render.rendered === 1 ? "batch" : "batches"} made, {s.render.from_cache} from the cache.
              {s.render.warnings?.length ? ` Check: ${s.render.warnings.join("; ")}` : ""}
            </p>
          )}
          {s.sample && s.sample.items.length > 0 && <Grid items={s.sample.items} />}
        </>
      )}
    </section>
  );
}
