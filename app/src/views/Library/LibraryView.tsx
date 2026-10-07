import { useVirtualizer } from "@tanstack/react-virtual";
import { useShallow } from "zustand/react/shallow";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { useStore } from "../../store";
import { visible } from "../../store/library";
import { sel } from "../../store/settings";
import type { LibClip } from "../../ipc/types";
import { Banner } from "../../components/Banner";
import { Button } from "../../components/Button";
import { Chip } from "../../components/Chip";
import { Icon } from "../../components/Icon";
import { fmtBytes, fmtDay, fmtDur, fmtLong, tilde } from "../../lib/format";
import { byDay, daySub, flyingOf, thumbPx } from "../../lib/library";
import { shareClips, trashClips } from "../../actions/library";
import { rebuildLibrary, revealLibrary } from "../../actions/setup";
import { ClipCard, MiniBar, stripStyle } from "./ClipCard";
import { ClipList } from "./ClipList";
import { SelectionBar } from "./SelectionBar";
import { KIND_ICON } from "./moments";
import styles from "./LibraryView.module.css";

const GAP = 16;

type Row = { kind: "head"; day: string; clips: LibClip[]; summary: boolean } | { kind: "cards"; clips: LibClip[] } | { kind: "flat" };

/** The library: banners, the clips (grid by day, or a list), the footer. */
export function LibraryView({ onMenu }: { onMenu: (e: MouseEvent, id: string) => void }) {
  const list = useStore(useShallow(visible));
  const view = useStore(sel.libView);
  const query = useStore((s) => s.query);
  return (
    <section className={styles.lib} aria-label="Library">
      <Banners list={list} />
      {!list.length ? <p className={styles.empty}>{query ? "No clips match." : "No clips here."}</p> : view === "list" ? <ClipList clips={list} onMenu={onMenu} /> : <Grid clips={list} onMenu={onMenu} />}
      <SelectionBar />
      <Footer list={list} />
    </section>
  );
}

function Banners({ list }: { list: LibClip[] }) {
  const lib = useStore((s) => s.lib);
  const group = useStore((s) => s.filter.group);
  const home = useStore((s) => s.home);
  const out: ReactNode[] = [];
  if (lib && lib.unindexed > 0) {
    const n = lib.unindexed;
    out.push(
      <Banner key="unindexed" icon="folder-open" tint="blue" action={<Button size="sm" variant="primary" onClick={rebuildLibrary}>Scan folder</Button>}>
        {n} video{n === 1 ? "" : "s"} in {tilde(lib.root, home)} {n === 1 ? "is" : "are"} not in the library.
      </Banner>,
    );
  }
  if (group === "rejected" && list.length) {
    out.push(
      <Banner key="rejected" kind="warning" icon="close" tint="red" action={<Button size="sm" variant="danger" icon="trash-bin-trash" onClick={() => trashClips(list.map((c) => c.id))}>{`Move ${list.length} to Trash`}</Button>}>
        {list.length} rejected clip{list.length === 1 ? "" : "s"}.
      </Banner>,
    );
  }
  return out.length ? <div className={styles.banners}>{out}</div> : null;
}

