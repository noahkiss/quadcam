// The Aircraft page's Rates segment (design 2.2, read only): every rate profile of the FC's
// latest backup, or of a dump or diff file, drawn as curves for roll, pitch and yaw with the
// maximum and centre rates, and the throttle curve. A second curve compares the profile with
// another profile, another quad or a sim's profile; the sims' own rates are listed with
// "Matches the quad" or "Out of date". On a saved FC, Edit changes a profile with the
// curves redrawing live and stages the change (the apply sheet writes it), and each sim profile
// has a Sync button that opens the apply sheet on a plan to write the quad's rates into it. A sim
// QuadCam backed up shows when, with Restore backup: the same sheet, on a plan to put the file back.
import { useEffect, useMemo, useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Chip } from "../../../components/Chip";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { api, errText, pickFiles } from "../../../ipc/api";
import type { RateProfile, RatesView, SimRates } from "../../../ipc/types";
import {
  AXIS_LABEL,
  LIMIT_LABEL,
  RATES_TYPE,
  deg,
  niceMax,
  profileLabel,
  profileTarget,
  simTargets,
  type CompareTarget,
} from "../../../lib/rates";
import { useStore } from "../../../store";
import { RateEditor } from "./RateEditor";
import { RatesChart } from "./RatesChart";
import { SimList } from "../Sims/SimList";
import styles from "./Rates.module.css";

interface Props {
  /** Files to show at once (a dump, then apply files). */
  paths?: string[];
  /** A saved FC with a backup: its latest `dump all` shows until a file is opened. */
  device?: string | null;
}

const NONE = "none";
const pct = (v: number) => `${Math.round(v * 100)}%`;

