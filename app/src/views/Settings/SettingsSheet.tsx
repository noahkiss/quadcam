import { useState, type ReactNode } from "react";
import { store, useStore } from "../../store";
import { sel } from "../../store/settings";
import type { SettingsSection } from "../../store/ui";
import { api, errText, pickFolder } from "../../ipc/api";
import type { GeoResult } from "../../ipc/types";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Checkbox, Field, Input, SearchField, SelectField, TextField } from "../../components/Field";
import { Icon, type IconName } from "../../components/Icon";
import { SegmentedControl } from "../../components/SegmentedControl";
import { toast } from "../../components/toastStore";
import { tilde } from "../../lib/format";
import { applyNameFormat, matchLibraryLogs, rebuildLibrary, revealLibrary } from "../../actions/setup";
import { draftFrom, emptyProfile, layoutExample, validPlaces, valuesOf, VIDEO_SYSTEMS, type Draft } from "./draft";
import { ModulesPane } from "./ModulesPane";
import { GearPane } from "./GearPane";
import styles from "./SettingsSheet.module.css";

const SECTIONS: [SettingsSection, string, IconName][] = [
  ["library", "Library", "folder-open"],
  ["aircraft", "Aircraft", "quad"],
  ["places", "Places", "map-point"],
  ["import", "Import", "import"],
  ["photos", "Photos", "photos"],
  ["gear", "Gear", "signal"],
  ["modules", "Modules", "hdd"],
  ["advanced", "Advanced", "sliders"],
];

/** Settings: a sheet with sections. Done saves what changed; Cancel and Escape save nothing. */
export function SettingsSheet() {
  const section = useStore((s) => s.settingsOpen);
  return section ? <Sheet key="open" first={section} /> : null;
}

function Sheet({ first }: { first: SettingsSection }) {
  const section = useStore((s) => s.settingsOpen) || first;
  const [start] = useState(() => {
    const d = draftFrom(store.getState());
    if (first === "aircraft" && !d.profiles.length) d.profiles = [emptyProfile()];
    if (first === "places" && !d.places.length) d.places = [{ name: "", lat: "", lon: "" }];
    return { draft: d, before: valuesOf(draftFrom(store.getState()), sel.tunables(store.getState())) };
  });
  const [d, setD] = useState<Draft>(start.draft);
  const [profileAt, setProfileAt] = useState(start.draft.profiles.length && first === "aircraft" ? start.draft.profiles.length - 1 : 0);
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => setD((x) => ({ ...x, [k]: v }));
  const show = (s: SettingsSection) => store.getState().openSettings(s);

  const close = async (value: string) => {
    const st = store.getState();
    st.closeSettings();
    if (value !== "save") {
      await st.loadSettings();
      return;
    }
    if (validPlaces(d.places).dropped) toast("Places without a name or a valid latitude and longitude were not saved.", true);
    const outChanged = d.outputDir !== start.draft.outputDir;
    await st.saveChanged(valuesOf(d, sel.tunables(st)), start.before);
    await st.loadSettings();
    await st.loadGear();
    if (outChanged) await api.libraryScope().catch(() => {});
    const sess = store.getState().session;
    if (sess) store.getState().setSession(await api.planDates(sel.logDir(store.getState()), sess.log_day));
    await store.getState().loadLibrary();
  };

  return (
    <Dialog
      open
      kind="sheet"
      className={styles.sheet}
      title={
        <>
          <Icon name="settings" />
          Settings
        </>
      }
      onClose={close}
      actions={
        <>
          <Button type="submit" value="cancel" variant="ghost">
            Cancel
          </Button>
          <Button type="submit" value="save" variant="primary">
            Done
          </Button>
        </>
      }
    >
      <div className={styles.body}>
        <nav aria-label="Settings sections" className={styles.nav}>
          <ul>
            {SECTIONS.map(([id, label, icon]) => (
              <li key={id}>
                <button type="button" aria-current={section === id ? "page" : undefined} onClick={() => show(id)}>
                  <Icon name={icon} />
                  {label}
                </button>
              </li>
            ))}
          </ul>
          <Version />
        </nav>
        <div className={styles.pane}>
          {section === "library" && <LibraryPane d={d} set={set} />}
          {section === "aircraft" && <AircraftPane d={d} setD={setD} at={profileAt} setAt={setProfileAt} />}
          {section === "places" && <PlacesPane d={d} set={set} />}
          {section === "import" && <ImportPane d={d} set={set} />}
          {section === "photos" && (
            <section>
              <h3>Photos</h3>
              <TextField label="Album (empty: library only)" placeholder="Drone" value={d.photosAlbum} onChange={(e) => set("photosAlbum", e.target.value)} />
            </section>
          )}
          {section === "gear" && <GearPane d={d} setD={setD} />}
          {section === "modules" && <ModulesPane d={d} set={set} />}
          {section === "advanced" && <AdvancedPane />}
        </div>
      </div>
    </Dialog>
  );
}

