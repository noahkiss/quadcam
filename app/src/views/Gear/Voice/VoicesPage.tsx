// Gear > Voices (design 7.4): the voice packs on this Mac, rendered or installed, with a sample,
// Apply to radios… and Delete; the index's packs and Render my voice; and the Voice studio.
// Packs are not per radio: a radio's Voice segment picks one of these.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Input } from "../../../components/Field";
import { api, errText, fileSrc } from "../../../ipc/api";
import type { RenderReport, SetInfo, VoicePack, VoiceView } from "../../../ipc/types";
import { fmtBytes } from "../../../lib/format";
import { deviceName } from "../../../lib/gear";
import { useStore } from "../../../store";
import { ApplyToRadios } from "./ApplyToRadios";
import { DeletePack } from "./DeletePack";
import { Studio } from "./Studio";
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
  return (
    <section className={styles.section} aria-label="Library">
      <h3>Library</h3>
      {packs.length === 0 ? (
        <p className={styles.muted}>No voice pack on this Mac. Render one in the Voice studio, or install one below.</p>
      ) : (
        <div className={styles.tableWrap}>
          <table className={styles.table} aria-label="Voice packs on this Mac">
            <thead>
              <tr>
                <th scope="col">Voice</th>
                <th scope="col">Model</th>
                <th scope="col">Line sets</th>
                <th scope="col">Lines</th>
                <th scope="col">Size</th>
                <th scope="col">Date</th>
                <th scope="col">Radios</th>
                <th scope="col">
                  <span className={styles.hidden}>Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {packs.map((k) => (
                <tr key={k.id}>
                  <th scope="row">
                    {k.voice}
                    <span className={styles.sub}>
                      {k.id}
                      {k.local ? " · rendered here" : " · installed"}
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
                  </th>
                  <td>{k.model || k.provider}</td>
                  <td>{(k.sets ?? []).map(setTitle).join(", ") || "Some lines"}</td>
                  <td>{k.lines}</td>
                  <td>{fmtBytes(k.bytes)}</td>
                  <td>{fmtDay(k.made)}</td>
                  <td>{(k.radios ?? []).map(radioName).join(", ")}</td>
                  <td>
                    <div className={styles.cell}>
                      <Button size="sm" icon="play" aria-label={`Play a sample of ${k.voice}`} onClick={() => sample(k)} />
                      <Button size="sm" onClick={() => setApplying(k)}>
                        Apply to radios…
                      </Button>
                      <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Delete ${k.voice}`} onClick={() => setDeleting(k)} />
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
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
    </section>
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
    <section className={styles.section} aria-label="Get voice packs">
      <h3>Get voice packs</h3>
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
    </section>
  );
}

export function VoicesPage() {
  const v = useVoice(null, true);
  const [sets, setSets] = useState<SetInfo[]>([]);
  useEffect(() => {
    let gone = false;
    api.gearVoiceSets().then(
      (x) => !gone && setSets(x.sets),
      () => undefined,
    );
    return () => {
      gone = true;
    };
  }, []);
  return (
    <div className={styles.voices} aria-labelledby="voices-title">
      <h2 id="voices-title" className={styles.title}>
        Voices
      </h2>
      {v.error && (
        <Banner kind="error" icon="danger-triangle">
          {v.error}
        </Banner>
      )}
      {v.view && <Library v={v} view={v.view} sets={sets} />}
      <Studio onRendered={() => v.refresh()} />
      {v.view && <GetPacks v={v} view={v.view} />}
    </div>
  );
}
