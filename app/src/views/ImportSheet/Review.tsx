import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../../store";
import { planOf, resultOf } from "../../store/session";
import { sel } from "../../store/settings";
import { api, errText, fileSrc } from "../../ipc/api";
import type { Clip, ClipPlan, Session } from "../../ipc/types";
import { AgentBadge, Chip } from "../../components/Chip";
import { Button } from "../../components/Button";
import { CommitInput } from "../../components/CommitInput";
import { Field, Input, Select } from "../../components/Field";
import { Icon } from "../../components/Icon";
import { toast } from "../../components/toastStore";
import { MomentList, TrimEditor } from "../../components/trim/TrimEditor";
import type { TrimModel } from "../../components/trim/model";
import { useTrim } from "../../components/trim/useTrim";
import { base, fmtBytes, fmtDur, tilde, SOURCE_LABEL } from "../../lib/format";
import { edit, forAll, metaToAll, pickLogs, runExport, savePlace, setLogDay, setLogDir } from "../../actions/session";
import { remember, setSessionCuts } from "../../actions/cuts";
import { MiniBar, MomentChips } from "../Library/ClipCard";
import { Flight } from "../ClipDetail/Inspector";
import styles from "./Review.module.css";

/** The review panel's keys, set while it shows. */
export const reviewTrim: { handleKey?: (e: KeyboardEvent, onButton: boolean) => boolean } = {};

export function Review() {
  const s = useStore((x) => x.session);
  const loadingFrom = useStore((x) => x.loadingFrom);
  const home = useStore((x) => x.home);
  const step = useStore((x) => x.step);
  if (!s || step === "load")
    return (
      <div className={styles.review}>
        <p className={styles.loading}>{loadingFrom ? `Copying from ${tilde(loadingFrom, home)}…` : "Loading…"}</p>
      </div>
    );
  return (
    <div className={styles.review}>
      <SessionBar s={s} />
      {[...s.warnings, ...s.date_warnings].length > 0 && (
        <ul className={styles.warnings}>
          {[...s.warnings, ...s.date_warnings].map((w) => (
            <li key={w}>
              <Icon name="danger-triangle" tint="yellow" size={14} />
              {w}
            </li>
          ))}
        </ul>
      )}
      <div className={styles.split}>
        <ClipRows s={s} />
        <ReviewPanel s={s} />
      </div>
    </div>
  );
}

