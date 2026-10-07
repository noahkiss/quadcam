import { useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { Button } from "../Button";
import { Kbd, AgentBadge } from "../Chip";
import { Icon } from "../Icon";
import { fmtT, parseT } from "../../lib/format";
import { KIND, KIND_ICON } from "../../views/Library/moments";
import { dragPoint, frameMoment, rulerStep } from "./model";
import type { Trim } from "./useTrim";
import styles from "./TrimEditor.module.css";

/** A time field that commits on change (Tab, Return, leaving). */
function TimeInput({ label, value, onCommit }: { label: string; value: number | null; onCommit: (v: number | null) => void }) {
  const [text, setText] = useState(value != null ? fmtT(value) : "");
  const [focus, setFocus] = useState(false);
  const shown = focus ? text : value != null ? fmtT(value) : "";
  return (
    <input
      type="text"
      className={styles.time}
      aria-label={label}
      placeholder="0:00.0"
      spellCheck={false}
      value={shown}
      onFocus={() => {
        setText(value != null ? fmtT(value) : "");
        setFocus(true);
      }}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        setFocus(false);
        if (text !== (value != null ? fmtT(value) : "")) onCommit(parseT(text));
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
    />
  );
}

/** The trim editor: in and out points, Add cut, the lanes (video, signal, moments, cuts),
 * the cut list, the log offset and the keys. */
export function TrimEditor({ trim, strip }: { trim: Trim; strip?: string }) {
  const { model, sel } = trim;
  const lanesRef = useRef<HTMLDivElement>(null);
  if (!model || !(model.duration > 0)) return null;
  const dur = model.duration;
  const pct = (t: number) => `${(Math.min(Math.max(t, 0), dur) / dur) * 100}%`;
  const width = (a: number, b: number) => `${((Math.min(b, dur) - Math.max(a, 0)) / dur) * 100}%`;
  const step = rulerStep(dur);
  const ticks: number[] = [];
  for (let t = 0; t <= dur + 0.001; t += step) ticks.push(t);
  const unsaved = model.cuts.filter((k) => k.state === "new").length;
  const deadSecs = model.deadAir.reduce((a, d) => a + (Math.min(d.end, dur) - Math.max(d.start, 0)), 0);
  const keepSecs = model.keep.reduce((a, k) => a + (k.end - k.start), 0);
  const hasSel = sel.in != null && sel.out != null && sel.out > sel.in;

  const timeAt = (clientX: number) => {
    const r = lanesRef.current!.getBoundingClientRect();
    return ((clientX - r.left) / r.width) * dur;
  };
  const drag = (e: RPointerEvent, which: "in" | "out") => {
    e.preventDefault();
    e.stopPropagation();
    const move = (ev: PointerEvent) => {
      const s = dragPoint(trim.sel, which, timeAt(ev.clientX), dur);
      trim.setSel(s);
      trim.seek(s[which]!);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <section className={styles.trim} aria-label="Trim">
      <div className={styles.bar}>
        <h3>Trim</h3>
        <label className={styles.inline}>
          In
          <TimeInput label="In point" value={sel.in} onCommit={(v) => trim.setSel({ ...sel, in: v })} />
        </label>
        <Button size="sm" variant="ghost" onClick={() => trim.setPoint("in")}>
          Set in <Kbd>I</Kbd>
        </Button>
        <label className={styles.inline}>
          Out
          <TimeInput label="Out point" value={sel.out} onCommit={(v) => trim.setSel({ ...sel, out: v })} />
        </label>
        <Button size="sm" variant="ghost" onClick={() => trim.setPoint("out")}>
          Set out <Kbd>O</Kbd>
        </Button>
        <Button size="sm" icon="scissors" onClick={() => trim.addCut()}>
          Add cut
        </Button>
        {model.keep.length > 0 && (
          <Button size="sm" variant="ghost" icon="scissors" onClick={() => trim.useKeep()}>
            {`Use keep ranges (${model.keep.length})`}
          </Button>
        )}
        {trim.opts.split && (model.flights ?? 0) > 0 && (
          <Button size="sm" variant="ghost" icon="scissors" title="Adds one cut per radio-log flight" onClick={() => trim.opts.split?.()}>
            {`Split by flight (${model.flights})`}
          </Button>
        )}
        {trim.opts.save && unsaved > 0 && (
          <Button size="sm" variant="primary" onClick={() => trim.opts.save?.()}>
            {unsaved === 1 ? "Save 1 cut" : `Save ${unsaved} cuts`}
          </Button>
        )}
      </div>

      <div className={styles.timeline}>
        <div className={styles.labels} aria-hidden="true">
          <span />
          <span>Video</span>
          <span>Signal</span>
          <span>Moments</span>
          <span>Cuts</span>
        </div>
        <div
          ref={lanesRef}
          className={styles.lanes}
          role="group"
          aria-label="Clip timeline"
          onClick={(e) => {
            if ((e.target as Element).closest("button")) return;
            trim.seek(timeAt(e.clientX));
          }}
        >
          <div className={styles.ruler} aria-hidden="true">
            {ticks.map((t) => (
              <span key={t} style={{ left: pct(t) }}>
                {fmtT(t, false)}
              </span>
            ))}
          </div>
          <div className={styles.video} style={strip ? { backgroundImage: `url("${strip}")` } : undefined}>
            {model.deadAir.map((d, i) => (
              <div key={i} className={styles.deadZone} style={{ left: pct(d.start), width: width(d.start, d.end) }} title={`Dead air ${fmtT(d.start)}–${fmtT(d.end)}`} />
            ))}
            {hasSel && (
              <>
                <div className={styles.outside} style={{ left: 0, width: pct(sel.in!) }} />
                <div className={styles.outside} style={{ left: pct(sel.out!), right: 0 }} />
                {(["in", "out"] as const).map((which) => (
                  <button
                    key={which}
                    type="button"
                    className={styles.handle}
                    aria-label={which === "in" ? "Drag the in point" : "Drag the out point"}
                    style={{ left: pct(sel[which]!) }}
                    onPointerDown={(e) => drag(e, which)}
                    onKeyDown={(e) => {
                      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                      e.preventDefault();
                      e.stopPropagation();
                      trim.setSel({ ...sel, [which]: Math.max(0, Math.min(dur, sel[which]! + (e.key === "ArrowLeft" ? -0.1 : 0.1))) });
                    }}
                  />
                ))}
              </>
            )}
          </div>
          <div className={styles.signal} aria-hidden="true">
            {model.deadAir.map((d, i) => (
              <span key={i} className={styles.signalDead} style={{ left: pct(d.start), width: width(d.start, d.end) }} />
            ))}
          </div>
          <div className={styles.marks}>
            {model.moments.map((m, i) => (
              <button
                key={i}
                type="button"
                className={[styles.mark, m.score < 0.5 && styles.low].filter(Boolean).join(" ")}
                style={{ left: pct(m.start) }}
                title={`${KIND[m.kind]} at ${fmtT(m.start)} · score ${Math.round(m.score * 100)} %`}
                aria-label={`${KIND[m.kind]} at ${fmtT(m.start)}`}
                onClick={() => trim.frame(m)}
              >
                <Icon name={KIND_ICON[m.kind]} size={12} />
                <span>{KIND[m.kind]}</span>
              </button>
            ))}
          </div>
          <div className={styles.cutLane} aria-hidden="true">
            {model.keep.map((k, i) => (
              <span key={`k${i}`} className={styles.keep} style={{ left: pct(k.start), width: width(k.start, k.end) }} />
            ))}
            {model.cuts.map((k, i) => (
              <span key={i} className={[styles.cut, k.state === "new" && styles.cutNew].filter(Boolean).join(" ")} style={{ left: pct(k.start), width: width(k.start, k.end) }}>
                Cut {i + 1}
              </span>
            ))}
          </div>
          {trim.head != null && <span className={styles.head} style={{ left: pct(trim.head) }} data-t={fmtT(trim.head, false)} />}
        </div>
      </div>

      <div className={styles.legend} aria-hidden="true">
        <span>
          <i className={styles.swFly} />
          Flying
        </span>
        {deadSecs > 0 && (
          <span>
            <i className={styles.swDead} />
            Dead air {fmtT(deadSecs, false)}
          </span>
        )}
        {keepSecs > 0 && (
          <span>
            <i className={styles.swKeep} />
            Keep {fmtT(keepSecs, false)}
          </span>
        )}
        <span>
          <i className={styles.swCut} />
          Cuts
        </span>
      </div>

      <ol className={styles.cuts} aria-label="Cuts">
        {model.cuts.map((k, i) => (
          <li key={i} className={styles.cutRow}>
            <span className="mono">{i + 1}</span>
            <button
              type="button"
              className={styles.link}
              title="Go to this cut"
              onClick={() => {
                trim.setSel({ in: k.start, out: k.end });
                trim.seek(k.start);
              }}
            >
              {fmtT(k.start)} – {fmtT(k.end)}
            </button>
            <span className={styles.muted}>{(k.end - k.start).toFixed(1)} s</span>
            <span className={styles.file}>{k.file ? k.file.split("/").pop() : ""}</span>
            {k.state === "saved" ? (
              <span className={styles.ok}>
                <Icon name="check-circle" size={14} />
                Saved
              </span>
            ) : k.state === "failed" ? (
              <span className={styles.err} title={k.error || ""}>
                <Icon name="close-circle" size={14} />
                Failed
              </span>
            ) : (
              <span className={styles.muted}>Not saved yet</span>
            )}
            {k.agent && <AgentBadge />}
            <Button size="sm" variant="ghost" icon="close" aria-label={`Remove cut ${i + 1}`} title="Remove" onClick={() => trim.removeCut(i)} />
          </li>
        ))}
        {!model.cuts.length && <li className={styles.muted}>No cuts.</li>}
      </ol>

      {model.hasLog && <OffsetRow trim={trim} />}
      {model.logInterval != null && model.logInterval > 0.3 && (
        <p className={styles.muted}>
          Radio log rows are {model.logInterval.toFixed(1)} s apart. Moment times are within about {model.logInterval.toFixed(1)} s.
        </p>
      )}
      <p className={styles.keys}>
        <span>
          <Kbd>Space</Kbd> play
        </span>
        <span>
          <Kbd>J</Kbd>
          <Kbd>K</Kbd>
          <Kbd>L</Kbd> back, stop, forward
        </span>
        <span>
          <Kbd>←</Kbd>
          <Kbd>→</Kbd> one frame
        </span>
        <span>
          <Kbd>I</Kbd>
          <Kbd>O</Kbd> set in, out
        </span>
        <span>
          <Kbd>C</Kbd> add cut
        </span>
      </p>
    </section>
  );
}

function OffsetRow({ trim }: { trim: Trim }) {
  const off = trim.model?.logOffset ?? 0;
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    const n = parseFloat(draft ?? "");
    if (draft != null && Number.isFinite(n) && n !== off) trim.opts.setOffset?.(n);
    setDraft(null);
  };
  return (
    <div className={styles.offset}>
      <label className={styles.inline}>
        Log starts at
        <input
          type="number"
          step="0.1"
          className={styles.time}
          aria-label="Log offset in seconds"
          value={draft ?? String(off)}
          onFocus={() => setDraft(String(off))}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
          onBlur={commit}
        />
        s
      </label>
      <Button size="sm" variant="ghost" title="Sets the log start to the playhead" onClick={() => trim.armHere()}>
        Arm is here
      </Button>
    </div>
  );
}

/** The clip's moments as a list: select one to frame it, or cut it at once. */
export function MomentList({ trim }: { trim: Trim }) {
  const ms = trim.model?.moments || [];
  const deadSecs = (trim.model?.deadAir || []).reduce((a, d) => a + d.end - d.start, 0);
  const keepSecs = (trim.model?.keep || []).reduce((a, k) => a + k.end - k.start, 0);
  return (
    <div className={styles.moments}>
      <h3>Moments</h3>
      <ul>
        {ms.length ? (
          ms.map((m, i) => {
            const on = trim.sel.in != null && Math.abs((trim.sel.in ?? 0) - Math.max(0, m.start - (m.kind === "dead_air" ? 0 : 1))) < 0.05;
            return (
              <li key={i} className={on ? styles.momentOn : undefined}>
                <Icon name={KIND_ICON[m.kind]} size={14} className={`k-${m.kind}`} />
                <button type="button" className={styles.momentName} onClick={() => trim.frame(m)}>
                  {KIND[m.kind]}
                </button>
                <span className={styles.score} title={`Score ${m.score.toFixed(2)}`} aria-hidden="true">
                  <span style={{ width: `${Math.round(m.score * 100)}%` }} />
                </span>
                <span className="mono">{fmtT(m.start, false)}</span>
                <Button
                  size="sm"
                  variant="ghost"
                  icon="scissors"
                  aria-label={`Cut the ${KIND[m.kind].toLowerCase()} at ${fmtT(m.start, false)}`}
                  title="Cut"
                  onClick={() => {
                    trim.frame(m);
                    trim.addCut(frameMoment(m, trim.model!.duration));
                  }}
                />
              </li>
            );
          })
        ) : (
          <li className={styles.muted}>No moments.</li>
        )}
      </ul>
      {(deadSecs > 0 || keepSecs > 0) && (
        <p className={styles.muted}>
          Dead air {fmtT(deadSecs, false)} · Keep {fmtT(keepSecs, false)}
        </p>
      )}
    </div>
  );
}
