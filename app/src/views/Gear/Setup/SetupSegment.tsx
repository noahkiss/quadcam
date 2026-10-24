// The radio's Setup segment (`docs/radio-design.md` 5.6): one row per job with its channel,
// the control that does it and its direction, and what each position sends; the fitted
// part in each switch and pot slot; Learn (move a control on the radio in USB Joystick
// mode); Copy from another radio; Review, which stages the one "Radio setup" change and
// opens the apply sheet. The setup is QuadCam's own data and saves as it changes; nothing
// reaches the radio until the apply sheet writes it.
import { useEffect, useEffectEvent, useId, useRef, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Select } from "../../../components/Field";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { SectionCard } from "../../../components/dash/SectionCard";
import { api, errText } from "../../../ipc/api";
import type { JobRow, RadioSetupDetail, SlotRow, SwitchMap } from "../../../ipc/types";
import { assign, atText, controlChoices, controlText, defaultHigh, fit, generalProblems, hasPositions, kindsOf, learnBaseline, learnPick, learnResult, positionText, problemsFor, slotTypeText, sourceWords, statusText, type LearnBaseline } from "../../../lib/setup";
import { useStore } from "../../../store";
import { discardWithAsk } from "../Changes/discard";
import type { DeviceRef } from "../slots";
import { useRadio } from "../Switches/hooks";
import { CopyDialog } from "./CopyDialog";
import { openSetupChange, useSetup, type UseSetup } from "./useSetup";
import styles from "./Setup.module.css";

export function SetupSegment({ d }: { d: DeviceRef }) {
  const device = d.device?.id ?? null;
  const s = useSetup(device);
  const [learning, setLearning] = useState<string | null>(null);
  const [copying, setCopying] = useState(false);
  const detail = s.detail;
  const general = generalProblems(detail?.problems);
  return (
    <div className={styles.segment}>
      {!device && <EmptyState icon="info">Save this radio from Overview to set it up.</EmptyState>}
      {device && detail && <StatusBar device={device} detail={detail} s={s} />}
      {s.error && (
        <Banner kind="error" icon="danger-triangle">
          {s.error}
        </Banner>
      )}
      {s.notes.map((n) => (
        <p key={n} className={styles.note}>
          {n}
        </p>
      ))}
      {general.map((p) => (
        <Banner key={p.message} kind="error" icon="danger-triangle">
          {p.message}
        </Banner>
      ))}
      {detail?.card_problem && (
        <Banner kind="warning" icon="danger-triangle" tint="peach">
          The card does not take this setup yet: {detail.card_problem}
        </Banner>
      )}
      {detail && (
        <>
          <p className={styles.source}>{[detail.setup.board.toUpperCase(), sourceWords(detail.source), detail.origin === "default" ? "The board's default setup" : null].filter(Boolean).join(" · ")}</p>
          {device && learning && <Learn device={device} detail={detail} job={learning} s={s} done={() => setLearning(null)} />}
          <SectionCard
            title="Jobs"
            actions={
              <>
                <Button size="sm" onClick={() => setCopying(true)}>
                  Copy from…
                </Button>
                <MoreMenu items={[{ label: "Start again from the default setup", run: () => void s.edit({ reset: true }) }]} />
              </>
            }
          >
            <JobsTable detail={detail} s={s} learning={learning} learn={setLearning} />
          </SectionCard>
          <SectionCard title="Fitted controls">
            <SlotsTable detail={detail} s={s} />
          </SectionCard>
        </>
      )}
      {device && copying && <CopyDialog device={device} s={s} close={() => setCopying(false)} />}
    </div>
  );
}

