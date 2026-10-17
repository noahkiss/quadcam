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
import { fmtBytes, fmtDay, fmtDur, fmtLong } from "../../lib/format";
import { byDay, flyingOf, thumbPx } from "../../lib/library";
import { shareClips, trashClips } from "../../actions/library";
import { rebuildLibrary, revealLibrary } from "../../actions/setup";
import { ClipCard, MiniBar, stripStyle } from "./ClipCard";
import { ClipList } from "./ClipList";
import { SelectionBar } from "./SelectionBar";
import { KIND_ICON } from "./moments";
import { MomentsView, momentFrame } from "./MomentsView";
import { bestMoments } from "../../lib/moments";
import styles from "./LibraryView.module.css";
import { PathText } from "../../components/PathText";

const GAP = 16;

type Row = { kind: "head"; day: string; clips: LibClip[] } | { kind: "cards"; clips: LibClip[] } | { kind: "flat" };

/** The library: banners, the clips (grid by day, or a list), the footer. */
export function LibraryView({ onMenu }: { onMenu: (e: MouseEvent, id: string) => void }) {
  const list = useStore(useShallow(visible));
  const view = useStore(sel.libView);
  const query = useStore((s) => s.query);
  const group = useStore((s) => s.filter.group);
  return (
    <section className={styles.lib} aria-label="Library">
      <Banners list={list} />
      {group === "best_moments" ? <MomentsView clips={list} /> : !list.length ? <p className={styles.empty}>{query ? "No clips match." : "No clips here."}</p> : view === "list" ? <ClipList clips={list} onMenu={onMenu} /> : <Grid clips={list} onMenu={onMenu} />}
      <SelectionBar />
      <Footer list={list} />
    </section>
  );
}

function Banners({ list }: { list: LibClip[] }) {
  const lib = useStore((s) => s.lib);
  const group = useStore((s) => s.filter.group);
  const out: ReactNode[] = [];
  if (lib && lib.unindexed > 0) {
    const n = lib.unindexed;
    out.push(
      <Banner key="unindexed" icon="folder-open" tint="blue" action={<Button size="sm" variant="primary" onClick={rebuildLibrary}>Scan folder</Button>}>
        {n} video{n === 1 ? "" : "s"} in <PathText path={lib.root} /> {n === 1 ? "is" : "are"} not in the library.
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
  const picked = thumbPx(useStore(sel.thumbSize));
  const byDate = useStore((s) => sel.sort(s).key === "date");
  const selected = useStore((s) => s.selected);
  const cursor = useStore((s) => s.cursor);
  const width = useWidth(ref);
  // A narrow window hides the size slider: the width alone sets the columns (one or two).
  const thumb = width > 0 && width < 600 ? thumbPx(3) : picked;
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
      byDay(clips).forEach(([day, cs]) => {
        out.push({ kind: "head", day, clips: cs });
        cardRows(cs);
      });
    return out;
  }, [clips, cols, byDate]);

  const cardH = thumb + 3 + 64;
  const v = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: (i) => (rows[i].kind === "head" ? 64 : cardH + GAP),
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
                  <DayHead day={r.day} clips={r.clips} />
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
    // The content width: the scroller less its side padding, which holds the content to
    // --content-max.
    const inner = () => {
      const cs = getComputedStyle(el);
      return el.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    };
    setW(inner());
    const ro = new ResizeObserver(() => setW(inner()));
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return w;
}

/** A day's head on one line: the date, its numbers (clips, armed or flying time, flights,
 *  lowest battery), where and with what, the best moments, and Share. It wraps in a narrow
 *  window: the numbers, then the moments, each on their own row. */
function DayHead({ day, clips: cs }: { day: string; clips: LibClip[] }) {
  const openDetail = useStore((s) => s.openDetail);
  const withStats = cs.filter((c) => c.stats);
  const air = withStats.length ? withStats.reduce((a, c) => a + (c.stats!.armed_s || 0), 0) : cs.reduce((a, c) => a + flyingOf(c), 0);
  const flights = withStats.reduce((a, c) => a + (c.stats!.flights || 0), 0);
  const volts = withStats.map((c) => c.stats!.min_rx_bat_v).filter((v): v is number => v != null);
  const where = [...new Set([...cs.map((c) => c.place), ...cs.map((c) => c.aircraft)].filter(Boolean))].join(" · ");
  const best = cs.length > 1 ? bestMoments(cs, 3) : [];
  const num = (v: string, l: string) => (
    <span className={styles.num}>
      <b>{v}</b> {l}
    </span>
  );
  return (
    <div className={styles.day}>
      <div className={styles.dayHead}>
        <h2>{fmtDay(day, true)}</h2>
        <span className={styles.nums}>
          {num(String(cs.length), cs.length === 1 ? "clip" : "clips")}
          {num(withStats.length ? fmtDur(air) : fmtLong(air), withStats.length ? "armed" : "flying")}
          {flights > 0 && num(String(flights), flights === 1 ? "flight" : "flights")}
          {volts.length > 0 && num(`${Math.min(...volts).toFixed(2)} V`, "lowest battery")}
          {where && <span className={styles.where}>{where}</span>}
        </span>
        {cs.some((c) => c.last_import) && (
          <Chip kind="accent" icon="sparkle" tint="pink">
            Last import
          </Chip>
        )}
        {best.length > 0 && (
          <ul className={styles.best} aria-label="Best moments">
            {best.map(({ clip: c, moment: m }, i) => (
              <li key={i}>
                <button type="button" title={`${c.name} ${fmtDur(m.start)}`} onClick={() => openDetail(c.id, m.start)}>
                  <span className={styles.mini} style={stripStyle(c, momentFrame(c, m))} />
                  <Icon name={KIND_ICON[m.kind]} size={14} />
                  <span className={styles.name}>{c.name}</span>
                  <span className="mono">{fmtDur(m.start)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        <span className={styles.act}>
          <Button size="sm" variant="ghost" icon="share" onClick={(e) => shareClips(cs.map((c) => c.id), e.currentTarget)}>
            Share
          </Button>
        </span>
      </div>
    </div>
  );
}

function Footer({ list }: { list: LibClip[] }) {
  const root = useStore((s) => s.lib?.root || sel.outputDir(s) || "");
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
        <PathText path={root} selectable={false} />
      </button>
    </footer>
  );
}