function Grid({ clips, onMenu }: { clips: LibClip[]; onMenu: (e: MouseEvent, id: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const thumb = thumbPx(useStore(sel.thumbSize));
  const byDate = useStore((s) => sel.sort(s).key === "date");
  const filterDay = useStore((s) => s.filter.day);
  const selected = useStore((s) => s.selected);
  const cursor = useStore((s) => s.cursor);
  const width = useWidth(ref);
  const cols = Math.max(1, Math.floor((width + GAP) / (thumb * 1.6 + GAP)));
  useEffect(() => {
    colsNow = cols;
  }, [cols]);

  const rows = useMemo(() => {
    const out: Row[] = [];
    const cardRows = (cs: LibClip[]) => {
      for (let i = 0; i < cs.length; i += cols) out.push({ kind: "cards", clips: cs.slice(i, i + cols) });
    };
    if (!byDate) cardRows(clips);
    else
      byDay(clips).forEach(([day, cs], i) => {
        out.push({ kind: "head", day, clips: cs, summary: i === 0 || filterDay === day });
        cardRows(cs);
      });
    return out;
  }, [clips, cols, byDate, filterDay]);

  const cardH = thumb + 3 + 64;
  const v = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: (i) => (rows[i].kind === "head" ? (rows[i].summary ? 150 : 56) : cardH + GAP),
    overscan: 4,
    getItemKey: (i) => (rows[i].kind === "head" ? `h${(rows[i] as { day: string }).day}` : `r${i}`),
  });

  // Keys move the cursor; keep its row on screen.
  useEffect(() => {
    if (!cursor) return;
    const i = rows.findIndex((r) => r.kind === "cards" && r.clips.some((c) => c.id === cursor));
    if (i >= 0) v.scrollToIndex(i, { align: "auto" });
  }, [cursor, rows, v]);

  return (
    <div ref={ref} role="grid" aria-label="Clips" aria-multiselectable="true" aria-rowcount={rows.length} className={styles.scroll} style={{ ["--thumb" as string]: `${thumb}px` }}>
      <div style={{ height: v.getTotalSize(), position: "relative" }}>
        {v.getVirtualItems().map((it) => {
          const r = rows[it.index];
          return (
            <div key={it.key} role="row" aria-rowindex={it.index + 1} data-index={it.index} ref={v.measureElement} className={styles.vrow} style={{ transform: `translateY(${it.start}px)` }}>
              {r.kind === "head" ? (
                <div role="gridcell" aria-colspan={cols}>
                  <DayHead day={r.day} clips={r.clips} summary={r.summary} />
                </div>
              ) : r.kind === "cards" ? (
                <div className={styles.cards} style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}>
                  {r.clips.map((c) => (
                    <ClipCard key={c.id} clip={c} selected={selected.has(c.id)} onMenu={onMenu} />
                  ))}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** Columns in the grid now, for Up and Down. */
let colsNow = 1;
export const gridColumns = () => colsNow;

function useWidth(ref: React.RefObject<HTMLElement | null>) {
  const [w, setW] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const pad = 2 * 24;
    setW(el.clientWidth - pad);
    const ro = new ResizeObserver(() => setW(el.clientWidth - pad));
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return w;
}

function DayHead({ day, clips, summary }: { day: string; clips: LibClip[]; summary: boolean }) {
  return (
    <div className={styles.day}>
      <div className={styles.dayHead}>
        <h2>{fmtDay(day, true)}</h2>
        <span className={styles.sub}>{daySub(clips, fmtLong)}</span>
        {clips.some((c) => c.last_import) && (
          <Chip kind="accent" icon="sparkle" tint="pink">
            Last import
          </Chip>
        )}
        <span className={styles.act}>
          <Button size="sm" variant="ghost" icon="share" onClick={(e) => shareClips(clips.map((c) => c.id), e.currentTarget)}>
            Share
          </Button>
        </span>
      </div>
      {summary && <DaySummary clips={clips} />}
    </div>
  );
}

/** The day's numbers beyond the head's count and flying time; nothing when it would only
 * repeat the head. */
function DaySummary({ clips: cs }: { clips: LibClip[] }) {
  const openDetail = useStore((s) => s.openDetail);
  const withStats = cs.filter((c) => c.stats);
  if (cs.length < 2 || (!withStats.length && !cs.some((c) => c.moments.length))) return null;
  const air = withStats.length ? withStats.reduce((a, c) => a + (c.stats!.armed_s || 0), 0) : cs.reduce((a, c) => a + flyingOf(c), 0);
  const flights = withStats.reduce((a, c) => a + (c.stats!.flights || 0), 0);
  const volts = withStats.map((c) => c.stats!.min_rx_bat_v).filter((v): v is number => v != null);
  const best = cs
    .flatMap((c) => c.moments.map((m) => ({ c, m })))
    .sort((a, b) => b.m.score - a.m.score)
    .slice(0, 3);
  const stat = (v: string, l: string) => (
    <div className={styles.stat}>
      <b>{v}</b>
      <span>{l}</span>
    </div>
  );
  return (
    <section className={styles.summary} aria-label="Day summary">
      {stat(String(cs.length), cs.length === 1 ? "clip" : "clips")}
      {stat(fmtDur(air), withStats.length ? "armed time" : "flying")}
      {flights > 0 && stat(String(flights), flights === 1 ? "flight" : "flights")}
      {volts.length > 0 && stat(`${Math.min(...volts).toFixed(2)} V`, "lowest battery")}
      {best.length > 0 && (
        <div className={styles.best}>
          <span className={styles.lbl}>Best moments</span>
          <ul>
            {best.map(({ c, m }, i) => (
              <li key={i}>
                <button type="button" onClick={() => openDetail(c.id, m.start)}>
                  <span className={styles.mini} style={stripStyle(c, Math.min(9, Math.floor((m.start / c.duration) * 10)))} />
                  <Icon name={KIND_ICON[m.kind]} size={14} />
                  <span>{c.name}</span>
                  <span className="mono">{fmtDur(m.start)}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function Footer({ list }: { list: LibClip[] }) {
  const root = useStore((s) => s.lib?.root || sel.outputDir(s) || "");
  const home = useStore((s) => s.home);
  const secs = list.reduce((a, c) => a + c.duration, 0);
  const fly = list.reduce((a, c) => a + flyingOf(c), 0);
  const bytes = list.reduce((a, c) => a + c.size + c.cuts.reduce((x, k) => x + k.size, 0), 0);
  const pct = secs ? Math.round((fly / secs) * 100) : 0;
  return (
    <footer className={styles.footer}>
      <span>
        {list.length} clip{list.length === 1 ? "" : "s"} · {fmtLong(fly)} flying · {fmtBytes(bytes)}
      </span>
      {secs > 0 && (
        <span className={styles.pct}>
          <span className={styles.footBar}>
            <MiniBar duration={secs} dead={[{ start: fly, end: secs }]} />
          </span>
          {pct} % flying
        </span>
      )}
      <button type="button" className={styles.path} title="Show in Finder" onClick={() => revealLibrary()}>
        <Icon name="folder-open" size={14} />
        {tilde(root, home)}
      </button>
    </footer>
  );
}
