// The apply sheet (design 2.4) for an FC: the change's diff, each guard with a pass mark or
// its reason, then Cancel and Apply. Return does not press Apply. During the write a row
// shows the step; after it, "Verified" or the lines that failed, with Restore backup.
import { useStore } from "../../../store";
import { SIMS_DEVICE } from "../../../store/gear";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Icon } from "../../../components/Icon";
import { ChecksList } from "../../../components/gear/ChecksList";
import { DiffView, type DiffEntry } from "../../../components/gear/DiffView";
import { deviceName } from "../../../lib/gear";
import type { ApplyReport } from "../../../ipc/types";
import styles from "./Apply.module.css";

export function ApplySheet() {
  const a = useStore((s) => s.applySheet);
  const devices = useStore((s) => s.devices);
  const jobs = useStore((s) => s.gear?.jobs);
  const connected = useStore((s) => s.gear?.connected);
  const changes = useStore((s) => s.changes);
  const close = useStore((s) => s.closeApply);
  const run = useStore((s) => s.runApply);
  const restore = useStore((s) => s.restoreBeforeApply);
  const next = useStore((s) => s.nextApply);
  const dev = a ? devices.find((d) => d.id === a.device) : undefined;
  const flash = !!a?.flash || a?.change?.id === "flash";
  const kind = flash ? "firmware" : a?.device === SIMS_DEVICE ? "sims" : dev?.kind === "radio" ? "card" : dev?.kind === "elrs_tx" || dev?.kind === "elrs_rx" ? "ELRS" : "FC";
  const name = kind === "sims" ? "sims" : dev ? deviceName(dev) : kind === "card" || kind === "firmware" ? "radio" : "FC";
  const job = a ? jobs?.find((j) => j.device === a.device) : undefined;
  const more = a?.change ? changes.filter((c) => c.device === a.device && (c.status === "ready" || c.status === "try") && c.id !== a.change?.id).length : 0;
  const overUsb = kind === "card" && !!connected?.find((c) => c.id === a?.device)?.usb;
  const done = !!a?.report;
  const ready = !!a?.plan && a.plan.checks.every((c) => c.ok);
  const working = !!a?.busy && !done;
  return (
    <Dialog
      open={!!a}
      kind="sheet"
      blockReturn
      blockEscape={working}
      title={kind === "firmware" ? `Flash ${name}` : `Apply to ${name}`}
      onClose={() => close()}
      actions={
        done ? (
          <>
            {a?.report?.status === "failed" && a.report.backup && kind !== "sims" && kind !== "firmware" && (kind === "FC" || a.report.steps.some((x) => x.name === "Roll back" && x.state === "failed")) && (
              <Button variant="ghost" onClick={() => restore()}>
                Restore backup
              </Button>
            )}
            {more > 0 ? (
              <Button variant="primary" onClick={() => next()}>
                Next change
              </Button>
            ) : (
              <Button variant="primary" onClick={() => close()}>
                Done
              </Button>
            )}
          </>
        ) : (
          <>
            <Button type="submit" value="cancel" variant="ghost" disabled={working}>
              Cancel
            </Button>
            <Button variant="primary" disabled={!ready || working} onClick={() => run()}>
              Apply
            </Button>
          </>
        )
      }
    >
      {a && (
        <div className={styles.sheet}>
          {a.agent != null && !done && <Banner icon="info">An agent, or "Apply ready changes", asked to apply this change. It goes ahead only if you click Apply.</Banner>}
          {a.error && <Banner kind="error">{a.error}</Banner>}
          {!a.change && !a.error && <p className={styles.muted}>No staged changes for this device.</p>}
          {a.change && (
            <div className={styles.head}>
              <h3>{a.change.title || (kind === "card" ? "Radio card" : kind === "sims" ? "Sim rates" : kind === "firmware" ? "Radio firmware" : "FC settings")}</h3>
              {a.plan && (
                <p className={`${styles.muted} selectable`}>
                  {[a.plan.device.board, a.plan.device.firmware, a.plan.device.version].filter(Boolean).join(" · ")}
                </p>
              )}
              {more > 0 && <p className={styles.muted}>{more === 1 ? "1 more change waits." : `${more} more changes wait.`}</p>}
            </div>
          )}
          {a.plan && !done && (
            <>
              <section aria-label="Changes">
                <h4>Changes</h4>
                <DiffView items={a.plan.diff as DiffEntry[]} />
              </section>
              <section aria-label="Checks">
                <h4>Checks</h4>
                <ChecksList checks={a.plan.checks} />
              </section>
              {(a.plan.warnings ?? []).length > 0 && (
                <section aria-label="Warnings">
                  <h4>Warnings</h4>
                  <ul className={styles.warnings}>
                    {a.plan.warnings!.map((w) => (
                      <li key={w}>{w}</li>
                    ))}
                  </ul>
                </section>
              )}
              {kind === "firmware" && dev?.kind === "fc" ? (
                <p className={styles.muted}>QuadCam saves the FC's settings first and keeps that backup, restarts the FC into its bootloader, reads the firmware the FC runs now twice and keeps it as a copy, erases and writes the flash, reads it back and compares it, and restarts the FC. It then puts your settings back and checks them against the FC. Unplug the battery first. If the read back differs, the FC stays in its bootloader.</p>
              ) : kind === "firmware" ? (
                <p className={styles.muted}>QuadCam reads the firmware the radio runs now twice and keeps it as a copy, then erases and writes the flash in segments, reads each back, compares the whole image, and restarts the radio. The radio must be in DFU mode: turn it off and plug in the USB cable, holding no button. If a read back differs, the radio stays in DFU mode.</p>
              ) : kind === "sims" ? (
                <p className={styles.muted}>QuadCam backs up each file first and keeps the backup, writes it, reads it back, and puts every file back if one reads wrong. Quit the game before you apply.</p>
              ) : kind === "ELRS" ? (
                <p className={styles.muted}>QuadCam reads the device again first and refuses if an option moved since you read it. It keeps the parameters as a backup, writes each option, reads them back and compares. The radio or FC stays in passthrough afterwards: restart the radio, or unplug the FC.</p>
              ) : kind === "card" ? (
                <p className={styles.muted}>QuadCam mounts the card if needed, backs it up first and keeps that backup, writes file by file, reads each back, and unmounts it. It puts every file back if one reads wrong.</p>
              ) : (
                <p className={styles.muted}>QuadCam backs the FC up first and keeps that backup. The FC restarts when the backup is read, and again when the change is saved.</p>
              )}
            </>
          )}
          {working && a.plan && (
            <p className={styles.progress} role="status">
              <Icon name="refresh" /> {job?.step || "Applying"}…
            </p>
          )}
          {working && overUsb && <p className={styles.muted}>Keep the radio plugged in until this sheet shows the result. Writes over USB are slow.</p>}
          {a.report && <Result r={a.report} />}
          {a.report?.status === "verified" && a.change?.status === "try" && <p className={styles.muted}>This was a Try change. Fly it, then keep it or revert it on the Bench.</p>}
        </div>
      )}
    </Dialog>
  );
}

function Result({ r }: { r: ApplyReport }) {
  const ok = r.status === "verified";
  return (
    <section aria-label="Result" className={styles.result} data-state={ok ? "ok" : "failed"}>
      <p className={styles.verdict}>
        <Icon name={ok ? "check-circle" : "close-circle"} tint={ok ? "green" : "red"} /> {r.message}
      </p>
      <ol className={styles.steps} aria-label="Steps">
        {r.steps.map((s) => (
          <li key={s.name} data-state={s.state}>
            <span>{s.name}</span>
            <span className="visually-hidden">{s.state}</span>
            {s.detail && <span className={`${styles.detail} mono selectable`}>{s.detail}</span>}
          </li>
        ))}
      </ol>
      {r.verify.length > 0 && (
        <ul aria-label="Lines that did not read back" className={`${styles.lines} mono selectable`}>
          {r.verify.map((f) => (
            <li key={f.line}>
              {f.line} <span className={styles.muted}>(found {f.found ?? "nothing"})</span>
            </li>
          ))}
        </ul>
      )}
      {(r.notes ?? []).map((n) => (
        <p key={n} className={styles.muted}>
          {n}
        </p>
      ))}
    </section>
  );
}