/** The app version and the commit it was built from. */
function Version() {
  const env = useStore((s) => s.env);
  if (!env?.version) return null;
  return (
    <p className={`${styles.version} selectable`} aria-label="Version">
      QuadCam {env.version}
      <br />
      Build {env.build}
    </p>
  );
}

type SetFn = <K extends keyof Draft>(k: K, v: Draft[K]) => void;

function LibraryPane({ d, set }: { d: Draft; set: SetFn }) {
  const home = useStore((s) => s.home);
  const place = useStore((s) => sel.places(s)[0]?.name || "Home field");
  const day = new Date().toISOString().slice(0, 10);
  const radio = (value: Draft["libraryLayout"], title: string, sub: ReactNode) => (
    <label className={styles.radioCard}>
      <input type="radio" name="layout" value={value} checked={d.libraryLayout === value} onChange={() => set("libraryLayout", value)} />
      <span>
        <b>{title}</b>
        <span className={styles.sub}>{sub}</span>
      </span>
    </label>
  );
  return (
    <section>
      <h3>Library folder</h3>
      <div className={styles.row}>
        <Input className={styles.grow} mono readOnly aria-label="Library folder" value={tilde(d.outputDir, home)} />
        <Button
          size="sm"
          onClick={async () => {
            const p = await pickFolder("Library folder", d.outputDir);
            if (p) set("outputDir", p);
          }}
        >
          Change…
        </Button>
        <Button size="sm" variant="ghost" icon="finder" onClick={revealLibrary}>
          Show in Finder
        </Button>
      </div>
      <h3>Folder layout</h3>
      <div className={styles.radioCards} role="radiogroup" aria-label="Folder layout">
        {radio("year_day", "Year / Day", <span className="mono">2026/2026-09-27/</span>)}
        {radio("day", "Day only", <span className="mono">2026-09-27/</span>)}
        {radio("flat", "Flat", "One folder")}
      </div>
      <div className={styles.layoutRow}>
        <div className={styles.checks}>
          <Checkbox label={<b>Add the place to day folders</b>} detail={<span className="mono">{`${day} ${place}`}</span>} checked={d.placeFolders} onChange={(e) => set("placeFolders", e.target.checked)} />
          <Checkbox label={<b>Keep originals</b>} detail="The DVR file goes in originals/ in the day folder." checked={d.keepOriginals} onChange={(e) => set("keepOriginals", e.target.checked)} />
        </div>
        <div className={styles.example}>
          <span className={styles.sub}>Example</span>
          <pre className="mono selectable">{layoutExample(d, place)}</pre>
        </div>
      </div>
      <div className={styles.row}>
        <SelectField label="Date in file names" value={d.nameDateFormat} onChange={(e) => set("nameDateFormat", e.target.value as Draft["nameDateFormat"])}>
          <option value="YYYY-MM-DD">2026-09-25_name.mp4</option>
          <option value="YY.MM.DD">26.09.25_name.mp4</option>
        </SelectField>
        <Button size="sm" variant="ghost" className={styles.alignEnd} onClick={() => applyNameFormat(d.nameDateFormat)}>
          Rename library files to this format
        </Button>
      </div>
      <div className={styles.info}>
        <Icon name="info" tint="sky" />
        <p>
          The files hold every detail. The index in <span className="mono">.quadcam/</span> is a cache.
        </p>
        <Button size="sm" variant="ghost" onClick={rebuildLibrary}>
          Rebuild from files
        </Button>
        <Button size="sm" variant="ghost" onClick={matchLibraryLogs}>
          Match radio logs
        </Button>
      </div>
    </section>
  );
}

