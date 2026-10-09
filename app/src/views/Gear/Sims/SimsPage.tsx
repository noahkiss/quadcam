// Gear > Sims (design 2.1, 6.6): every sim on this Mac against one quad's rate profile in use,
// with Sync and Restore backup per sim. The sidebar badge "Out of date" counts the sims whose
// rates differ from the quad the status names (the saved FC seen last that has a backup).
import { useEffect, useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { SelectField } from "../../../components/Field";
import { api, errText } from "../../../ipc/api";
import type { RateProfile, SimRates } from "../../../ipc/types";
import { profileLabel } from "../../../lib/rates";
import { useStore } from "../../../store";
import { SimList } from "./SimList";
import styles from "../Storage/Storage.module.css";

export function SimsPage() {
  const devices = useStore((s) => s.devices);
  const preferred = useStore((s) => s.gear?.sims_quad ?? null);
  const quads = devices.filter((d) => d.kind === "fc" && d.last_backup);
  const [pick, setPick] = useState<string | null>(null);
  const [profile, setProfile] = useState<RateProfile | null>(null);
  const [sims, setSims] = useState<SimRates[]>([]);
  const [error, setError] = useState<string | null>(null);
  const sheetOpen = useStore((s) => !!s.applySheet);
  const [reload, setReload] = useState(0);
  const quad = quads.find((d) => d.id === pick) ?? quads.find((d) => d.id === preferred) ?? quads[0];
  const id = quad?.id ?? null;

  // A sync or a restore just finished: read the sims again.
  const wasOpen = useRef(false);
  useEffect(() => {
    if (wasOpen.current && !sheetOpen) setReload((r) => r + 1);
    wasOpen.current = sheetOpen;
  }, [sheetOpen]);

  useEffect(() => {
    if (!id) return;
    let gone = false;
    (async () => {
      try {
        const view = await api.gearRates({ paths: [], device: id });
        const p = view.profiles.find((x) => x.index === view.active) ?? view.profiles[0];
        if (!p) throw new Error("The latest backup holds no rate profile.");
        const list = await api.gearSims({ paths: [], device: id, profile: p.index });
        if (gone) return;
        setProfile(p);
        setSims(list);
        setError(null);
      } catch (e) {
        if (!gone) setError(errText(e));
      }
    })();
    return () => {
      gone = true;
    };
  }, [id, reload]);

  return (
    <div className={styles.page} aria-labelledby="sims-title">
      <h2 id="sims-title" className={styles.title}>
        Sims
      </h2>
      {!quad && (
        <p className={styles.muted}>
          No quad to compare with. Back up a flight controller, then its rates show here against each sim on this Mac.
        </p>
      )}
      {quad && quads.length > 1 && (
        <SelectField label="Compare with" value={quad.id} onChange={(e) => setPick(e.target.value)}>
          {quads.map((d) => (
            <option key={d.id} value={d.id}>
              {d.name || d.id}
            </option>
          ))}
        </SelectField>
      )}
      {quad && profile && (
        <p className={styles.muted}>
          {quad.name || "The quad"}: {profileLabel(profile)}, from its latest backup. A sim is out of date when its rates differ from these.
        </p>
      )}
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {quad && profile && <SimList sims={sims} error={null} profile={profile} quad={{ paths: [], device: quad.id }} heading={false} />}
    </div>
  );
}