function SessionBar({ s }: { s: Session }) {
  const profiles = useStore(sel.profiles).filter((p) => p.name);
  const places = useStore(sel.places);
  const home = useStore((x) => x.home);
  const tools = useStore((x) => !!x.env?.tools);
  const busy = useStore((x) => x.busy);
  const plans = s.plans.filter((p) => !p.skip);
  const allProf = new Set(plans.map((p) => p.meta?.profile || ""));
  const allPlace = new Set(plans.map((p) => p.meta?.location?.name || (p.meta?.location ? "*coords" : "")));
  const mixedPlace = allPlace.size > 1 || allPlace.has("*coords");
  const dates = [...new Set(plans.map((p) => p.date))];
  const fromLog = plans.filter((p) => p.source === "log").length;
  const fromClip = plans.filter((p) => p.source === "clip").length;
  const logModels = [...new Set(plans.map((p) => p.log_model).filter(Boolean))];
  const live = s.clips.filter((c) => !planOf(s, c.id)?.skip);
  const dur = live.reduce((a, c) => a + c.duration, 0);
  const dead = live.reduce((a, c) => a + (c.signal?.dead_air || []).reduce((x, d) => x + d.end - d.start, 0), 0);
  const todo = plans.length;
  return (
    <div className={styles.bar}>
      <Field label="Aircraft" hint={logModels.length ? `Radio log model ${logModels.join(", ")}` : undefined}>
        {(id, hint) => (
          <Select id={id} aria-describedby={hint} value={allProf.size > 1 ? "*" : [...allProf][0] || ""} onChange={(e) => e.target.value !== "*" && forAll(() => ({ profile: e.target.value }))}>
            <option value="">Automatic</option>
            {profiles.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
            {allProf.size > 1 && <option value="*">Mixed</option>}
          </Select>
        )}
      </Field>
      <Field label="Place">
        {(id) => (
          <Select id={id} value={mixedPlace ? "*" : [...allPlace][0] || ""} onChange={(e) => e.target.value !== "*" && forAll(() => ({ place: e.target.value }))}>
            <option value="">None</option>
            {places.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
            {mixedPlace && <option value="*">Mixed</option>}
          </Select>
        )}
      </Field>
      <Field label="Date" hint={dates.length > 1 ? `${dates.length} dates` : fromLog ? `${fromLog} of ${plans.length} from the radio log` : fromClip ? `${fromClip} of ${plans.length} from the clip clock` : "Import date"}>
        {(id, hint) => <CommitInput id={id} type="date" aria-describedby={hint} value={dates.length === 1 ? dates[0] : ""} onCommit={(v) => v && forAll(() => ({ date: v }))} />}
      </Field>
      <div className={styles.logs}>
        <span className={styles.label}>Radio logs</span>
        <span className={styles.picker}>
          <Input readOnly aria-label="Radio log folder" placeholder="None" value={tilde(s.log_dir || "", home)} />
          <Button size="sm" onClick={pickLogs}>
            Choose…
          </Button>
          {s.log_dir && <Button size="sm" variant="ghost" icon="close" aria-label="Stop using radio logs" title="Stop using radio logs" onClick={() => setLogDir(null)} />}
        </span>
        {s.log_days.length > 1 && (
          <Select aria-label="Log day" value={s.log_day || ""} onChange={(e) => setLogDay(e.target.value)}>
            {s.log_days.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </Select>
        )}
      </div>
      <div className={styles.sum}>
        <span className="mono">
          {s.clips.length} clips · {fmtDur(dur - dead)} flying · {fmtDur(dead)} dead air
        </span>
        <Button variant="primary" iconEnd="arrow-right" title={`Add ${todo} clip${todo === 1 ? "" : "s"} to the library (⌘↩)`} disabled={!tools || busy} onClick={runExport}>
          Add to Library
        </Button>
      </div>
    </div>
  );
}

function ClipRows({ s }: { s: Session }) {
  const selected = useStore((x) => x.selectedClip);
  const progress = useStore((x) => x.progress);
  return (
    <div role="grid" aria-label="Clips" className={styles.rows} tabIndex={-1}>
      {s.clips.map((c) => (
        <ClipRow key={c.id} clip={c} plan={planOf(s, c.id)!} session={s} selected={selected === c.id} progress={progress[c.id] || 0} />
      ))}
    </div>
  );
}

const ClipRow = memo(function ClipRow({ clip: c, plan: p, session: s, selected, progress }: { clip: Clip; plan: ClipPlan; session: Session; selected: boolean; progress: number }) {
  const selectClip = useStore((x) => x.selectClip);
  const defaultName = useStore(sel.defaultName);
  const r = resultOf(s, c.id);
  const outcome = r && r.outcome !== "skipped" ? r.outcome : null;
  const unusable = c.status === "empty" || !!c.stage_error;
  const sugg = p.suggested && Object.values(p.suggested).some(Boolean);
  const hhmm = p.time ? p.time.slice(0, 5) : "";
  const moments = (p.moments || []).filter((m) => m.end > 0 && m.start < c.duration);
  return (
    <div
      role="row"
      aria-selected={selected}
      data-clip={c.id}
      className={[styles.row, p.skip && styles.skipped].filter(Boolean).join(" ")}
      onClick={(e) => {
        if ((e.target as Element).closest("input,button,label")) return;
        selectClip(c.id);
      }}
    >
      <div role="gridcell" className={styles.thumbCell}>
        {c.thumb ? <img className={styles.thumb} src={fileSrc(c.thumb)} alt="" /> : <div className={styles.thumb} />}
      </div>
      <div role="gridcell" className={styles.main}>
        <div className={styles.nameLine}>
          <CommitInput
            aria-label={`Short name for ${c.name}`}
            placeholder={defaultName}
            className={p.suggested.name ? styles.suggested : undefined}
            title={p.suggested.name ? `Agent suggestion${p.reason ? ": " + p.reason : ""}` : undefined}
            value={p.name}
            onCommit={(v) => edit({ id: c.id, name: v })}
            onFocus={() => selectClip(c.id)}
          />
          {sugg && <AgentBadge title={p.reason || "Suggested by an agent"} />}
        </div>
        <div className={styles.meta}>
          <CommitInput type="date" aria-label={`Date for ${c.name}`} className={p.suggested.date ? styles.suggested : undefined} value={p.date} onCommit={(v) => v && edit({ id: c.id, date: v, time: hhmm })} />
          <CommitInput type="time" aria-label={`Time for ${c.name}`} title="Time of day (empty: noon)" className={p.suggested.date ? styles.suggested : undefined} value={hhmm} onCommit={(v) => edit({ id: c.id, time: v })} />
          <SourceChip plan={p} hasLogs={!!s.log_dir} />
          <span className="mono">{c.name}</span>
          {(c.status !== "ok" || c.stage_error) && (
            <Chip tint="yellow" icon="danger-triangle">
              {c.stage_error ? "missing" : c.detail}
            </Chip>
          )}
        </div>
        <CommitInput
          aria-label={`Note for ${c.name}`}
          placeholder="Note"
          list="recent-notes"
          className={p.suggested.note ? styles.suggested : undefined}
          value={p.note}
          onCommit={(v) => {
            remember("notes", v);
            edit({ id: c.id, note: v });
          }}
        />
        {sugg && p.reason && <div className={styles.agentHint}>{p.reason}</div>}
        {progress > 0 && !outcome && <progress max={1} value={progress} aria-label={`Converting ${c.name}`} />}
        {outcome === "verified" && (
          <div className={styles.ok}>
            <Icon name="check-circle" size={14} />
            {base(r!.output)}
          </div>
        )}
        {outcome === "failed" && <div className={styles.err}>{r!.error}</div>}
      </div>
      <div role="gridcell" className={styles.side}>
        <div className={styles.dur}>
          <span className="mono">{fmtDur(c.duration)}</span>
          <MiniBar duration={c.duration || 1} dead={c.signal?.dead_air || []} />
        </div>
        <MomentChips moments={moments} />
        {p.cuts?.length > 0 && (
          <span className={styles.cuts}>
            <Icon name="scissors" tint="sky" size={13} /> {p.cuts.length} cut{p.cuts.length === 1 ? "" : "s"}
          </span>
        )}
      </div>
      <div role="gridcell" className={styles.skip}>
        <label>
          <input type="checkbox" checked={p.skip} disabled={unusable} onChange={(e) => edit({ id: c.id, skip: e.target.checked })} />
          Skip
        </label>
      </div>
    </div>
  );
});

function SourceChip({ plan: p, hasLogs }: { plan: ClipPlan; hasLogs: boolean }) {
  const why = p.match_reason ?? undefined;
  const likely = p.badge === "likely" ? " · likely" : "";
  // A log matched, but the clip keeps its own date (a wrong radio clock or a far clip clock).
  const logged = p.badge !== "unmatched" && p.source !== "log" ? ` · radio log${likely}` : "";
  if (p.source === "log")
    return (
      <Chip icon="radio" tint={p.badge === "likely" ? "yellow" : "green"} title={why}>
        {p.time ? `radio log ${p.time.slice(0, 5)}` : "radio log"}
        {p.segments > 1 ? ` · ${p.segments} packs` : ""}
        {likely}
      </Chip>
    );
  if (p.source === "clip")
    return (
      <Chip icon="clock" tint="sky" title={why}>
        {p.time ? `clip clock ${p.time.slice(0, 5)}` : "clip clock"}
        {logged}
      </Chip>
    );
  if (p.source === "edited")
    return (
      <Chip icon="pen" tint="mauve">
        edited
      </Chip>
    );
  if (logged)
    return (
      <Chip icon="radio" tint="yellow" title={why}>
        import date{logged}
      </Chip>
    );
  return <Chip icon="calendar">{hasLogs ? "no log match" : "import date"}</Chip>;
}

function sessionTrimModel(s: Session, c: Clip, p: ClipPlan): TrimModel {
  const r = resultOf(s, c.id);
  const cuts = (p.cuts || []).map((k) => {
    const done = (r?.cuts || []).find((x) => Math.abs(x.start - k.start) < 0.001 && Math.abs(x.end - k.end) < 0.001);
    return { start: k.start, end: k.end, state: done ? (done.outcome === "verified" ? ("saved" as const) : ("failed" as const)) : ("new" as const), file: done?.output, error: done?.error, agent: p.suggested?.cuts };
  });
  const moments = (p.moments || []).filter((m) => m.end > 0 && m.start < c.duration);
  return {
    key: `s:${s.source}:${c.id}`,
    duration: c.duration,
    moments,
    deadAir: c.signal?.dead_air || [],
    keep: c.signal?.keep || [],
    cuts,
    hasLog: p.source === "log" || moments.length > 0,
    logOffset: p.log_offset_s,
    logInterval: p.log_interval_s,
  };
}

function ReviewPanel({ s }: { s: Session }) {
  const id = useStore((x) => x.selectedClip);
  const c = s.clips.find((x) => x.id === id);
  if (!c) return <aside className={styles.panel} />;
  return <Panel key={c.id} s={s} clip={c} plan={planOf(s, c.id)!} />;
}

function Panel({ s, clip: c, plan: p }: { s: Session; clip: Clip; plan: ClipPlan }) {
  const tab = useStore((x) => x.rTab);
  const setTab = useStore((x) => x.setRTab);
  const video = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState<"idle" | "loading" | "on">("idle");
  const model = useMemo(() => (c.probe ? sessionTrimModel(s, c, p) : null), [s, c, p]);
  const trim = useTrim(model, video, {
    setCuts: (cuts) => setSessionCuts(c.id, cuts),
    setOffset: (v) => edit({ id: c.id, log_offset_s: v }),
  });
  const play = async () => {
    if (playing === "loading") return;
    setPlaying("loading");
    try {
      const path = await api.preview(c.id);
      video.current!.src = fileSrc(path);
      setPlaying("on");
      video.current!.play().catch(() => {});
    } catch (e) {
      setPlaying("idle");
      toast(errText(e), true);
    }
  };
  const playRef = useRef(play);
  useEffect(() => {
    playRef.current = play;
    reviewTrim.handleKey = (e, onButton) => {
      if (trim.handleKey(e)) return true;
      if (e.key === " " && !onButton && !e.metaKey) {
        playRef.current();
        return true;
      }
      return false;
    };
  });
  useEffect(
    () => () => {
      reviewTrim.handleKey = undefined;
    },
    [],
  );
  const pr = c.probe;
  const tabs = [
    ["details", "Details", "tag"],
    ["flight", "Flight", "radio"],
    ["file", "File", "film"],
  ] as const;
  return (
    <aside className={styles.panel} aria-label="Selected clip">
      <div className={styles.player} style={playing === "on" || !c.thumb ? undefined : { backgroundImage: `url("${fileSrc(c.thumb)}")` }}>
        <video ref={video} playsInline controls hidden={playing !== "on"} />
        {playing !== "on" && c.probe && (
          <button type="button" className={styles.play} disabled={playing === "loading"} onClick={play}>
            <Icon name="play" />
            <span>{playing === "loading" ? "Making preview…" : "Play"}</span>
          </button>
        )}
      </div>
      {model && (
        <>
          <MomentList trim={trim} />
          <TrimEditor trim={trim} />
        </>
      )}
      <div role="tablist" aria-label="Clip" className={styles.tabs}>
        {tabs.map(([t, label, icon]) => (
          <button key={t} type="button" role="tab" id={`rtab-${t}`} aria-selected={tab === t} aria-controls={`rpanel-${t}`} tabIndex={tab === t ? 0 : -1} className={styles.tab} onClick={() => setTab(t)}>
            <Icon name={icon} size={14} />
            {label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`rpanel-${tab}`} aria-labelledby={`rtab-${tab}`} className={styles.tabPanel}>
        {tab === "details" && <Meta clip={c} plan={p} />}
        {tab === "flight" && <Flight stats={p.flight} moments={(p.moments || []).filter((m) => m.end > 0 && m.start < c.duration)} />}
        {tab === "file" && (
          <dl className={styles.file}>
            {(
              [
                ["Clip", c.rel],
                ["Duration", fmtDur(c.duration)],
                ["Frames", pr?.video_packets ?? "–"],
                ["Size", fmtBytes(c.size)],
                ["Video", pr?.width ? `${pr.width}×${pr.height} @ ${pr.fps?.toFixed(2) ?? "?"} fps` : "–"],
                ["Audio", pr?.audio_streams ? "yes" : "none"],
                ["Status", c.stage_error || c.detail],
              ] as const
            ).map(([k, v]) => (
              <div key={k}>
                <dt>{k}</dt>
                <dd className="selectable">{String(v)}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
    </aside>
  );
}

const locationText = (l: { lat: number; lon: number; name?: string | null } | null) => (l ? l.name || `${l.lat.toFixed(5)}, ${l.lon.toFixed(5)}` : "");

/** "Field" (a saved place), "40.6892, -74.0445", or empty (no location). */
function placePatch(text: string, places: { name: string }[]) {
  const t = text.trim();
  if (!t) return { place: "" };
  const known = places.find((x) => x.name.toLowerCase() === t.toLowerCase());
  if (known) return { place: known.name };
  const m = t.match(/^\s*(-?\d+(?:\.\d+)?)\s*[, ]\s*(-?\d+(?:\.\d+)?)\s*$/);
  if (m) return { location: { lat: +m[1], lon: +m[2] } };
  return null;
}

function Meta({ clip: c, plan: p }: { clip: Clip; plan: ClipPlan }) {
  const profiles = useStore(sel.profiles);
  const places = useStore(sel.places);
  const defaultProfile = useStore(sel.defaultProfile);
  const [placeName, setPlaceName] = useState("");
  const m = p.meta || { profile: null, location: null, keywords: [], author: null };
  const find = (n: string | null | undefined) => profiles.find((x) => x.name.toLowerCase() === (n || "").trim().toLowerCase());
  // The same choice the core makes at export: the clip's own, the log's model, the first
  // profile whose video system names the clip's source, the default.
  let prof = m.profile ? find(m.profile) : undefined;
  let why = m.profile ? "chosen" : "";
  if (!prof) {
    const lm = (p.log_model || "").toLowerCase();
    const byLog = lm ? profiles.find((x) => (x.edgetx_models || []).some((n) => n.trim().toLowerCase() === lm)) : undefined;
    const sys = SOURCE_LABEL[c.kind].toLowerCase();
    const byKind = profiles.find((x) => (x.video_system || "").trim().toLowerCase() === sys);
    if (byLog) [prof, why] = [byLog, `radio log model ${p.log_model}`];
    else if (byKind) [prof, why] = [byKind, `video system ${byKind.video_system}`];
    else if (find(defaultProfile)) [prof, why] = [find(defaultProfile), "default"];
  }
  const loc = m.location || (prof?.place ? places.find((x) => x.name === prof!.place) : null);
  const words = ["FPV", ...(prof?.keywords || []), ...(m.keywords || [])];
  const effective = [
    prof ? prof.name : "No aircraft",
    [prof?.camera_make, prof?.camera_model].filter(Boolean).join(" "),
    loc ? locationText(loc) : "no location",
    `keywords ${[...new Set(words.map((w) => w.trim()).filter(Boolean))].join(", ")} + moment kinds`,
  ].filter(Boolean);
  return (
    <div className={styles.metaFields}>
      <Field label="Aircraft">
        {(id) => (
          <Select id={id} value={m.profile ? find(m.profile)?.name || "" : ""} onChange={(e) => edit({ id: c.id, profile: e.target.value })}>
            <option value="">{prof && why !== "chosen" ? `Automatic · ${prof.name}` : "Automatic"}</option>
            {profiles
              .filter((x) => x.name)
              .map((x) => (
                <option key={x.name} value={x.name}>
                  {x.name}
                </option>
              ))}
          </Select>
        )}
      </Field>
      <Field label="Place">
        {(id) => (
          <CommitInput
            id={id}
            list="places-list"
            placeholder="Saved place, or lat, lon"
            value={locationText(m.location)}
            onCommit={(v) => {
              const patch = placePatch(v, places);
              if (!patch) return toast("Type a saved place, or latitude and longitude like 40.6892, -74.0445.", true);
              edit({ id: c.id, ...patch });
            }}
          />
        )}
      </Field>
      {m.location && !m.location.name && (
        <div className={styles.saveRow}>
          <Input aria-label="Name this place" placeholder="Name this place" value={placeName} onChange={(e) => setPlaceName(e.target.value)} />
          <Button size="sm" variant="ghost" onClick={() => savePlace(c.id, placeName).then(() => setPlaceName(""))}>
            Save as place
          </Button>
        </div>
      )}
      <Field label="Keywords">
        {(id) => (
          <CommitInput
            id={id}
            list="recent-keywords"
            placeholder="Comma-separated"
            value={(m.keywords || []).join(", ")}
            onCommit={(v) => {
              remember("keywords", v);
              edit({ id: c.id, keywords: v.split(",").map((x) => x.trim()).filter(Boolean) });
            }}
          />
        )}
      </Field>
      <Field label="Author">
        {(id) => (
          <CommitInput
            id={id}
            list="recent-authors"
            placeholder={prof?.author || ""}
            value={m.author || ""}
            onCommit={(v) => {
              remember("authors", v);
              edit({ id: c.id, author: v });
            }}
          />
        )}
      </Field>
      <p className={styles.small}>Written into the file: {effective.join(" · ")}.</p>
      <div className={styles.saveRow}>
        <Button size="sm" variant="secondary" onClick={() => metaToAll(c.id)}>
          Apply to all clips
        </Button>
        <span className={styles.small}>Aircraft, place, keywords and author.</span>
      </div>
    </div>
  );
}
