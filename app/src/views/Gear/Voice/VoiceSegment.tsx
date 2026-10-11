// The radio's Voice segment (design 7.4, WP9): which pack the radio uses (Choose voice, which
// stages one card change), the lines QuadCam knows with the text a voice speaks, and a
// per-line override. Packs are rendered and installed on the Voices page. Nothing reaches the
// card until the apply sheet writes it.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Checkbox, Input, Select } from "../../../components/Field";
import { api, errText, fileSrc } from "../../../ipc/api";
import type { EditCost, StagedChange, VoiceLine, VoiceView } from "../../../ipc/types";
import { useStore } from "../../../store";
import type { DeviceRef } from "../slots";
import { useVoice } from "./useVoice";
import styles from "./Voice.module.css";

/** The title prefix of the change Choose voice keeps (`core/voice.rs`). */
const VOICE_CHANGE = "Voice: ";

const GROUPS = ["all", "callouts", "numbers", "system", "units", "extras"];

function StagedVoice({ radio }: { radio: string | null }) {
  const change: StagedChange | undefined = useStore((s) => s.changes.find((c) => c.device === radio && c.title.startsWith(VOICE_CHANGE) && (c.status === "draft" || c.status === "ready")));
  const open = useStore((s) => s.openApply);
  const discard = useStore((s) => s.discardChange);
  if (!radio || !change) return null;
  const n = change.edits.reduce((k, e) => k + (e.kind === "card_files" ? e.put.length : 0), 0);
  return (
    <Banner
      kind="info"
      icon="info"
      tint="blue"
      action={
        <>
          <Button size="sm" onClick={() => open(radio, change.id)}>
            Review…
          </Button>
          <Button size="sm" variant="danger-ghost" onClick={() => discard(change.id)}>
            Undo voice change
          </Button>
        </>
      }
    >
      {change.title} is staged: {n === 1 ? "1 sound" : `${n} sounds`} to put on the card. The radio changes when you apply it.
    </Banner>
  );
}

function OpenVoices() {
  const open = useStore((s) => s.openGear);
  return (
    <Button size="sm" onClick={() => open({ page: "slot", id: "voices" })}>
      Open Voices
    </Button>
  );
}

function Choose({ radio, view, act }: { radio: string; view: VoiceView; act: <T>(f: () => Promise<T>) => Promise<T | undefined> }) {
  const installed = view.packs.filter((k) => k.installed && (k.firmware ?? "edgetx") === "edgetx");
  const [pack, setPack] = useState("");
  const [keep, setKeep] = useState(true);
  const current = installed.some((k) => k.id === pack) ? pack : (installed[0]?.id ?? "");
  return (
    <section className={styles.section} aria-label="Choose voice">
      <h3>Choose voice</h3>
      {installed.length === 0 ? (
        <div className={styles.bar}>
          <p className={styles.muted}>No voice pack on this Mac. Render or install one on the Voices page.</p>
          <OpenVoices />
        </div>
      ) : (
        <div className={styles.bar}>
          <label className={styles.field}>
            Voice
            <Select aria-label="Voice" value={current} onChange={(e) => setPack(e.target.value)}>
              {installed.map((k) => (
                <option key={k.id} value={k.id}>
                  {k.voice} ({k.id})
                </option>
              ))}
            </Select>
          </label>
          <Checkbox label="Keep my overrides" checked={keep} onChange={(e) => setKeep(e.target.checked)} />
          <Button variant="primary" onClick={() => act(() => api.gearVoiceChoose({ radio, pack: current, keep_overrides: keep, editor: null }))}>
            Choose voice
          </Button>
          <OpenVoices />
        </div>
      )}
      {view.chosen && <p className={styles.status}>Chosen for this radio: {view.chosen}.</p>}
    </section>
  );
}

function play(path: string) {
  const a = new Audio(fileSrc(path));
  void a.play().catch(() => undefined);
}

