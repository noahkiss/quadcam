import { useState } from "react";
import { useStore } from "../../store";
import { sel } from "../../store/settings";
import type { FlightStats, LibClip, Moment } from "../../ipc/types";
import { CommitInput } from "../../components/CommitInput";
import { Field, Select } from "../../components/Field";
import { FlagButtons } from "../../components/FlagButton";
import { Stars } from "../../components/Stars";
import { fmtBytes, fmtDur } from "../../lib/format";
import { editClip, rate, renameClip } from "../../actions/library";
import { remember } from "../../actions/cuts";
import { KIND } from "../Library/moments";
import styles from "./Inspector.module.css";

/** The open clip's details and flight numbers. */
export function Inspector({ clip: c }: { clip: LibClip }) {
  const tab = useStore((s) => s.dTab);
  const setTab = useStore((s) => s.setDTab);
  return (
    <aside className={styles.inspector} aria-label="Inspector">
      <div role="tablist" aria-label="Inspector" className={styles.tabs}>
        {(["details", "flight"] as const).map((t) => (
          <button
            key={t}
            type="button"
            role="tab"
            id={`dtab-${t}`}
            aria-selected={tab === t}
            aria-controls={`dpanel-${t}`}
            tabIndex={tab === t ? 0 : -1}
            className={styles.tab}
            onClick={() => setTab(t)}
            onKeyDown={(e) => {
              if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
                const next = t === "details" ? "flight" : "details";
                setTab(next);
                document.getElementById(`dtab-${next}`)?.focus();
                e.preventDefault();
              }
            }}
          >
            {t === "details" ? "Details" : "Flight"}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`dpanel-${tab}`} aria-labelledby={`dtab-${tab}`} className={styles.panel}>
        {tab === "details" ? <Details clip={c} /> : <Flight stats={c.stats} moments={c.moments} />}
      </div>
    </aside>
  );
}

function Details({ clip: c }: { clip: LibClip }) {
  const places = useStore(sel.places);
  const profiles = useStore(sel.profiles).filter((p) => p.name);
  const [kw, setKw] = useState("");
  const isProfile = profiles.some((p) => p.name.toLowerCase() === (c.aircraft || "").toLowerCase());
  const placeKnown = places.some((p) => p.name.toLowerCase() === (c.place || "").toLowerCase());
  const words = c.keywords || [];
  return (
    <div className={styles.fields}>
      <Field label="Name">{(id) => <CommitInput id={id} value={c.title || c.name} onCommit={(v) => renameClip(c.id, v)} />}</Field>
      <div className={styles.pair}>
        <Field label="Date">{(id) => <CommitInput id={id} type="date" value={c.date} onCommit={(v) => v && editClip(c.id, { date: v })} />}</Field>
        <Field label="Time">{(id) => <CommitInput id={id} type="time" aria-label="Time of day" value={c.time || ""} onCommit={(v) => editClip(c.id, { time: v })} />}</Field>
      </div>
      <div className={styles.pair}>
        <Field label="Aircraft">
          {(id) => (
            <Select id={id} value={isProfile ? profiles.find((p) => p.name.toLowerCase() === (c.aircraft || "").toLowerCase())!.name : c.aircraft ? "*" : ""} onChange={(e) => editClip(c.id, { profile: e.target.value })}>
              <option value="">None</option>
              {profiles.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
              {c.aircraft && !isProfile && (
                <option value="*" disabled>
                  {c.aircraft}
                </option>
              )}
            </Select>
          )}
        </Field>
        <Field label="Place" hint={c.location ? <span className="mono selectable">{`${c.location.lat.toFixed(4)}, ${c.location.lon.toFixed(4)}`}</span> : undefined}>
          {(id, hintId) => (
            <Select id={id} aria-describedby={hintId} value={placeKnown ? places.find((p) => p.name.toLowerCase() === (c.place || "").toLowerCase())!.name : c.place || ""} onChange={(e) => editClip(c.id, { place: e.target.value })}>
              <option value="">None</option>
              {places.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
              {c.place && !placeKnown && <option value={c.place}>{c.place}</option>}
            </Select>
          )}
        </Field>
      </div>
      <div className={styles.kwField}>
        <span className={styles.label} id="kw-label">
          Keywords
        </span>
        <div className={styles.kw} role="group" aria-labelledby="kw-label">
          {words.map((w, i) => (
            <span key={w + i} className={styles.chip}>
              {w}
              <button type="button" aria-label={`Remove ${w}`} onClick={() => editClip(c.id, { keywords: words.filter((_, j) => j !== i) })}>
                ×
              </button>
            </span>
          ))}
          <input
            type="text"
            aria-label="Add keyword"
            placeholder="Add…"
            list="recent-keywords"
            spellCheck={false}
            value={kw}
            onChange={(e) => setKw(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && kw.trim()) {
                e.preventDefault();
                remember("keywords", kw);
                editClip(c.id, { keywords: [...words, kw.trim()] });
                setKw("");
              }
            }}
          />
        </div>
      </div>
      <Field label="Author">
        {(id) => (
          <CommitInput
            id={id}
            list="recent-authors"
            value={c.author || ""}
            onCommit={(v) => {
              remember("authors", v);
              editClip(c.id, { author: v });
            }}
          />
        )}
      </Field>
      <Field label="Note">
        {(id) => (
          <CommitInput
            id={id}
            list="recent-notes"
            value={c.note || ""}
            onCommit={(v) => {
              remember("notes", v);
              editClip(c.id, { note: v });
            }}
          />
        )}
      </Field>
      <div className={styles.rateRow}>
        <Stars rating={c.rating || 0} onRate={(r) => rate([c.id], r)} size={18} />
        <span className={styles.flags}>
          <FlagButtons flag={c.flag} labels onFlag={(f) => rate([c.id], null, f)} />
        </span>
      </div>
      <dl className={styles.meta}>
        <dt>File</dt>
        <dd className="selectable">{c.path}</dd>
        <dt>Size</dt>
        <dd>{fmtBytes(c.size)}</dd>
        {c.dvr && (
          <>
            <dt>{c.parts.length ? "DVR files" : "DVR file"}</dt>
            <dd className="selectable">{[c.dvr, ...c.parts.map((x) => x.dvr)].join(", ")}</dd>
          </>
        )}
        {c.original && (
          <>
            <dt>Original</dt>
            <dd className="selectable">{c.original}</dd>
          </>
        )}
      </dl>
    </div>
  );
}

/** The radio log's numbers for a clip. */
export function Flight({ stats: f, moments }: { stats: FlightStats | null; moments: Moment[] }) {
  if (!f) return <p className={styles.muted}>No radio log for this clip.</p>;
  const kinds = new Map<Moment["kind"], number>();
  for (const m of moments || []) kinds.set(m.kind, (kinds.get(m.kind) || 0) + 1);
  const box = (label: string, value: string, sub = "") => (
    <div className={styles.stat}>
      <span className={styles.label}>{label}</span>
      <b>{value}</b>
      <span className={styles.muted}>{sub}</span>
    </div>
  );
  return (
    <div className={styles.stats}>
      {box("Armed time", fmtDur(Math.round(f.armed_s || 0)), `${f.packs} pack${f.packs === 1 ? "" : "s"}`)}
      {box("Lowest battery", f.min_rx_bat_v != null ? `${f.min_rx_bat_v.toFixed(2)} V` : "–", "receiver voltage")}
      {box("Link quality", f.min_lq != null ? `${Math.round(f.min_lq)} %` : "–", "lowest")}
      {box("Signal", f.min_rssi_db != null ? `${Math.round(f.min_rssi_db)} dBm` : "–", "weakest RSSI")}
      {box("Max throttle", f.max_throttle != null ? `${Math.round(f.max_throttle * 100)} %` : "–")}
      {box(
        "Moments",
        String((moments || []).length),
        [...kinds].map(([k, n]) => (n > 1 ? `${KIND[k].toLowerCase()} ×${n}` : KIND[k].toLowerCase())).join(" · "),
      )}
    </div>
  );
}

