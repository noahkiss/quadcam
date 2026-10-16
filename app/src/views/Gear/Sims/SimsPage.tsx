// Gear > Sim > Rates (design 2.1, 6.6): every sim on this Mac against one quad's rate profile
// in use, with Sync and Restore backup per sim. The sidebar badge "Out of date" counts the
// sims whose rates differ from the quad the status names (the saved FC seen last that has a
// backup).
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Banner } from "../../../components/Banner";
import { EmptyState } from "../../../components/dash/EmptyState";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { api, errText } from "../../../ipc/api";
import type { RateProfile, SimRates } from "../../../ipc/types";
import { profileLabel } from "../../../lib/rates";
import { useStore } from "../../../store";
import { SimTable } from "./SimTable";
import styles from "./Sims.module.css";

export function SimsPage({ tabs }: { tabs?: ReactNode }) {
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

  const shown = quad && profile ? sims : [];
  const found = shown.filter((s) => s.enabled && s.found).length;
  const late = shown.filter((s) => s.in_sync === false).length;
  const running = shown.filter((s) => s.running).length;

  return (
    <>
      <PageHeader
        id="sims-title"
        title="Sim"
        subtitle={
          quad &&
          profile && (
            <SummaryLine
              parts={[
                `${found} ${found === 1 ? "sim" : "sims"} found`,
                late > 0 && { text: `${late} out of date`, tone: "warn" },
                running > 0 && `${running} running`,
                `Compared with ${quad.name || quad.id} · ${profileLabel(profile)}`,
              ]}
            />
          )
        }
        actions={
          quad &&
          quads.length > 1 && (
            <label className={styles.compare}>
              Compare with
              <select value={quad.id} onChange={(e) => setPick(e.target.value)}>
                {quads.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name || d.id}
                  </option>
                ))}
              </select>
            </label>
          )
        }
      />
      {tabs}
      {!quad && <EmptyState icon="play">No quad to compare with. Back up a flight controller first.</EmptyState>}
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {quad && profile && <SimTable sims={sims} error={null} profile={profile} quad={{ paths: [], device: quad.id }} />}
    </>
  );
}
