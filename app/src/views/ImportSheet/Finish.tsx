import { useState } from "react";
import { store, useStore } from "../../store";
import { clipsOf, planOf, resultOf } from "../../store/session";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Checkbox, Input } from "../../components/Field";
import { Icon } from "../../components/Icon";
import { base, fmtBytes, fmtDay, fmtT, tilde, SOURCE_LABEL } from "../../lib/format";
import { addToPhotos, askFormat, checkFormat, eject, formatBlocker, runExport } from "../../actions/session";
import styles from "./Finish.module.css";

/** Finish: what was added, Photos, safe to remove, and the format step. */
export function Finish() {
  const s = useStore((x) => x.session);
  const home = useStore((x) => x.home);
  const defaultName = useStore(sel.defaultName);
  const deletion = useStore((x) => x.clipDeletion);
  if (!s) return null;
  const deleted = (deletion || []).filter((x) => x.state === "deleted").length;
  const onCard = (id: number) => {
    // A joined recording's files go or stay together; one kept file names the reason.
    const ids = [id, ...(s.clips.find((c) => c.id === id)?.join?.parts || [])];
    const ds = (deletion || []).filter((x) => ids.includes(x.id));
    const d = ds.find((x) => x.state === "kept") || ds[0];
    if (!d) return null;
    const [from, at] = s.card ? ["from the card", "on the card"] : ["from the folder", "in the folder"];
    return <span className={styles.muted}>{d.state === "deleted" ? `Deleted ${from}` : `Kept ${at}: ${d.reason || ""}`}</span>;
  };
  const ok = s.results.filter((r) => r.outcome === "verified");
  const cutsOk = ok.reduce((a, r) => a + (r.cuts || []).filter((k) => k.outcome === "verified").length, 0);
  const failed = s.results.filter((r) => r.outcome === "failed").length + s.results.reduce((a, r) => a + (r.cuts || []).filter((k) => k.outcome === "failed").length, 0);
  const dirs = [...new Set(ok.map((r) => (r.output || "").split("/").slice(0, -1).join("/")))];
  const days = [...new Set(s.plans.filter((p) => !p.skip).map((p) => p.date))];
  return (
    <div className={styles.finish}>
      <div className={styles.left}>
        <div className={styles.title}>
          <h3>
            Added {ok.length} clip{ok.length === 1 ? "" : "s"}
            {cutsOk ? ` and ${cutsOk} cut${cutsOk === 1 ? "" : "s"}` : ""} to the library
          </h3>
          {failed > 0 && <span className={styles.err}>{failed} did not verify</span>}
        </div>
        {deletion && (
          <p className={styles.muted}>
            Deleted {deleted} clip{deleted === 1 ? "" : "s"} from {tilde(s.source, home)}
            {deletion.length > deleted ? ` · kept ${deletion.length - deleted}` : ""}
          </p>
        )}
        <p className={`${styles.muted} selectable`}>
          {dirs.length === 1 ? `Saved to ${tilde(dirs[0], home)}/ · each file checked against its source` : `Saved under ${tilde(s.output_dir, home)} · each file checked against its source`}
        </p>
        <ol className={styles.files}>
          {clipsOf(s).map((c) => {
            const p = planOf(s, c.id)!;
            const r = resultOf(s, c.id);
            if (!r || r.outcome === "skipped")
              return (
                <li key={c.id} className={styles.skipped}>
                  {p.name || c.name} · skipped
                </li>
              );
            const files = [{ name: r.output ? base(r.output) : c.name, ok: r.outcome === "verified", size: r.size, error: r.error }];
            for (const k of r.cuts || []) files.push({ name: k.output ? base(k.output) : `cut ${fmtT(k.start)}–${fmtT(k.end)}`, ok: k.outcome === "verified", size: k.size, error: k.error });
            return (
              <li key={c.id}>
                <b>{p.name || defaultName}</b> {onCard(c.id)}
                <ul>
                  {files.map((f) => (
                    <li key={f.name}>
                      <Icon name={f.ok ? "check-circle" : "close-circle"} tint={f.ok ? "green" : "red"} size={14} />
                      <span className="mono selectable">{f.name}</span>
                      {f.ok ? (
                        <span className={styles.muted}>{fmtBytes(f.size)}</span>
                      ) : (
                        <>
                          <span className={styles.err}>{f.error || "failed"}</span>
                          <Button size="sm" onClick={runExport}>
                            Retry
                          </Button>
                        </>
                      )}
                    </li>
                  ))}
                </ul>
              </li>
            );
          })}
        </ol>
      </div>
      <div className={styles.right}>
        <section className={`${styles.panel} ${styles.accent}`}>
          <Icon name="sparkle" tint="pink" size={24} />
          <div>
            <b>In your library</b>
            <p className={styles.muted}>
              {ok.length} clip{ok.length === 1 ? "" : "s"} under {days.map((d) => fmtDay(d)).join(", ")}, in Last import.
            </p>
          </div>
        </section>
        <ReportPanel />
        <PhotosPanel />
        {s.card && (
          <section className={styles.panel}>
            <Icon name="sd-card" tint="green" size={24} />
            <div className={styles.grow}>
              <b>{s.card.volume_name || s.card_volume?.info.volume_name || "Card"}</b>
              <p className={styles.muted}>
                {fmtBytes(s.card_volume?.info.total_size || 0)} {SOURCE_LABEL[s.kind]} card{s.clips.every((c) => c.staged && !c.stage_error) ? " · every clip is on this Mac" : ""}
              </p>
            </div>
            <Button size="sm" variant="ghost" icon="eject" onClick={eject}>
              Safe to Remove
            </Button>
          </section>
        )}
        {s.card && s.kind !== "dji" && <FormatPanel />}
      </div>
    </div>
  );
}

