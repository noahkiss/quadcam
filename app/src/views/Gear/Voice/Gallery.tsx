// Gear > Voices > ElevenLabs > Samples (design 7.4): one card per voice, model and speed the
// Voice studio sampled, and one per pack on this Mac, each with its takes as an aligned grid of
// play rows. The cards wrap at any window width. Play all plays a card's takes in order (or every card's) and shows the take
// playing now; Stop ends it. A take the speech check heard wrong says so on its card. Flag marks
// a take with a reason and a note; Re-take makes a card's flagged takes again (priced first).
import { useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { SelectField, TextField } from "../../../components/Field";
import { Popover } from "../../../components/Popover";
import { Badge } from "../../../components/dash/Badge";
import type { ModelInfo, RetakeReport, SampleTake, SampleVoice, TakeFlag, VoiceInfo } from "../../../ipc/types";
import { modelName, voiceName } from "./names";
import { costText } from "./Studio";
import type { FlagInput, UseGallery } from "./useGallery";
import type { Player, Track } from "./usePlayer";
import styles from "./Gallery.module.css";

export const REASONS: [string, string][] = [
  ["mispronounced", "Mispronounced"],
  ["clipped", "Clipped"],
  ["wrong_tone", "Wrong tone"],
  ["noise", "Noise"],
  ["other", "Other"],
];

export const reasonText = (r: string) => REASONS.find(([k]) => k === r)?.[1] ?? r;

const flagText = (f: TakeFlag) => `${reasonText(f.reason)}${f.note ? `: ${f.note}` : ""}`;

/** The Flag button of one take, with its form: a reason, a note, Save and Clear. */
export function FlagControl({ text, flag, disabled, onSave }: { text: string; flag?: TakeFlag | null; disabled?: boolean; onSave: (f: FlagInput | null) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState(flag?.reason ?? "mispronounced");
  const [note, setNote] = useState(flag?.note ?? "");
  const show = () => {
    setReason(flag?.reason ?? "mispronounced");
    setNote(flag?.note ?? "");
    setOpen(!open);
  };
  return (
    <span className={styles.flagWrap}>
      <Button
        ref={anchor}
        size="sm"
        variant="ghost"
        icon="flag"
        className={flag ? styles.flagOn : undefined}
        aria-label={flag ? `Change the flag on ${text}` : `Flag ${text}`}
        aria-expanded={open}
        title={flag ? flagText(flag) : undefined}
        disabled={disabled}
        onClick={show}
      />
      {open && (
        <Popover label={`Flag ${text}`} anchor={anchor} onClose={() => setOpen(false)}>
          <form
            className={styles.flagForm}
            onSubmit={(e) => {
              e.preventDefault();
              setOpen(false);
              onSave({ reason, note: note.trim() });
            }}
          >
            <SelectField label="Reason" value={reason} onChange={(e) => setReason(e.target.value)}>
              {REASONS.map(([k, label]) => (
                <option key={k} value={k}>
                  {label}
                </option>
              ))}
            </SelectField>
            <TextField label="Note" maxLength={500} value={note} onChange={(e) => setNote(e.target.value)} />
            <div className={styles.flagActions}>
              {flag && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setOpen(false);
                    onSave(null);
                  }}
                >
                  Clear flag
                </Button>
              )}
              <Button type="submit" size="sm" variant="primary">
                Save flag
              </Button>
            </div>
          </form>
        </Popover>
      )}
    </span>
  );
}

const tracksOf = (c: SampleVoice): Track[] => c.takes.map((t) => ({ card: c.id, line: t.line, file: t.file }));

/** The account's voices and models, for readable names. */
interface Names {
  voices?: VoiceInfo[] | null;
  models?: ModelInfo[] | null;
}

function metaOf(c: SampleVoice, names: Names) {
  const lang = (c.languages ?? []).join(", ");
  return [c.model ? modelName(c.model, names.models) : c.provider, c.speed !== 1 ? `speed ${c.speed}` : "", c.accent ? c.accent.charAt(0).toUpperCase() + c.accent.slice(1) : "", lang].filter(Boolean).join(" · ");
}

