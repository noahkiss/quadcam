import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, type MouseEvent } from "react";
import { useStore } from "../../store";
import { sel } from "../../store/settings";
import type { LibClip } from "../../ipc/types";
import { FlagMark } from "../../components/FlagButton";
import { Icon } from "../../components/Icon";
import { Stars } from "../../components/Stars";
import { fmtDur } from "../../lib/format";
import { rate, setSort } from "../../actions/library";
import type { SortKey } from "../../lib/library";
import { MomentChips, stripStyle } from "./ClipCard";
import { NameEdit, useNameClick } from "./NameEdit";
import styles from "./ClipList.module.css";

const ROW = 44;
const SORTS: Record<string, SortKey> = { Name: "name", Date: "date", Length: "duration", Rating: "rating" };
const HEADS = ["", "Name", "Date", "Length", "Rating", "Flag", "Moments", "Place", "Cuts", "Photos"];

/** The library as a table, one row per clip; header buttons sort. */
export function ClipList({ clips, onMenu }: { clips: LibClip[]; onMenu: (e: MouseEvent, id: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const sort = useStore(sel.sort);
  const selected = useStore((s) => s.selected);
  const cursor = useStore((s) => s.cursor);
  const v = useVirtualizer({ count: clips.length, getScrollElement: () => ref.current, estimateSize: () => ROW, overscan: 8 });
  useEffect(() => {
    const i = clips.findIndex((c) => c.id === cursor);
    if (i >= 0) v.scrollToIndex(i, { align: "auto" });
  }, [cursor, clips, v]);
  const items = v.getVirtualItems();
  const top = items[0]?.start ?? 0;
  const bottom = v.getTotalSize() - (items.at(-1)?.end ?? 0);
  return (
    <div ref={ref} className={styles.scroll}>
      <table className={styles.table} role="grid" aria-label="Clips" aria-multiselectable="true">
        <thead>
          <tr>
            {HEADS.map((h, i) => {
              const k = SORTS[h];
              if (!k) return <th key={i}>{h ? h : <span className="visually-hidden">Thumbnail</span>}</th>;
              const on = k === sort.key;
              return (
                <th key={i} aria-sort={on ? (sort.dir === "asc" ? "ascending" : "descending") : undefined}>
                  <button type="button" className={styles.sort} onClick={() => setSort(k)}>
                    {h}
                    {on && <Icon name={sort.dir === "asc" ? "arrow-up" : "arrow-down"} size={12} />}
                  </button>
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {top > 0 && <tr aria-hidden="true" style={{ height: top }} />}
          {items.map((it) => (
            <Row key={clips[it.index].id} clip={clips[it.index]} selected={selected.has(clips[it.index].id)} onMenu={onMenu} />
          ))}
          {bottom > 0 && <tr aria-hidden="true" style={{ height: bottom }} />}
        </tbody>
      </table>
    </div>
  );
}

function Row({ clip: c, selected, onMenu }: { clip: LibClip; selected: boolean; onMenu: (e: MouseEvent, id: string) => void }) {
  const select = useStore((s) => s.select);
  const openDetail = useStore((s) => s.openDetail);
  const renaming = useStore((s) => s.renaming === c.id);
  const nameClick = useNameClick(c.id);
  return (
    <tr
      tabIndex={0}
      aria-selected={selected}
      data-id={c.id}
      className={c.flag === "reject" ? styles.rejected : undefined}
      onClick={(e) => select(c.id, { meta: e.metaKey, shift: e.shiftKey })}
      onDoubleClick={() => {
        nameClick.cancel();
        openDetail(c.id);
      }}
      onContextMenu={(e) => onMenu(e, c.id)}
    >
      <td>
        <div className={styles.lthumb} style={stripStyle(c)} />
      </td>
      <td className={styles.name} onClick={nameClick.onClick}>
        {renaming ? <NameEdit clip={c} /> : <b>{c.name}</b>}
      </td>
      <td className="mono">
        {c.date} {c.time || ""}
      </td>
      <td className="mono">{fmtDur(c.duration)}</td>
      <td>
        <Stars rating={c.rating || 0} onRate={(r) => rate([c.id], r)} size={12} />
      </td>
      <td>
        <FlagMark flag={c.flag} />
      </td>
      <td>
        <MomentChips moments={c.moments} />
      </td>
      <td>{c.place || ""}</td>
      <td>{c.cuts.length ? c.cuts.length : ""}</td>
      <td>{c.in_photos ? <Icon name="photos" tint="green" size={14} /> : null}</td>
    </tr>
  );
}