function PhotosPanel() {
  const s = useStore((x) => x.session)!;
  const album = useStore(sel.photosAlbum);
  const status = useStore((x) => x.photosStatus);
  const save = useStore((x) => x.saveSetting);
  const [draft, setDraft] = useState<string | null>(null);
  const verified = s.results.filter((r) => r.outcome === "verified");
  const left = verified.filter((r) => !s.in_photos.includes(r.id));
  const files = left.reduce((a, r) => a + 1 + (r.cuts || []).filter((k) => k.outcome === "verified").length, 0);
  return (
    <section className={styles.panel} aria-label="Add to Photos">
      <div className={styles.grow}>
        <h4>
          <Icon name="photos" tint="yellow" />
          Add to Photos
        </h4>
        <div className={styles.row}>
          <label className={styles.inline}>
            Album
            <Input
              placeholder="No album"
              value={draft ?? album}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => {
                if (draft != null && draft.trim() !== album) save("photosAlbum", draft.trim());
                setDraft(null);
              }}
            />
          </label>
          <Button size="sm" variant="primary" disabled={!left.length} onClick={() => addToPhotos(draft ?? album)}>
            {!verified.length ? "Add" : !left.length ? "All in Photos" : `Add ${files} file${files === 1 ? "" : "s"}`}
          </Button>
        </div>
        <p className={styles.small}>{status}</p>
      </div>
    </section>
  );
}

function FormatPanel() {
  const s = useStore((x) => x.session)!;
  const plan = useStore((x) => x.formatPlan);
  const err = useStore((x) => x.formatError);
  const unlocked = useStore((x) => x.formatUnlocked);
  const label = useStore((x) => x.formatLabelDraft);
  const saved = useStore(sel.formatLabel);
  const save = useStore((x) => x.saveSetting);
  const why = formatBlocker(s);
  const guards: [string, boolean][] = [
    ["Clips came from a card", !!s.card],
    [`All ${s.clips.length} clips copied off the card`, s.clips.every((c) => !c.stage_error)],
    ["Every clip that is not skipped verified", !why],
  ];
  if (!why && plan)
    guards.push([`Same card: ${plan.disk}, volume UUID matches`, true], [`Removable, ${fmtBytes(plan.size)}, erased as ${plan.filesystem || "FAT32"}`, true]);
  if (!why && err) guards.push([err, false]);
  const ready = !why && !!plan;
  return (
    <section className={`${styles.panel} ${styles.danger}`} aria-label="Format card">
      <div className={styles.grow}>
        <h4>
          <Icon name="danger-triangle" tint="red" />
          Format card
        </h4>
        <ul className={styles.guards}>
          {guards.map(([t, good]) => (
            <li key={t}>
              <Icon name={good ? "check-circle" : "close-circle"} tint={good ? "green" : "red"} size={14} />
              {t}
            </li>
          ))}
        </ul>
        <div className={styles.row}>
          <Checkbox label="Unlock" checked={unlocked} onChange={(e) => store.setState({ formatUnlocked: e.target.checked })} />
          <label className={styles.inline}>
            Volume name
            <Input
              maxLength={11}
              value={label || saved}
              onChange={(e) => store.setState({ formatLabelDraft: e.target.value.toUpperCase().replace(/[^A-Z0-9_-]/g, "").slice(0, 11) })}
              onBlur={() => {
                const v = store.getState().formatLabelDraft;
                if (v && v !== saved) save("formatLabel", v);
                checkFormat();
              }}
            />
          </label>
          <Button size="sm" variant="danger" icon={unlocked && ready ? "danger-triangle" : "lock"} disabled={!(unlocked && ready)} onClick={askFormat}>
            Format card…
          </Button>
        </div>
        <p className={styles.small}>{ready ? "A dialog names the disk before anything is erased. Return does not confirm." : why || ""}</p>
      </div>
    </section>
  );
}

/** Opens the session report for this import on the Flights page. */
function ReportPanel() {
  const show = () => {
    const st = store.getState();
    st.setImportOpen(false);
    st.openGear({ page: "slot", id: "flights" });
    st.setGearSegment("report");
  };
  return (
    <section className={styles.panel}>
      <Icon name="stopwatch" tint="blue" size={24} />
      <div className={styles.grow}>
        <b>Session report</b>
        <p className={styles.muted}>Flights, air time, link and packs from the radio logs.</p>
      </div>
      <Button size="sm" variant="ghost" onClick={show}>
        Show report
      </Button>
    </section>
  );
}