/** Where the radio stands against the setup, with Review. */
function StatusBar({ device, detail, s }: { device: string; detail: RadioSetupDetail; s: UseSetup }) {
  const change = useStore((st) => openSetupChange(st.changes, device));
  const open = useStore((st) => st.openApply);
  const review = async () => {
    const c = await s.edit({ stage: true });
    if (c) await open(device, c.id);
  };
  const copies = detail.copies ?? [];
  const copyText = copies.length ? ` First it copies ${copies.map((c) => `${c.name} to ${c.to}`).join(", ")} from the ${c0Board(detail)}.` : "";
  if (change) {
    return (
      <Banner
        kind="info"
        icon="info"
        tint="blue"
        action={
          <>
            <Button size="sm" disabled={s.busy || s.draft} onClick={() => void review()}>
              Review…
            </Button>
            <Button size="sm" variant="danger-ghost" onClick={() => void discardWithAsk(change, { title: "Discard the staged setup?" })}>
              Discard staged setup
            </Button>
          </>
        }
      >
        The setup is staged: {detail.files.length === 1 ? "1 file changes" : `${detail.files.length} files change`} when you apply it.{copyText}
      </Banner>
    );
  }
  if (detail.files.length && !s.draft && !detail.card_problem) {
    return (
      <Banner
        kind="info"
        icon="info"
        tint="blue"
        action={
          <Button size="sm" disabled={s.busy} onClick={() => void review()}>
            Review…
          </Button>
        }
      >
        {statusText(detail)}
      </Banner>
    );
  }
  return (
    <p className={styles.status} role="status">
      {statusText(detail)}
    </p>
  );
}

const c0Board = (d: RadioSetupDetail) => (d.copies?.[0]?.from_board ?? "").toUpperCase() || "other radio";

/** One row per job: channel, control, direction, and what each position sends. */
function JobsTable({ detail, s, learning, learn }: { detail: RadioSetupDetail; s: UseSetup; learning: string | null; learn: (job: string | null) => void }) {
  const kinds = kindsOf(detail);
  const setControl = (row: JobRow, control: string) => void s.change(assign(detail.setup, kinds, row.job, control || null, control === row.control ? (row.high ?? null) : defaultHigh(row)));
  const setHigh = (row: JobRow, high: number) => void s.change(assign(detail.setup, kinds, row.job, row.control ?? null, high));
  const columns: Column<JobRow>[] = [
    { key: "job", header: "Job", rowHeader: true, cell: (r) => r.label },
    { key: "ch", header: "Channel", cell: (r) => r.channel ?? "None", width: "6rem" },
    {
      key: "control",
      header: "Control",
      cell: (r) => <ControlCell row={r} detail={detail} onChange={(c) => setControl(r, c)} />,
    },
    {
      key: "dir",
      header: "2000 µs at",
      cell: (r) =>
        r.control && hasPositions(r.kind) ? (
          <Select aria-label={`${r.label}: 2000 µs at`} value={String(r.high ?? 2)} onChange={(e) => setHigh(r, Number(e.target.value))}>
            <option value="2">{atText(r.control, 2, r.kind)}</option>
            <option value="0">{atText(r.control, 0, r.kind)}</option>
          </Select>
        ) : (
          <span className={styles.muted}>{r.control ? "Follows the control" : ""}</span>
        ),
    },
    {
      key: "states",
      header: "What it sends",
      optional: true,
      cell: (r) => (
        <ul className={styles.states} aria-label={`${r.label}: what it sends`}>
          {r.states.map((st, i) => (
            <li key={i}>
              {st.position && <span className={styles.pos}>{positionText(st.position, r.kind)}</span>}
              {st.us != null && <span className={styles.us}>{st.us} µs</span>}
              <span>{st.words}</span>
            </li>
          ))}
        </ul>
      ),
    },
    {
      key: "learn",
      header: <span className={styles.visuallyHidden}>Learn</span>,
      width: "6rem",
      cell: (r) => (
        <Button size="sm" variant="ghost" aria-label={`Learn ${r.label}`} aria-pressed={learning === r.job} onClick={() => learn(learning === r.job ? null : r.job)}>
          Learn
        </Button>
      ),
    },
  ];
  return <DataTable label="Jobs" columns={columns} rows={detail.jobs} rowKey={(r) => r.job} />;
}

/** The Control pop-up, with the row's problems under it. */
function ControlCell({ row, detail, onChange }: { row: JobRow; detail: RadioSetupDetail; onChange: (control: string) => void }) {
  const hintId = useId();
  const problems = problemsFor(detail.problems, row.job);
  // A control the setup names that the radio lacks (a blanked slot) stays in the list.
  const assigned = detail.setup.assignments.find((a) => a.job === row.job)?.control ?? null;
  const choices = controlChoices(row);
  if (assigned && !choices.includes(assigned)) choices.unshift(assigned);
  const value = row.control ?? assigned ?? "";
  return (
    <div className={styles.cell}>
      <Select aria-label={`${row.label}: control`} aria-invalid={problems.length ? true : undefined} aria-describedby={problems.length ? hintId : undefined} value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">None</option>
        {choices.map((c) => (
          <option key={c} value={c}>
            {controlText(c)}
          </option>
        ))}
      </Select>
      {problems.length > 0 && (
        <span id={hintId} className={styles.error} role="alert">
          {problems.map((p) => p.message).join(" ")}
        </span>
      )}
    </div>
  );
}

