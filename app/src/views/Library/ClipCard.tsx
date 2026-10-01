import { memo, useRef, useState, type MouseEvent } from "react";
import { useStore } from "../../store";
import { fileSrc } from "../../ipc/api";
import type { LibClip, Moment, Span } from "../../ipc/types";
import { Chip } from "../../components/Chip";
import { FlagMark } from "../../components/FlagButton";
import { Icon } from "../../components/Icon";
import { Stars } from "../../components/Stars";
import { fmtDur } from "../../lib/format";
import { deadOf } from "../../lib/library";
import { rate } from "../../actions/library";
import { KIND, KIND_ICON } from "./moments";
import { NameEdit, useNameClick } from "./NameEdit";
import styles from "./ClipCard.module.css";

/** The strip shows frame `f` of 10; a clip without one shows its poster. */
export function stripStyle(c: Pick<LibClip, "strip" | "poster">, frame = 2) {
  if (c.strip) return { backgroundImage: `url("${fileSrc(c.strip)}")`, backgroundSize: "1000% 100%", backgroundPosition: `${(frame / 9) * 100}% 0` };
  if (c.poster) return { backgroundImage: `url("${fileSrc(c.poster)}")`, backgroundSize: "cover", backgroundPosition: "center" };
  return {};
}

/** The flying bar: the clip, with its dead air cut out. */
export function MiniBar({ duration, dead }: { duration: number; dead: Span[] }) {
  return (
    <span className={styles.minibar} aria-hidden="true">
      {dead.map((d, i) => (
        <span key={i} className={styles.dead} style={{ left: `${(d.start / duration) * 100}%`, width: `${((d.end - d.start) / duration) * 100}%` }} />
      ))}
    </span>
  );
}

export function MomentChips({ moments }: { moments: Moment[] }) {
  const counts = new Map<Moment["kind"], number>();
  for (const m of moments || []) if (m.kind !== "dead_air") counts.set(m.kind, (counts.get(m.kind) || 0) + 1);
  if (!counts.size) return <span className={styles.none}>No moments</span>;
  return (
    <span className={styles.mchips}>
      {[...counts].map(([k, n]) => (
        <span key={k} className={`${styles.mchip} k-${k}`} title={KIND[k]} role="img" aria-label={n > 1 ? `${KIND[k]} ×${n}` : KIND[k]}>
          <Icon name={KIND_ICON[k]} size={13} />
          {n > 1 ? n : null}
        </span>
      ))}
    </span>
  );
}

interface Props {
  clip: LibClip;
  selected: boolean;
  onMenu: (e: MouseEvent, id: string) => void;
}

/** One clip in the grid: the hover-scrub thumbnail, name, time, rating, moments. */
export const ClipCard = memo(function ClipCard({ clip: c, selected, onMenu }: Props) {
  const select = useStore((s) => s.select);
  const openDetail = useStore((s) => s.openDetail);
  const renaming = useStore((s) => s.renaming === c.id);
  const nameClick = useNameClick(c.id);
  return (
    <div
      role="gridcell"
      className={[styles.card, selected && styles.selected, c.flag === "reject" && styles.rejected].filter(Boolean).join(" ")}
      tabIndex={0}
      aria-selected={selected}
      aria-label={c.name}
      data-id={c.id}
      onClick={(e) => select(c.id, { meta: e.metaKey, shift: e.shiftKey })}
      onDoubleClick={() => {
        nameClick.cancel();
        openDetail(c.id);
      }}
      onContextMenu={(e) => onMenu(e, c.id)}
    >
      <article aria-label={c.name} className={styles.inner}>
      <Thumb clip={c} selected={selected} />
      <MiniBar duration={c.duration || 1} dead={deadOf(c)} />
      <div className={styles.body}>
        <div className={styles.line}>
          <span className={styles.name} title={c.name} onClick={(e) => nameClick.onClick(e)}>
            {renaming ? <NameEdit clip={c} /> : c.name}
          </span>
          <FlagMark flag={c.flag} />
        </div>
        <div className={styles.line}>
          <span className={styles.time}>{c.time || ""}</span>
          <MomentChips moments={c.moments} />
          <span className={styles.stars}>
            <Stars rating={c.rating || 0} onRate={(r) => rate([c.id], r)} size={12} />
          </span>
        </div>
      </div>
      </article>
    </div>
  );
});

function Thumb({ clip: c, selected }: { clip: LibClip; selected: boolean }) {
  const [scrub, setScrub] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const style = stripStyle(c, scrub == null ? 2 : Math.floor(scrub * 10));
  return (
    <div
      ref={ref}
      className={[styles.thumb, c.no_picture && styles.noPicture].filter(Boolean).join(" ")}
      style={style}
      onMouseMove={(e) => {
        if (!c.strip) return;
        const r = ref.current!.getBoundingClientRect();
        setScrub(Math.min(0.999, Math.max(0, (e.clientX - r.left) / r.width)));
      }}
      onMouseLeave={() => setScrub(null)}
    >
      <span className={styles.tl}>
        {c.flag === "reject" ? (
          <Chip kind="overlay" icon="close" tint="red">
            Rejected
          </Chip>
        ) : c.cuts.length ? (
          <Chip kind="overlay" icon="scissors" tint="sky">
            {c.cuts.length} cut{c.cuts.length === 1 ? "" : "s"}
          </Chip>
        ) : null}
      </span>
      <span className={styles.tr}>
        {c.in_photos && (
          <span className={styles.round} title="In Photos" role="img" aria-label="In Photos">
            <Icon name="photos" size={13} />
          </span>
        )}
        {selected && (
          <span className={[styles.round, styles.check].join(" ")} aria-hidden="true">
            <Icon name="check" size={12} />
          </span>
        )}
      </span>
      {scrub != null && (
        <>
          <span className={styles.scrub} style={{ left: `${scrub * 100}%` }} />
          <span className={styles.scrubT} style={{ left: `${scrub * 100}%` }}>
            {fmtDur(scrub * c.duration)}
          </span>
        </>
      )}
      <span className={styles.dur}>{fmtDur(c.duration)}</span>
    </div>
  );
}