function AircraftPane({ d, setD, at, setAt }: { d: Draft; setD: (f: (d: Draft) => Draft) => void; at: number; setAt: (i: number) => void }) {
  const places = useStore(sel.places);
  const aircraftCounts = useStore((s) => s.lib?.groups?.aircraft);
  const counts = new Map(aircraftCounts || []);
  const saved = useStore(sel.defaultProfile);
  const p = d.profiles[at];
  const edit = (patch: Partial<(typeof d.profiles)[number]>) => setD((x) => ({ ...x, profiles: x.profiles.map((q, i) => (i === at ? { ...q, ...patch } : q)) }));
  const list = (v: string) => v.split(",").map((s) => s.trim()).filter(Boolean);
  const add = () => {
    setD((x) => ({ ...x, profiles: [...x.profiles, emptyProfile()] }));
    setAt(d.profiles.length);
  };
  return (
    <section className={styles.aircraft}>
      <div>
        <h3>Aircraft</h3>
        <ul className={styles.pickList}>
          {d.profiles.map((q, i) => (
            <li key={i}>
              <button type="button" aria-current={i === at ? "true" : undefined} onClick={() => setAt(i)}>
                <b>
                  <Icon name="quad" tint="pink" />
                  {q.name || "New aircraft"}
                </b>
                <span className={styles.sub}>{[q.video_system, q.name && q.name === saved ? "default" : null, counts.get(q.name) ? `${counts.get(q.name)} clips` : null].filter(Boolean).join(" · ")}</span>
              </button>
            </li>
          ))}
          <li>
            <button type="button" onClick={add}>
              <b>
                <Icon name="add" tint="blue" />
                Add aircraft
              </b>
            </button>
          </li>
        </ul>
        <p className={styles.sub}>A clip uses the aircraft whose radio model name matches its log, else the default.</p>
      </div>
      {p && (
        <div className={styles.form}>
          <TextField label="Name" value={p.name} onChange={(e) => edit({ name: e.target.value })} />
          <TextField label="Aircraft" value={p.aircraft} onChange={(e) => edit({ aircraft: e.target.value })} />
          <div className={styles.wide}>
            <span className={styles.label}>Video system</span>
            <SegmentedControl label="Video system" size="sm" value={p.video_system || "Analog"} onChange={(v) => edit({ video_system: v })} segments={VIDEO_SYSTEMS.map((v) => ({ value: v, label: v }))} />
          </div>
          <TextField label="Camera make" value={p.camera_make} onChange={(e) => edit({ camera_make: e.target.value })} />
          <TextField label="Camera model" value={p.camera_model} onChange={(e) => edit({ camera_model: e.target.value })} />
          <div className={styles.wide}>
            <ListField
              label="Radio model names"
              placeholder="AIR65, …"
              hint="The model name on your radio, as it starts the log file names. Comma-separated. Only this aircraft's clips match those logs."
              value={p.edgetx_models}
              onChange={(v) => edit({ edgetx_models: list(v) })}
            />
          </div>
          <SelectField label="Default place" value={p.place || ""} onChange={(e) => edit({ place: e.target.value || null })}>
            <option value="">None</option>
            {places.map((x) => (
              <option key={x.name} value={x.name}>
                {x.name}
              </option>
            ))}
          </SelectField>
          <TextField label="Author" value={p.author} onChange={(e) => edit({ author: e.target.value })} />
          <div className={styles.wide}>
            <ListField label="Keywords" placeholder="Comma-separated" value={p.keywords} onChange={(v) => edit({ keywords: list(v) })} />
          </div>
          <div className={`${styles.wide} ${styles.row}`}>
            <Checkbox
              label={<b>Default aircraft</b>}
              detail="For clips without a matching radio log."
              checked={d.defaultIndex === at}
              onChange={(e) => setD((x) => ({ ...x, defaultIndex: e.target.checked ? at : null, defaultCleared: !e.target.checked }))}
            />
            <Button
              size="sm"
              variant="danger-ghost"
              icon="trash-bin-trash"
              className={styles.pushRight}
              onClick={() => {
                setD((x) => ({
                  ...x,
                  profiles: x.profiles.filter((_, i) => i !== at),
                  defaultIndex: x.defaultIndex == null || x.defaultIndex === at ? null : x.defaultIndex > at ? x.defaultIndex - 1 : x.defaultIndex,
                }));
                setAt(0);
              }}
            >
              Delete
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

/** A comma-separated list field that keeps the typing (commas, spaces) until it loses focus. */
function ListField({ label, placeholder, hint, value, onChange }: { label: string; placeholder: string; hint?: string; value: string[]; onChange: (v: string) => void }) {
  const [text, setText] = useState<string | null>(null);
  return (
    <Field label={label} hint={hint}>
      {(id, hintId) => <Input id={id} aria-describedby={hintId} placeholder={placeholder} value={text ?? value.join(", ")} onFocus={() => setText(value.join(", "))} onChange={(e) => setText(e.target.value)} onBlur={() => {
        if (text != null) onChange(text);
        setText(null);
      }} />}
    </Field>
  );
}

function PlacesPane({ d, set }: { d: Draft; set: SetFn }) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<GeoResult[] | string | null>(null);
  const [target, setTarget] = useState<number | null>(null);
  const googleSet = useStore((s) => s.googleKeySet);
  const search = async () => {
    const q = query.trim();
    if (!q) return;
    setHits("Searching…");
    try {
      const r = await api.placeSearch(q, d.geocoder);
      setHits(r.length ? r : "No places found.");
    } catch (e) {
      setHits(errText(e));
    }
  };
  const pick = (h: GeoResult) => {
    const rows = [...d.places];
    const lat = String(+h.lat.toFixed(6));
    const lon = String(+h.lon.toFixed(6));
    if (target != null && rows[target]) rows[target] = { name: rows[target].name || h.name, lat, lon };
    else {
      const blank = rows.findIndex((r) => !r.name && !Number.isFinite(parseFloat(r.lat)));
      if (blank >= 0) rows.splice(blank, 1);
      rows.push({ name: h.name, lat, lon });
    }
    set("places", rows);
    setHits(null);
    setQuery("");
    setTarget(null);
  };
  const editRow = (i: number, k: "name" | "lat" | "lon", v: string) => set("places", d.places.map((r, j) => (j === i ? { ...r, [k]: v } : r)));
  return (
    <section>
      <h3>Places</h3>
      <div className={styles.row}>
        <SearchField
          label="Search for a place"
          className={styles.grow}
          placeholder="Address or place name"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            // Searches only on Return or the button, never per keystroke.
            if (e.key === "Enter") {
              e.preventDefault();
              search();
            }
          }}
        />
        <Button size="sm" variant="ghost" icon="search" onClick={search}>
          Search
        </Button>
      </div>
      {hits != null && (
        <ul className={styles.results}>
          {typeof hits === "string" ? (
            <li className={styles.sub}>{hits}</li>
          ) : (
            hits.map((h, i) => (
              <li key={i}>
                <button type="button" onClick={() => pick(h)}>
                  <b>{h.name}</b> <span className={styles.sub}>{`${h.address} · ${h.lat.toFixed(5)}, ${h.lon.toFixed(5)}`}</span>
                </button>
              </li>
            ))
          )}
        </ul>
      )}
      <div className={styles.places}>
        {d.places.map((r, i) => (
          <div key={i} className={[styles.placeRow, target === i && styles.target].filter(Boolean).join(" ")}>
            <Input aria-label="Place name" placeholder="Name" value={r.name} onChange={(e) => editRow(i, "name", e.target.value)} />
            <Input aria-label="Latitude" placeholder="Latitude" type="number" step="any" value={r.lat} onChange={(e) => editRow(i, "lat", e.target.value)} />
            <Input aria-label="Longitude" placeholder="Longitude" type="number" step="any" value={r.lon} onChange={(e) => editRow(i, "lon", e.target.value)} />
            <Button
              size="sm"
              variant="ghost"
              icon="search"
              aria-label="Search for this place"
              title="Search"
              onClick={() => {
                setTarget(i);
                if (r.name.trim() && !query.trim()) setQuery(r.name.trim());
                (document.querySelector('input[aria-label="Search for a place"]') as HTMLInputElement | null)?.focus();
              }}
            />
            <Button size="sm" variant="ghost" icon="trash-bin-trash" aria-label="Delete this place" title="Delete" onClick={() => set("places", d.places.filter((_, j) => j !== i))} />
          </div>
        ))}
      </div>
      <Button size="sm" variant="ghost" icon="add" onClick={() => set("places", [...d.places, { name: "", lat: "", lon: "" }])}>
        Add place
      </Button>
      <div className={styles.row}>
        <SelectField label="Search with" value={d.geocoder} onChange={(e) => set("geocoder", e.target.value)}>
          <option value="apple">Apple Maps</option>
          <option value="nominatim">OpenStreetMap (Nominatim)</option>
          <option value="census">US Census (US street addresses)</option>
          <option value="google">Google Places (API key)</option>
        </SelectField>
        <TextField label="Google Places API key" type="password" autoComplete="off" placeholder={googleSet ? "Saved" : "Not set"} value={d.googleKey} onChange={(e) => set("googleKey", e.target.value)} />
      </div>
    </section>
  );
}

function ImportPane({ d, set }: { d: Draft; set: SetFn }) {
  return (
    <section>
      <h3>Files</h3>
      <div className={styles.row}>
        <SelectField label="Format" value={d.format} onChange={(e) => set("format", e.target.value as Draft["format"])}>
          <option value="mp4">MP4, H.264</option>
          <option value="mov">MOV, original MJPEG frames</option>
        </SelectField>
        <SelectField label="MP4 encoder" value={d.encoder} onChange={(e) => set("encoder", e.target.value as Draft["encoder"])}>
          <option value="videotoolbox">VideoToolbox (hardware)</option>
          <option value="x264">x264 (software)</option>
        </SelectField>
      </div>
      <TextField label="Default short name" placeholder="flight" value={d.defaultName} onChange={(e) => set("defaultName", e.target.value)} />
      <Checkbox label="Add the time to file names (radio-log dates only)" checked={d.addTime} onChange={(e) => set("addTime", e.target.checked)} />
      <Checkbox label="Delete clips after import" detail="Deletes each clip that verified from the card or folder. Other files stay." checked={d.deleteClips} onChange={(e) => set("deleteClips", e.target.checked)} />
      <Checkbox label="Join split recordings" detail="Imports a recording the DVR split into several files as one clip." checked={d.joinSplit} onChange={(e) => set("joinSplit", e.target.checked)} />
      <h3>Log matching</h3>
      <div className={styles.row}>
        <TextField label="Segment gap (s)" type="number" min={1} step={1} value={d.segGap} onChange={(e) => set("segGap", e.target.value)} />
        <TextField label="Session gap (min)" type="number" min={1} step={1} value={d.sessionGap} onChange={(e) => set("sessionGap", e.target.value)} />
        <TextField label="Tolerance (s)" type="number" min={0} step={1} value={d.tolerance} onChange={(e) => set("tolerance", e.target.value)} />
        <TextField label="Clip clock skew (s)" type="number" min={0} step={1} value={d.clockSkew} onChange={(e) => set("clockSkew", e.target.value)} />
      </div>
    </section>
  );
}

function AdvancedPane() {
  const env = useStore((s) => s.env);
  const lines = [env?.tools ? `ffmpeg: ${env.tools.ffmpeg}` : env?.error, env?.socket ? `Agent socket: ${env.socket}` : ""].filter(Boolean);
  return (
    <section>
      <h3>Tools</h3>
      <pre className={`${styles.tools} mono selectable`}>{lines.join("\n")}</pre>
    </section>
  );
}

