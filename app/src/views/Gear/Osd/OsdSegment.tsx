// The Aircraft page's OSD segment (design 2.2): the FC's latest backup, or a Betaflight
// dump or diff file, drawn per OSD profile. The FC's device page mounts it
// (`views/Gear/segments.tsx`); an opened file replaces the backup.
//
// On a saved FC the segment is the editor (design 7.3): moves, toggles and profile copies
// stage one "OSD layout" change through `gear_osd_edit`, and the grid shows the layout
// with that change on top. Nothing reaches the FC until the apply sheet writes it.
import { useEffect, useMemo, useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Select } from "../../../components/Field";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { api, errText, pickFiles } from "../../../ipc/api";
import type { OsdMove, OsdView } from "../../../ipc/types";
import { useStore } from "../../../store";
import { CopyDialog } from "../Bench/CopyDialog";
import { OsdScreen } from "./OsdScreen";
import styles from "./OsdSegment.module.css";

type GridChoice = "auto" | "NTSC" | "PAL" | "HD";

/** The title of the change the editor keeps its edits in (`core/osd.rs`). */
const OSD_CHANGE = "OSD layout";

interface Props {
  /** Files to show at once (a dump, then apply files). */
  paths?: string[];
  /** A saved FC with a backup: its latest `dump all` shows until a file is opened. */
  device?: string | null;
}

/** The view with moves that are still on their way to the core drawn as if they had landed. */
function withPending(view: OsdView, pending: Record<string, { x: number; y: number }>): OsdView {
  const names = Object.keys(pending);
  if (!names.length) return view;
  const els = view.elements.map((e) => (pending[e.name] ? { ...e, ...pending[e.name] } : e));
  const shift = new Map(view.elements.filter((e) => pending[e.name]).map((e) => [e.name, [pending[e.name].x - e.x, pending[e.name].y - e.y] as const]));
  const profiles = view.profiles.map((p) => ({
    ...p,
    boxes: p.boxes.map((b) => {
      const d = shift.get(b.element);
      return d ? { ...b, x: b.x + d[0], y: b.y + d[1] } : b;
    }),
  }));
  return { ...view, elements: els, profiles };
}