/** One take: Play with its text, and Flag. */
function Take({ c, t, player, g }: { c: SampleVoice; t: SampleTake; player: Player; g: UseGallery }) {
  const playing = player.now?.card === c.id && player.now.line === t.line;
  return (
    <li className={[styles.take, playing ? styles.playing : "", t.flag ? styles.flagged : ""].filter(Boolean).join(" ")} aria-current={playing ? "true" : undefined}>
      <Button size="sm" variant="ghost" icon={playing ? "pause" : "play"} className={styles.takePlay} aria-label={`Play ${t.text}`} title={t.text} onClick={() => (playing ? player.stop() : player.play({ card: c.id, line: t.line, file: t.file }))}>
        <span className={styles.takeText}>{t.text}</span>
      </Button>
      <FlagControl text={t.text} flag={t.flag} disabled={g.busy} onSave={(f) => void g.flag(c, t.line, f)} />
    </li>
  );
}

function VoiceCard({ c, player, g, names, onDelete }: { c: SampleVoice; player: Player; g: UseGallery; names: Names; onDelete: (c: SampleVoice) => void }) {
  const [ask, setAsk] = useState<RetakeReport | null>(null);
  const [done, setDone] = useState<RetakeReport | null>(null);
  const wrong = c.takes.filter((t) => t.heard_ok === false);
  const flagged = c.takes.filter((t) => t.flag);
  const queued = player.queue === c.id;
  const studio = c.provider === "elevenlabs" && !!c.voice;
  const title = voiceName(c.voice_name, c.voice, names.voices);
  const name = `${title}, ${c.model ? modelName(c.model, names.models) : c.provider}`;
  return (
    <article className={styles.card} aria-label={name} data-component="voice-card">
      <header className={styles.head}>
        <h5 className={styles.name}>{title}</h5>
        {c.kind === "pack" && <Badge>Pack</Badge>}
      </header>
      <p className={styles.meta}>{metaOf(c, names)}</p>
      {c.description && <p className={styles.desc}>{c.description}</p>}
      <ul className={styles.takes} aria-label={`Takes of ${name}`}>
        {c.takes.map((t) => (
          <Take key={t.line} c={c} t={t} player={player} g={g} />
        ))}
      </ul>
      {wrong.length > 0 && (
        <ul className={styles.notes} aria-label={`Heard wrong in ${name}`}>
          {wrong.map((t) => (
            <li key={t.line}>
              {t.text}: the speech check heard {t.heard ? `“${t.heard}”` : "nothing"}
            </li>
          ))}
        </ul>
      )}
      {flagged.length > 0 && (
        <ul className={styles.notes} aria-label={`Flagged in ${name}`}>
          {flagged.map((t) => (
            <li key={t.line}>
              {t.text}: {flagText(t.flag!)}
            </li>
          ))}
        </ul>
      )}
      {ask && (
        <Banner
          kind="warning"
          icon="danger-triangle"
          action={
            <>
              <Button
                size="sm"
                variant="danger"
                disabled={g.busy}
                onClick={async () => {
                  const r = await g.retake(c, ask.digest);
                  setAsk(null);
                  setDone(r);
                }}
              >
                Re-take and pay
              </Button>
              <Button size="sm" onClick={() => setAsk(null)}>
                Cancel
              </Button>
            </>
          }
        >
          This sends {ask.lines.length === 1 ? "1 line" : `${ask.lines.length} lines`} to ElevenLabs, which bills it. {ask.estimate ? costText(ask.estimate) : ""}
        </Banner>
      )}
      {done && !ask && (
        <p className={styles.status} role="status">
          {(done.cleared ?? []).length === done.lines.length ? `Made ${done.lines.length === 1 ? "1 take" : `${done.lines.length} takes`} again.` : `Made ${(done.cleared ?? []).length} of ${done.lines.length} takes again; the rest stay flagged.`}
        </p>
      )}
      <div className={styles.actions}>
        <Button size="sm" icon={queued ? "pause" : "play"} aria-label={queued ? `Stop ${name}` : `Play all of ${name}`} onClick={() => (queued ? player.stop() : player.playAll(tracksOf(c), c.id))}>
          {queued ? "Stop" : "Play all"}
        </Button>
        {studio && flagged.length > 0 && (
          <Button
            size="sm"
            disabled={g.busy}
            aria-label={`Re-take the flagged takes of ${name}`}
            onClick={async () => {
              setDone(null);
              const r = await g.retake(c, null);
              if (r?.needs_confirm) setAsk(r);
              else setDone(r);
            }}
          >
            Re-take flagged
          </Button>
        )}
        <span className={styles.grow} />
        {c.kind === "sample" && <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" aria-label={`Delete the samples of ${name}`} onClick={() => onDelete(c)} />}
      </div>
    </article>
  );
}

/** The Samples part of the ElevenLabs section: every voice's samples, and the packs on this
 *  Mac. `voices` and `models` (the account's) give the cards their readable names. */
export function Gallery({ g, player, voices, models }: { g: UseGallery; player: Player } & Names) {
  const [deleting, setDeleting] = useState<SampleVoice | "all" | null>(null);
  const cards = g.gallery?.voices ?? [];
  const samples = cards.filter((c) => c.kind === "sample");
  const all = player.queue === "all";
  const names = { voices, models };
  return (
    <section className={styles.part} aria-label="Samples">
      <header className={styles.partHead}>
        <h4 className={styles.partTitle}>
          Samples
        </h4>
        {cards.length > 0 && (
          <div className={styles.partActions}>
            <Button size="sm" icon={all ? "pause" : "play"} onClick={() => (all ? player.stop() : player.playAll(cards.flatMap(tracksOf), "all"))}>
              {all ? "Stop" : "Play all voices"}
            </Button>
            {samples.length > 0 && (
              <Button size="sm" variant="danger-ghost" onClick={() => setDeleting("all")}>
                Delete all…
              </Button>
            )}
          </div>
        )}
      </header>
      {g.error && (
        <Banner kind="error" icon="danger-triangle">
          {g.error}
        </Banner>
      )}
      {(g.gallery?.notes ?? []).map((n) => (
        <p key={n} className={styles.status}>
          {n}
        </p>
      ))}
      {g.gallery && cards.length === 0 && <p className={styles.muted}>No samples. Sample makes them.</p>}
      {cards.length > 0 && (
        <div className={styles.grid} data-component="sample-gallery">
          {cards.map((c) => (
            <VoiceCard key={`${c.kind}|${c.id}`} c={c} player={player} g={g} names={names} onDelete={setDeleting} />
          ))}
        </div>
      )}
      {deleting && (
        <Dialog
          open
          title={deleting === "all" ? "Delete every sample?" : `Delete the samples of ${voiceName(deleting.voice_name, deleting.voice, voices)}?`}
          onClose={async (v) => {
            const d = deleting;
            setDeleting(null);
            if (v !== "delete") return;
            player.stop();
            await g.remove(d === "all" ? "all" : [d.id]);
          }}
          actions={
            <>
              <Button type="submit" value="cancel" variant="ghost">
                Cancel
              </Button>
              <Button type="submit" value="delete" variant="danger">
                Delete
              </Button>
            </>
          }
        >
          <p className={styles.status}>
            {deleting === "all" ? `Removes the samples of ${samples.length === 1 ? "1 voice" : `${samples.length} voices`} from this Mac.` : `Removes ${deleting.takes.length === 1 ? "1 take" : `${deleting.takes.length} takes`} of ${voiceName(deleting.voice_name, deleting.voice, voices)}, ${modelName(deleting.model, models)}.`} The takes stay in the cache, so sampling again costs nothing. Packs stay.
          </p>
        </Dialog>
      )}
    </section>
  );
}
