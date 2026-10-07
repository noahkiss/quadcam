import { useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { store, useStore } from "../../store";
import { libClip, visible } from "../../store/library";
import { api, errText, fileSrc } from "../../ipc/api";
import type { LibClip } from "../../ipc/types";
import { Button } from "../../components/Button";
import { Chip } from "../../components/Chip";
import { Icon } from "../../components/Icon";
import { toast } from "../../components/toastStore";
import { MomentList, TrimEditor } from "../../components/trim/TrimEditor";
import type { TrimModel } from "../../components/trim/model";
import { useTrim } from "../../components/trim/useTrim";
import { fmtDay } from "../../lib/format";
import { deadOf } from "../../lib/library";
import { revealClip, shareClips, stepClip } from "../../actions/library";
import { saveLibraryCuts, setLibraryCuts, splitByFlight } from "../../actions/cuts";
import { screenKeys } from "../../keys";
import { stripStyle } from "../Library/ClipCard";
import { NameEdit } from "../Library/NameEdit";
import { Inspector } from "./Inspector";
import styles from "./ClipDetail.module.css";

function libTrimModel(c: LibClip): TrimModel {
  const cuts = [...c.cuts.map((k) => ({ start: k.start, end: k.end, state: "saved" as const, file: k.path })), ...(c.pending_cuts || []).map((k) => ({ start: k.start, end: k.end, state: "new" as const }))].sort((a, b) => a.start - b.start);
  return { key: `lib:${c.id}`, duration: c.duration, moments: c.moments, deadAir: deadOf(c), keep: c.keep, cuts, hasLog: false, logOffset: 0, flights: c.stats?.flight_spans.length ?? 0 };
}

/** The open clip: player, moments, the trim editor and the inspector. */
export function ClipDetail() {
  const c = useStore((s) => libClip(s, s.detailId));
  if (!c) return null;
  return <Detail key={c.id} clip={c} />;
}

function Detail({ clip: c }: { clip: LibClip }) {
  const close = useStore((s) => s.closeDetail);
  const renaming = useStore((s) => s.renaming === c.id);
  const setRenaming = useStore((s) => s.setRenaming);
  const setMenu = useStore((s) => s.setMenu);
  const ids = useStore(useShallow((s) => visible(s).map((x) => x.id)));
  const at = ids.indexOf(c.id);
  const video = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState<"idle" | "loading" | "on">("idle");
  const model = useMemo(() => libTrimModel(c), [c]);
  const trim = useTrim(model, video, {
    setCuts: (cuts) => setLibraryCuts(c.id, cuts),
    save: () => saveLibraryCuts(c.id),
    split: () => splitByFlight(() => api.librarySplit(c.id), c.cuts.length + c.pending_cuts.length),
  });

  const play = async () => {
    if (playing === "loading") return;
    setPlaying("loading");
    try {
      const path = await api.libraryPreview(c.id);
      const v = video.current!;
      v.src = fileSrc(path);
      const seekTo = store.getState().detailSeek;
      if (seekTo != null) {
        v.addEventListener("loadedmetadata", () => (v.currentTime = Math.max(0, seekTo - 1)), { once: true });
        store.setState({ detailSeek: null });
      }
      setPlaying("on");
      v.play().catch(() => {});
    } catch (e) {
      setPlaying("idle");
      toast(errText(e), true);
    }
  };
  const playRef = useRef(play);
  useEffect(() => {
    playRef.current = play;
  });

  // A moment picked in the day summary starts playing there.
  useEffect(() => {
    if (store.getState().detailSeek != null) playRef.current();
  }, []);

  useEffect(() => {
    screenKeys.detail = (e) => {
      if (trim.handleKey(e)) return true;
      const onButton = (e.target as Element | null)?.closest?.("button");
      if (e.key === " " && !onButton) {
        playRef.current();
        return true;
      }
      if (e.key === "Enter" && !onButton) {
        setRenaming(c.id);
        return true;
      }
      return false;
    };
    return () => {
      screenKeys.detail = undefined;
    };
  });

  return (
    <section className={styles.detail} aria-label="Clip">
      <div className={styles.bar} data-share-anchor>
        <Button variant="ghost" size="sm" icon="arrow-left" onClick={close}>
          Library
        </Button>
        <span className={styles.sep} aria-hidden="true" />
        <h2 className={styles.name} title="Rename" onClick={() => setRenaming(c.id)}>
          {renaming ? <NameEdit clip={c} /> : c.name}
        </h2>
        <span className={styles.sub}>
          {fmtDay(c.date)}
          {c.time ? ` · ${c.time}` : ""}
          {at >= 0 ? ` · ${at + 1} of ${ids.length}` : ""}
        </span>
        {c.aircraft && (
          <Chip icon="quad" tint="pink">
            {c.aircraft}
          </Chip>
        )}
        {c.place && (
          <Chip icon="map-point" tint="green">
            {c.place}
          </Chip>
        )}
        <span className={styles.right}>
          {c.in_photos && (
            <Chip icon="photos" tint="green" title="In Photos">
              In Photos
            </Chip>
          )}
          <Button size="sm" variant="ghost" icon="share" data-share onClick={(e) => shareClips([c.id], e.currentTarget)}>
            Share
          </Button>
          <Button size="sm" variant="ghost" icon="finder" aria-label="Show in Finder" title="Show in Finder (⌘R)" onClick={() => revealClip(c.id)} />
          <Button
            size="sm"
            variant="ghost"
            icon="dots"
            aria-label="More actions"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setMenu({ id: c.id, x: r.left, y: r.bottom + 4 });
            }}
          />
          <Button size="sm" icon="arrow-left" aria-label="Previous clip" title="Previous clip" disabled={at <= 0} onClick={() => stepClip(-1)} />
          <Button size="sm" icon="arrow-right" aria-label="Next clip" title="Next clip" disabled={at < 0 || at >= ids.length - 1} onClick={() => stepClip(1)} />
        </span>
      </div>
      <div className={styles.body}>
        <div className={styles.left}>
          <div className={styles.top}>
            <div className={styles.player} style={playing === "on" ? undefined : stripStyle(c)}>
              <video ref={video} playsInline controls hidden={playing !== "on"} />
              {playing !== "on" && (
                <button type="button" className={styles.play} disabled={playing === "loading"} onClick={play}>
                  <Icon name="play" />
                  <span>{playing === "loading" ? "Loading…" : "Play"}</span>
                </button>
              )}
            </div>
            <MomentList trim={trim} />
          </div>
          <TrimEditor trim={trim} strip={c.strip ? fileSrc(c.strip) : undefined} />
        </div>
        <Inspector clip={c} at={trim.head} />
      </div>
    </section>
  );
}
