// The Map page: saved places and clips on an Apple map (Settings > Places > Show the Map
// page), with the places list on the left. A place's pin carries its clip count; a click
// shows its native callout, whose Show clips button shows the place's clips in the library.
// A clip with a location but no place has its own pin, whose callout's Open button opens the
// clip. Add place… searches or takes this Mac's location; a place's … menu renames it, shows
// its clips or deletes it. Map, Hybrid and Satellite save the setting `mapStyle`.
import { useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { ask, useStore } from "../../store";
import { sel } from "../../store/settings";
import type { MapStyle } from "../../bindings";
import type { GeoResult, Place } from "../../ipc/types";
import { api, errText } from "../../ipc/api";
import { Button } from "../../components/Button";
import { SearchField, Select } from "../../components/Field";
import { Popover } from "../../components/Popover";
import { SegmentedControl } from "../../components/SegmentedControl";
import { MoreMenu } from "../../components/dash/MoreMenu";
import { PageHeader } from "../../components/dash/PageHeader";
import { EmptyState } from "../../components/dash/EmptyState";
import { toast } from "../../components/toastStore";
import { fmtDay } from "../../lib/format";
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
  const style = useStore(sel.mapStyle);
  const saveSetting = useStore((s) => s.saveSetting);
  const setStyle = (v: MapStyle) => void saveSetting("mapStyle", v);
  const setFilter = useStore((s) => s.setFilter);
  const openDetail = useStore((s) => s.openDetail);
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
              <span className={styles.wideOnly}>
                <SegmentedControl label="Map style" size="sm" value={style} segments={STYLES} onChange={setStyle} />
              </span>
              <span className={styles.narrowOnly}>
                <Select aria-label="Map style" value={style} onChange={(e) => setStyle(e.target.value as MapStyle)}>
                  {STYLES.map((s) => (
                    <option key={s.value} value={s.value}>
                      {s.label}
                    </option>
                  ))}
                </Select>
              </span>
              <Button size="sm" onClick={() => setFitSeq((n) => n + 1)} disabled={!data.pins.length}>
                Show all
              </Button>
            </>
          }
        />
        {!data.pins.length && <EmptyState icon="map-point">No places or clip locations yet.</EmptyState>}
      </div>
      <div className={styles.body}>
        <PlaceList places={places} onShow={(name) => setFilter({ group: "all", place: name })} />
        <MapHost pins={data.pins} style={style} fitSeq={fitSeq} onPick={onPick} />
      </div>
    </section>
  );
}

/** The saved places: name, clip count and last flown, each with a … menu; Add place… above. */
function PlaceList({ places, onShow }: { places: Place[]; onShow: (name: string) => void }) {
  const clips = useStore(useShallow((s) => s.lib?.clips ?? []));
  const saveSetting = useStore((s) => s.saveSetting);
  const stats = (name: string) => {
    const at = clips.filter((c) => c.place === name);
    return { n: at.length, last: at.reduce<string | null>((a, c) => (!a || c.date > a ? c.date : a), null) };
  };
  const rename = async (p: Place) => {
    const v = await ask(`Rename ${p.name}`, undefined, { input: p.name, ok: "Rename" });
    if (typeof v !== "string" || !v.trim() || v.trim() === p.name) return;
    await saveSetting("places", places.map((x) => (x.name === p.name ? { ...x, name: v.trim() } : x)));
  };
  const del = async (p: Place) => {
    const ok = await ask(`Delete ${p.name}?`, "Clips keep the place name.", { ok: "Delete", danger: true });
    if (ok !== true) return;
    await saveSetting("places", places.filter((x) => x.name !== p.name));
  };
  return (
    <aside className={styles.list} aria-label="Places">
      <div className={styles.listHead}>
        <h3>Places</h3>
        <AddPlace />
      </div>
      {places.length ? (
        <ul>
          {places.map((p) => {
            const st = stats(p.name);
            return (
              <li key={p.name} className={styles.place}>
                <button type="button" className={styles.placeName} onClick={() => onShow(p.name)}>
                  <span>{p.name}</span>
                  <span className={styles.placeSub}>{[`${st.n} ${st.n === 1 ? "clip" : "clips"}`, st.last && `last flown ${fmtDay(st.last)}`].filter(Boolean).join(" · ")}</span>
                </button>
                <MoreMenu
                  label={`More actions for ${p.name}`}
                  items={[
                    { label: "Rename…", run: () => void rename(p) },
                    { label: "Show clips", run: () => onShow(p.name) },
                    null,
                    { label: "Delete…", icon: "trash-bin-trash", danger: true, run: () => void del(p) },
                  ]}
                />
              </li>
            );
          })}
        </ul>
      ) : (
        <EmptyState icon="map-point">No places yet.</EmptyState>
      )}
    </aside>
  );
}

/** Add place…: a popover with a search (on Return or the button, never per keystroke) and
 *  Use current location; a pick saves the place. */
function AddPlace() {
  const anchor = useRef<HTMLButtonElement>(null);
  const geocoder = useStore((s) => (s.values.geocoder as string | undefined) ?? "apple");
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<GeoResult[] | string | null>(null);
  const [busy, setBusy] = useState(false);
  const close = () => {
    setOpen(false);
    setHits(null);
    setQuery("");
  };
  const search = async () => {
    const q = query.trim();
    if (!q) return;
    setHits("Searching…");
    try {
      const r = await api.placeSearch(q, geocoder);
      setHits(r.length ? r : "No places found.");
    } catch (e) {
      setHits(errText(e));
    }
  };
  const save = async (h: GeoResult) => {
    try {
      await api.placeSave(h.name, +h.lat.toFixed(6), +h.lon.toFixed(6));
      toast(`Saved ${h.name}.`);
      close();
    } catch (e) {
      setHits(errText(e));
    }
  };
  const here = async () => {
    setBusy(true);
    setHits("Finding your location…");
    try {
      const h = await api.placeHere();
      if (h) await save(h);
      else setHits("No location.");
    } catch (e) {
      setHits(errText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <span className={styles.anchor}>
      <Button ref={anchor} size="sm" icon="add" aria-haspopup="dialog" aria-expanded={open} onClick={() => (open ? close() : setOpen(true))}>
        Add place…
      </Button>
      {open && (
        <Popover label="Add place" anchor={anchor} onClose={close}>
          <div className={styles.add}>
            <SearchField
              label="Search for a place"
              placeholder="Address or place name"
              value={query}
              autoFocus
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void search();
                }
              }}
            />
            <div className={styles.addButtons}>
              <Button size="sm" icon="search" onClick={() => void search()}>
                Search
              </Button>
              <Button size="sm" variant="ghost" icon="gps" disabled={busy} onClick={() => void here()}>
                Use current location
              </Button>
            </div>
            {hits != null &&
              (typeof hits === "string" ? (
                <p className={styles.placeSub}>{hits}</p>
              ) : (
                <ul className={styles.hits} aria-label="Search results">
                  {hits.map((h, i) => (
                    <li key={i}>
                      <button type="button" onClick={() => void save(h)}>
                        <b>{h.name}</b> <span className={styles.placeSub}>{h.address}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              ))}
          </div>
        </Popover>
      )}
    </span>
  );
}