export function OsdSegment({ paths: initial = [], device = null }: Props) {
  const [paths, setPaths] = useState(initial);
  const [grid, setGrid] = useState<GridChoice>("auto");
  const [view, setView] = useState<OsdView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [pending, setPending] = useState<Record<string, { x: number; y: number }>>({});
  const [copy, setCopy] = useState({ from: "1", to: "2" });
  const [copying, setCopying] = useState(false);
  const loadGear = useStore((s) => s.loadGear);
  const open = useStore((s) => s.openApply);
  const discard = useStore((s) => s.discardChange);
  const changes = useStore((s) => s.changes);
  const editable = !!device && paths.length === 0;
  const mine = useMemo(() => changes.filter((c) => c.device === device), [changes, device]);
  // The staged OSD edits decide when the working view needs reading again.
  const stagedKey = JSON.stringify(mine.flatMap((c) => c.edits.filter((e) => e.kind === "osd_element")));
  const osdChange = mine.find((c) => c.title === OSD_CHANGE && (c.status === "draft" || c.status === "ready"));
  const queue = useRef<Promise<void>>(Promise.resolve());
  const last = useRef<string | null>(null);

  useEffect(() => {
    if (!paths.length && !device) return;
    let gone = false;
    api
      .gearOsd({ paths, device: paths.length ? null : device, grid: grid === "auto" ? null : grid, staged: editable })
      .then(
        (v) => {
          if (gone) return;
          setView(v);
          setError(null);
          setPending({});
          const e = last.current && v.elements.find((x) => x.name === last.current);
          if (e) {
            setStatus(`${e.label} is at column ${e.x}, row ${e.y}, ${e.profiles.length ? `shown in profile ${e.profiles.join(", ")}` : "off in every profile"}.`);
            last.current = null;
          }
        },
        (e) => {
          if (!gone) setError(errText(e));
        },
      );
    return () => {
      gone = true;
    };
  }, [paths, grid, device, editable, stagedKey]);

  const openFile = async () => {
    const picked = await pickFiles("Open a Betaflight dump or diff", [{ name: "Betaflight CLI text", extensions: ["txt", "cli"] }]);
    if (picked.length) setPaths(picked);
  };

  /** Runs one edit after the ones before it, then lets the store refresh the staged changes. */
  const run = (edit: () => Promise<unknown>) => {
    queue.current = queue.current.then(async () => {
      try {
        await edit();
        setError(null);
      } catch (e) {
        setPending({});
        setError(errText(e));
      }
      await loadGear();
    });
  };
  const move = (m: OsdMove) => {
    if (!device) return;
    const el = view?.elements.find((e) => e.name === m.element);
    if (el && (m.x != null || m.y != null)) setPending((p) => ({ ...p, [m.element]: { x: m.x ?? p[m.element]?.x ?? el.x, y: m.y ?? p[m.element]?.y ?? el.y } }));
    last.current = m.element;
    run(() => api.gearOsdEdit({ device, moves: [m], copy: null }));
  };
  const copyProfile = () => {
    if (!device || copy.from === copy.to) return;
    last.current = null;
    setStatus(`Profile ${copy.to} now shows what profile ${copy.from} shows.`);
    run(() => api.gearOsdEdit({ device, moves: [], copy: { from: Number(copy.from), to: Number(copy.to) } }));
  };

  const shown = useMemo(() => (view ? withPending(view, pending) : null), [view, pending]);
  const profileOpts = [1, 2, 3].map((p) => (
    <option key={p} value={p}>
      {p}
    </option>
  ));
  const n = osdChange?.edits.length ?? 0;
  return (
    <div className={styles.segment}>
      <div className={styles.bar}>
        <Button icon="folder-open" onClick={openFile}>
          Open dump…
        </Button>
        <SegmentedControl
          label="Grid"
          size="sm"
          value={grid}
          onChange={setGrid}
          segments={[
            { value: "auto", label: "Auto" },
            { value: "NTSC", label: "NTSC" },
            { value: "PAL", label: "PAL" },
            { value: "HD", label: "HD" },
          ]}
        />
        {view && <span className={styles.source}>{[view.firmware, view.source.join(", ")].filter(Boolean).join(" · ")}</span>}
      </div>
      {editable && view && (
        <div className={styles.bar} role="group" aria-label="Copy layout">
          <label className={styles.inline}>
            Copy profile
            <Select aria-label="Copy from profile" value={copy.from} onChange={(e) => setCopy({ ...copy, from: e.target.value })}>
              {profileOpts}
            </Select>
          </label>
          <label className={styles.inline}>
            to profile
            <Select aria-label="Copy to profile" value={copy.to} onChange={(e) => setCopy({ ...copy, to: e.target.value })}>
              {profileOpts}
            </Select>
          </label>
          <Button size="sm" disabled={copy.from === copy.to} onClick={copyProfile}>
            Copy profile
          </Button>
          <Button size="sm" onClick={() => setCopying(true)}>
            Copy from another quad…
          </Button>
        </div>
      )}
      {editable && n > 0 && osdChange && (
        <Banner
          kind="info"
          icon="info"
          tint="blue"
          action={
            <>
              <Button size="sm" onClick={() => device && open(device, osdChange.id)}>
                Review…
              </Button>
              <Button size="sm" variant="danger-ghost" onClick={() => discard(osdChange.id)}>
                Undo OSD edits
              </Button>
            </>
          }
        >
          {n === 1 ? "1 OSD edit is staged." : `${n} OSD edits are staged.`} The FC changes when you apply them.
        </Banner>
      )}
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {shown ? (
        <>
          <OsdScreen view={shown} edit={editable ? { move, status } : undefined} />
          {editable && <p className={styles.hint}>An element has one position for every profile. Drag it, or focus it and press the arrow keys (Shift moves five cells). The profile boxes only turn it on or off.</p>}
        </>
      ) : (
        !error && <p className={styles.empty}>Open a Betaflight dump or diff to see its OSD.</p>
      )}
      {copying && device && (
        <CopyDialog
          to={device}
          parts={["osd"]}
          onClose={() => {
            setCopying(false);
            void loadGear();
          }}
        />
      )}
    </div>
  );
}
