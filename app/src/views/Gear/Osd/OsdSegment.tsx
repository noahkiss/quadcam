// The Aircraft page's OSD segment (design 2.2): a Betaflight dump or diff file drawn per
// OSD profile. It stands alone (dev builds: `?osd`) until the Gear page frame mounts it with
// the aircraft's FC; then `device` replaces the file (its latest backup, or a live read).
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { api, errText, pickFiles } from "../../../ipc/api";
import type { OsdView } from "../../../ipc/types";
import { OsdScreen } from "./OsdScreen";
import styles from "./OsdSegment.module.css";

type GridChoice = "auto" | "NTSC" | "PAL" | "HD";

interface Props {
  /** Files to show at once (a dump, then apply files). */
  paths?: string[];
}

export function OsdSegment({ paths: initial = [] }: Props) {
  const [paths, setPaths] = useState(initial);
  const [grid, setGrid] = useState<GridChoice>("auto");
  const [view, setView] = useState<OsdView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!paths.length) return;
    let gone = false;
    api
      .gearOsd({ paths, device: null, grid: grid === "auto" ? null : grid })
      .then(
        (v) => {
          if (gone) return;
          setView(v);
          setError(null);
        },
        (e) => {
          if (!gone) setError(errText(e));
        },
      );
    return () => {
      gone = true;
    };
  }, [paths, grid]);

  const open = async () => {
    const picked = await pickFiles("Open a Betaflight dump or diff", [
      { name: "Betaflight CLI text", extensions: ["txt", "cli"] },
    ]);
    if (picked.length) setPaths(picked);
  };

  return (
    <div className={styles.segment}>
      <div className={styles.bar}>
        <Button icon="folder-open" onClick={open}>
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
        {view && (
          <span className={styles.source}>
            {[view.firmware, view.source.join(", ")]
              .filter(Boolean)
              .join(" · ")}
          </span>
        )}
      </div>
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {view ? (
        <OsdScreen view={view} />
      ) : (
        !error && (
          <p className={styles.empty}>
            Open a Betaflight dump or diff to see its OSD.
          </p>
        )
      )}
    </div>
  );
}