export function RatesSegment({ paths: initial = [], device = null }: Props) {
  const devices = useStore((s) => s.devices);
  const [paths, setPaths] = useState(initial);
  const [view, setView] = useState<RatesView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const [sims, setSims] = useState<SimRates[]>([]);
  const [simError, setSimError] = useState<string | null>(null);
  const [against, setAgainst] = useState(NONE);
  const [others, setOthers] = useState<Record<string, RateProfile>>({});
  const [editing, setEditing] = useState(false);
  const [draftView, setDraftView] = useState<RateProfile | null>(null);
  const [reload, setReload] = useState(0);
  const sheetOpen = useStore((s) => !!s.applySheet);
  const wasOpen = useRef(false);

  const source = paths.length ? { paths, device: null } : { paths: [], device };
  const has = paths.length > 0 || !!device;

  useEffect(() => {
    if (!has) return;
    let gone = false;
    api.gearRates(source).then(
      (v) => {
        if (gone) return;
        setView(v);
        setError(null);
        if (reload === 0) setPicked(null);
      },
      (e) => !gone && setError(errText(e)),
    );
    return () => {
      gone = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paths, device, reload]);

  // The apply sheet wrote something (a profile, or a sim file): read both again.
  useEffect(() => {
    if (wasOpen.current && !sheetOpen) setReload((r) => r + 1);
    wasOpen.current = sheetOpen;
  }, [sheetOpen]);

  const profile = view ? (view.profiles.find((p) => p.index === picked) ?? view.profiles.find((p) => p.index === view.active) ?? view.profiles[0]) : undefined;

  useEffect(() => {
    if (!view || !profile) return;
    let gone = false;
    api.gearSims({ ...source, profile: profile.index }).then(
      (s) => {
        if (gone) return;
        setSims(s);
        setSimError(null);
      },
      (e) => !gone && setSimError(errText(e)),
    );
    return () => {
      gone = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, profile?.index, reload]);

  // Other saved FCs with a backup: their profile in use is a target.
  const quads = devices.filter((d) => d.kind === "fc" && d.last_backup && d.id !== device);
  useEffect(() => {
    if (!against.startsWith("quad:")) return;
    const id = against.slice(5);
    if (others[id]) return;
    api.gearRates({ paths: [], device: id }).then(
      (v) => {
        const p = v.profiles.find((x) => x.index === v.active) ?? v.profiles[0];
        if (p) setOthers((o) => ({ ...o, [id]: p }));
      },
      (e) => setError(errText(e)),
    );
  }, [against, others]);

  const targets = useMemo<CompareTarget[]>(() => {
    if (!view || !profile) return [];
    const mine = view.profiles.filter((p) => p.index !== profile.index).map((p) => ({ ...profileTarget(p, "profile", profileLabel(p)), key: `profile:${p.index}` }));
    const quad = quads.map((d) => {
      const p = others[d.id];
      return { key: `quad:${d.id}`, group: "quad" as const, label: `${d.name || d.id} · ${p ? profileLabel(p) : "profile in use"}`, axes: p?.axes ?? [], throttle: p?.throttle ?? null };
    });
    return [...mine, ...quad, ...simTargets(sims)];
  }, [view, profile, quads, others, sims]);
  const target = targets.find((t) => t.key === against);

  const open = async () => {
    const files = await pickFiles("Open a Betaflight dump or diff", [{ name: "Betaflight CLI text", extensions: ["txt", "cli"] }]);
    if (files.length) {
      setPaths(files);
      setAgainst(NONE);
    }
  };

  // While editing, the charts show the edited curves against the profile as it is now.
  const shown = editing && draftView && profile ? draftView : profile;
  const axes = shown?.axes ?? [];
  const compareAxes = editing && profile ? profile.axes : target?.axes;
  const compareThrottle = editing && profile ? profile.throttle : target?.throttle;
  const compareName = editing ? "Before the edit" : target?.label;
  const top = niceMax([...axes.map((a) => a.max_deg_s), ...(compareAxes?.map((a) => a.max_deg_s) ?? [])]);
  const canEdit = !!device && paths.length === 0 && !!profile;

  return (
    <div className={styles.segment}>
      <div className={styles.bar}>
        <Button icon="folder-open" onClick={open}>
          Open dump…
        </Button>
        {view && <span className={styles.source}>{[view.firmware, view.source.join(", ")].filter(Boolean).join(" · ")}</span>}
      </div>
      {error && (
        <Banner kind="error" icon="danger-triangle" tint="red">
          {error}
        </Banner>
      )}
      {!view && !error && <p className={styles.empty}>Open a Betaflight dump or diff to see its rates.</p>}
      {view && profile && (
        <>
          <div className={styles.bar}>
            <SegmentedControl
              label="Rate profile"
              value={String(profile.index)}
              onChange={(v) => {
                setPicked(Number(v));
                setAgainst(NONE);
              }}
              segments={view.profiles.map((p) => ({ value: String(p.index), label: profileLabel(p) }))}
            />
            {profile.active && <Chip kind="accent">In use</Chip>}
            {canEdit && !editing && (
              <Button icon="pen" onClick={() => setEditing(true)}>
                Edit profile
              </Button>
            )}
            <label className={styles.compareLabel}>
              Compare with
              <select value={against} onChange={(e) => setAgainst(e.target.value)} aria-label="Compare with">
                <option value={NONE}>Nothing</option>
                {(["profile", "quad", "sim"] as const).map((g) => {
                  const list = targets.filter((t) => t.group === g);
                  return list.length ? (
                    <optgroup key={g} label={g === "profile" ? "Other profiles of this quad" : g === "quad" ? "Other quads" : "Sims"}>
                      {list.map((t) => (
                        <option key={t.key} value={t.key}>
                          {t.label}
                        </option>
                      ))}
                    </optgroup>
                  ) : null;
                })}
              </select>
            </label>
          </div>
          {view.notes.map((n) => (
            <Banner key={n} kind="info" icon="info">
              {n}
            </Banner>
          ))}
          <p className={styles.meta}>
            {RATES_TYPE[profile.rates_type] ?? profile.rates_type} rates
            {!profile.complete && ", defaults filled in"}
          </p>
          <div className={styles.charts} role="group" aria-label={`Curves of rate profile ${profileLabel(profile)}`}>
            {axes.map((a, i) => {
              const other = compareAxes?.[i];
              return (
                <RatesChart
                  key={a.axis}
                  title={AXIS_LABEL[a.axis] ?? a.axis}
                  summary={`${AXIS_LABEL[a.axis] ?? a.axis}: ${deg(a.center_deg_s)} per full stick at the centre, ${deg(a.max_deg_s)} at full stick.${other ? ` ${compareName}: ${deg(other.max_deg_s)} at full stick.` : ""}`}
                  main={a.curve}
                  compare={other?.curve}
                  max={top}
                  tick={(v) => `${Math.round(v)}`}
                />
              );
            })}
            <RatesChart
              title="Throttle"
              summary={`Throttle: mid ${shown!.throttle.mid}, expo ${shown!.throttle.expo}${shown!.throttle.hover != null ? `, hover ${shown!.throttle.hover}` : ""}, limit ${LIMIT_LABEL[shown!.throttle.limit] ?? shown!.throttle.limit} ${shown!.throttle.limit_percent}%.${compareThrottle ? ` ${compareName} is dashed.` : ""}`}
              main={shown!.throttle.curve}
              compare={compareThrottle?.curve}
              max={1}
              tick={pct}
            />
          </div>
          {(editing || target) && (
            <p className={styles.legend}>
              <span className={styles.keyMain} aria-hidden="true" /> {editing ? "Edited" : profileLabel(profile)}
              <span className={styles.keyCompare} aria-hidden="true" /> {compareName}
            </p>
          )}
          {editing && (
            <RateEditor
              key={`${device}:${profile.index}`}
              profile={profile}
              device={device!}
              onDraft={setDraftView}
              onClose={() => {
                setEditing(false);
                setDraftView(null);
              }}
            />
          )}
          {!editing && <table className={styles.table} aria-label={`Rates of ${profileLabel(profile)}`}>
            <thead>
              <tr>
                <th scope="col">Axis</th>
                <th scope="col">RC rate</th>
                <th scope="col">Super rate</th>
                <th scope="col">Expo</th>
                <th scope="col">Limit</th>
                <th scope="col">Centre</th>
                <th scope="col">Maximum</th>
                {target && <th scope="col">Maximum, {target.label}</th>}
              </tr>
            </thead>
            <tbody>
              {axes.map((a, i) => (
                <tr key={a.axis}>
                  <th scope="row">{AXIS_LABEL[a.axis] ?? a.axis}</th>
                  <td>{a.rc_rate}</td>
                  <td>{a.srate}</td>
                  <td>{a.expo}</td>
                  <td>{a.rate_limit}</td>
                  <td>{deg(a.center_deg_s)}</td>
                  <td>{deg(a.max_deg_s)}</td>
                  {target && <td>{target.axes[i] ? deg(target.axes[i].max_deg_s) : "–"}</td>}
                </tr>
              ))}
            </tbody>
          </table>}
          <p className={styles.meta}>
            Throttle: mid {profile.throttle.mid}, expo {profile.throttle.expo}
            {profile.throttle.hover != null && `, hover ${profile.throttle.hover}`}, limit {LIMIT_LABEL[profile.throttle.limit] ?? profile.throttle.limit} {profile.throttle.limit_percent}%
          </p>
          <SimList sims={sims} error={simError} profile={profile} quad={source} />
        </>
      )}
    </div>
  );
}
