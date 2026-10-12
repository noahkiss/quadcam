// The Map page: saved places and clips on an Apple map (Settings > Places > Show the Map
// page). A place's pin carries its clip count; clicking it shows the place's clips in the
// library. A clip with a location but no place has its own pin, which opens the clip.
import { useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useStore } from "../../store";
import { sel } from "../../store/settings";
import type { MapStyle } from "../../bindings";
import { Button } from "../../components/Button";
import { SegmentedControl } from "../../components/SegmentedControl";
import { PageHeader } from "../../components/dash/PageHeader";
import { EmptyState } from "../../components/dash/EmptyState";
import { mapData, mapSummary, pickOf } from "../../lib/map";
import { MapHost } from "./MapHost";
import styles from "./MapView.module.css";

const STYLES: { value: MapStyle; label: string }[] = [
  { value: "standard", label: "Map" },
  { value: "hybrid", label: "Hybrid" },
  { value: "satellite", label: "Satellite" },
];

export function MapView() {
  const clips = useStore(useShallow((s) => s.lib?.clips ?? []));
  const places = useStore(sel.places);
  const style = useStore((s) => s.mapStyle);
  const setStyle = useStore((s) => s.setMapStyle);
  const setFilter = useStore((s) => s.setFilter);
  const openDetail = useStore((s) => s.openDetail);
  const openSettings = useStore((s) => s.openSettings);
  const [fitSeq, setFitSeq] = useState(0);
  const data = useMemo(() => mapData(clips, places), [clips, places]);
  const onPick = (id: string) => {
    const p = pickOf(id);
    if (!p) return;
    if ("place" in p) setFilter({ group: "all", place: p.place });
    else openDetail(p.clip);
  };
  return (
    <section className={styles.page} aria-labelledby="map-title">
      <div className={styles.head}>
        <PageHeader
          id="map-title"
          title="Map"
          subtitle={mapSummary(data)}
          actions={
            <>
              <SegmentedControl label="Map style" size="sm" value={style} segments={STYLES} onChange={setStyle} />
              <Button size="sm" onClick={() => setFitSeq((n) => n + 1)} disabled={!data.pins.length}>
                Show all
              </Button>
            </>
          }
        />
        {!data.pins.length && (
          <EmptyState icon="map-point" action={<Button size="sm" onClick={() => openSettings("places")}>Add place…</Button>}>
            No places or clip locations yet.
          </EmptyState>
        )}
      </div>
      <MapHost pins={data.pins} style={style} fitSeq={fitSeq} onPick={onPick} />
    </section>
  );
}