/** Each switch and pot slot: the factory part, what is fitted, and what the radio says. */
function SlotsTable({ detail, s }: { detail: RadioSetupDetail; s: UseSetup }) {
  const columns: Column<SlotRow>[] = [
    { key: "slot", header: "Slot", rowHeader: true, cell: (r) => r.control, width: "6rem" },
    { key: "factory", header: "Factory part", cell: (r) => slotTypeText(r.kind, r.factory) },
    {
      key: "fitted",
      header: "Fitted",
      cell: (r) => (
        <Select aria-label={`${r.control}: fitted`} value={r.fitted} onChange={(e) => void s.change(fit(detail.setup, r.control, e.target.value, r.factory))}>
          {r.choices.map((c) => (
            <option key={c} value={c}>
              {slotTypeText(r.kind, c)}
            </option>
          ))}
        </Select>
      ),
    },
    {
      key: "radio",
      header: "On the radio",
      optional: true,
      cell: (r) =>
        r.on_radio == null ? <span className={styles.muted}>Not listed</span> : r.on_radio.toLowerCase() === r.fitted.toLowerCase() ? <span className={styles.muted}>Same</span> : <span className={styles.changed}>{slotTypeText(r.kind, r.on_radio)}</span>,
    },
  ];
  return <DataTable label="Fitted controls" columns={columns} rows={detail.slots ?? []} rowKey={(r) => r.control} compact empty={<EmptyState icon="info">This board has no switch or pot slots QuadCam knows.</EmptyState>} />;
}

/** Learn: the next control that moves on the radio (USB Joystick mode) does the job. */
function Learn({ device, detail, job, s, done }: { device: string; detail: RadioSetupDetail; job: string; s: UseSetup; done: () => void }) {
  const radio = useRadio(true);
  const [map, setMap] = useState<SwitchMap | null>(null);
  const [mapError, setMapError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const baseline = useRef<LearnBaseline | null>(null);
  const row = detail.jobs.find((r) => r.job === job);
  const frame = radio?.frame ?? null;
  const onFrame = useEffectEvent((channels: number[]) => {
    if (!map || !row) return;
    if (!baseline.current) {
      baseline.current = learnBaseline(map, channels);
      return;
    }
    const pick = learnPick(map, baseline.current, channels);
    if (!pick) return;
    const r = learnResult(row, pick, kindsOf(detail));
    if ("error" in r) {
      setMessage(r.error);
      baseline.current = learnBaseline(map, channels);
      return;
    }
    done();
    void s.change(assign(detail.setup, kindsOf(detail), row.job, r.control, r.high));
  });

  useEffect(() => {
    let gone = false;
    api.gearSwitchMap({ devices: [device], aircraft: null }).then(
      (m) => !gone && setMap(m),
      (e) => !gone && setMapError(errText(e)),
    );
    return () => {
      gone = true;
    };
  }, [device]);

  useEffect(() => {
    if (frame) onFrame(frame.channels);
  }, [frame, map]);

  if (!row) return null;
  const last = row.states.at(-1)?.words ?? "on";
  const status = mapError
    ? mapError
    : !radio
      ? "Looking for the radio…"
      : !radio.connected
        ? "Plug the radio in and choose USB Joystick on it."
        : `Move the control for ${row.label} to “${last}”. Learn sees controls the radio's model sends on CH1-CH8.`;
  return (
    <Banner
      kind="info"
      icon="radio"
      tint="blue"
      action={
        <Button size="sm" onClick={done}>
          Cancel
        </Button>
      }
    >
      <span role="status" aria-label="Learn">
        Learn {row.label}: {status}
      </span>
      {message && <span className={styles.learnError}> {message}</span>}
    </Banner>
  );
}