function LineRow({ radio, line, view, act, setError }: { radio: string; line: VoiceLine; view: VoiceView; act: <T>(f: () => Promise<T>) => Promise<T | undefined>; setError: (e: string | null) => void }) {
  const [own, setOwn] = useState<string | null>(null);
  // A paid take of the person's text: its characters and the digest Render and pay sends.
  const [ask, setAsk] = useState<EditCost | null>(null);
  const packs = view.packs.filter((k) => k.installed && line.packs.includes(k.id));
  const preview = async (pack: string | null) => {
    try {
      play(await api.gearVoicePreview({ line: line.path, pack, radio: pack ? null : radio }));
    } catch (e) {
      setError(errText(e));
    }
  };
  const ov = line.override;
  const edit = (text: string, confirm: boolean, digest: string | null) => act(() => api.gearVoiceEdit({ radio, line: line.path, text, pack: null, confirm, dry_run: false, digest }));
  const close = () => {
    setOwn(null);
    setAsk(null);
  };
  const renderOwn = async (text: string) => {
    if (view.provider.paid) {
      const dry = await act(() => api.gearVoiceEdit({ radio, line: line.path, text, pack: null, confirm: false, dry_run: true, digest: null }));
      if (!dry?.cost) return;
      if (dry.cost.paid) {
        setAsk(dry.cost);
        return;
      }
    }
    if ((await edit(text, false, null)) !== undefined) close();
  };
  return (
    <tr>
      <td className={styles.path}>{line.path.replace(/^SOUNDS\//, "")}</td>
      <td>
        {line.text}
        {line.spoken !== line.text && <span className={styles.muted}> (spoken: {line.spoken})</span>}
      </td>
      <td>
        <div className={styles.cell}>
          {packs.map((k) => (
            <Button key={k.id} size="sm" icon="play" aria-label={`Play ${line.text} in ${k.voice}`} onClick={() => preview(k.id)} />
          ))}
          {ov?.kind === "text" && <Button size="sm" icon="play" aria-label={`Play ${line.text} in my text`} onClick={() => preview(null)} />}
        </div>
      </td>
      <td>
        <div className={styles.cell}>
          {ov && <span>{ov.kind === "pack" ? `take from ${ov.pack}` : `my text: ${ov.text}`}</span>}
          <Select aria-label={`Take for ${line.text}`} value="" onChange={(e) => e.target.value && act(() => api.gearVoiceEdit({ radio, line: line.path, text: null, pack: e.target.value, confirm: false, dry_run: false, digest: null }))}>
            <option value="">Use another voice…</option>
            {packs.map((k) => (
              <option key={k.id} value={k.id}>
                {k.voice}
              </option>
            ))}
          </Select>
          {own === null ? (
            <Button size="sm" onClick={() => setOwn(ov?.text ?? line.text)}>
              My text…
            </Button>
          ) : (
            <span className={styles.own}>
              <Input
                aria-label={`My text for ${line.text}`}
                value={own}
                onChange={(e) => {
                  setOwn(e.target.value);
                  setAsk(null);
                }}
              />
              {ask ? (
                <>
                  <span className={styles.muted}>
                    {ask.chars} characters go to {ask.provider}, which may charge for them.
                  </span>
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={async () => {
                      if ((await edit(own, true, ask.digest)) !== undefined) close();
                    }}
                  >
                    Render and pay
                  </Button>
                </>
              ) : (
                <Button size="sm" variant="primary" onClick={() => renderOwn(own)}>
                  Render
                </Button>
              )}
              <Button size="sm" onClick={close}>
                Cancel
              </Button>
            </span>
          )}
          {ov && (
            <Button size="sm" variant="ghost" onClick={() => act(() => api.gearVoiceEdit({ radio, line: line.path, text: null, pack: null, confirm: false, dry_run: false, digest: null }))}>
              Reset
            </Button>
          )}
        </div>
      </td>
    </tr>
  );
}

export function VoiceSegment({ d }: { d: DeviceRef }) {
  const radio = d.device?.id ?? null;
  const v = useVoice(radio);
  const [group, setGroup] = useState("all");
  const view = v.view;
  return (
    <div className={styles.segment}>
      {!radio && <p className={styles.muted}>Save this radio from Overview to choose its voice.</p>}
      <StagedVoice radio={radio} />
      {v.error && (
        <Banner kind="error" icon="danger-triangle">
          {v.error}
        </Banner>
      )}
      {view && radio && (
        <>
          <Choose radio={radio} view={view} act={v.act} />
          <section className={styles.section} aria-label="Lines">
            <h3>Lines</h3>
            <div className={styles.bar}>
              <label className={styles.field}>
                Group
                <Select aria-label="Group" value={group} onChange={(e) => setGroup(e.target.value)}>
                  {GROUPS.map((g) => (
                    <option key={g} value={g}>
                      {g}
                    </option>
                  ))}
                </Select>
              </label>
            </div>
            <div className={styles.tableWrap}>
              <table className={styles.table} aria-label="Voice lines">
                <thead>
                  <tr>
                    <th scope="col">Sound</th>
                    <th scope="col">Text</th>
                    <th scope="col">Play</th>
                    <th scope="col">On this radio</th>
                  </tr>
                </thead>
                <tbody>
                  {view.lines
                    .filter((l) => group === "all" || l.group === group)
                    .map((l) => (
                      <LineRow key={l.path} radio={radio} line={l} view={view} act={v.act} setError={v.setError} />
                    ))}
                </tbody>
              </table>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
