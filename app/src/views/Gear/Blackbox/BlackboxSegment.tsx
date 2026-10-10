// An FC page's Blackbox segment (design 7.12): pull the flash now, erase it by hand, and
// the stored pulls with their logs and the flights QuadCam guesses they belong to.
import { useEffect, useState } from "react";
import { ask, useStore } from "../../../store";
import { api, errText } from "../../../ipc/api";
import type { BlackboxEntry, BlackboxPullResult } from "../../../ipc/types";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { toast } from "../../../components/toastStore";
import { fmtBytes, plural } from "../../../lib/format";
import { failureFor, fmtWhen, jobFor, jobText } from "../../../lib/backups";
import { linkHandle } from "../../../lib/gear";
import type { DeviceRef } from "../slots";
import styles from "./Blackbox.module.css";

const idOf = (d: DeviceRef) => d.device?.id || d.connected?.id || null;

/** What a pull did, as one toast. */
export function pullText(r: BlackboxPullResult): string {
  if (!r.pull) return "The blackbox flash is empty.";
  let out = `${plural(r.pull.logs.length, "log")} stored (${fmtBytes(r.read_bytes)}).`;
  if (r.erase === "done") out += " Flash erased. Safe to unplug.";
  else if (r.erase === "skipped") out += ` ${r.erase_note || "The flash was not erased."}`;
  else out += " Flash kept.";
  return out;
}

export function BlackboxSegment({ d }: { d: DeviceRef }) {
  const status = useStore((s) => s.gear);
  const devices = useStore((s) => s.devices);
  const id = idOf(d);
  const [list, setList] = useState<BlackboxEntry[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const job = jobFor(status, id);
  const link = d.connected && !d.unmounted ? d.connected.link : null;
  const port = link?.kind === "serial" ? link.port : null;
  const failure = failureFor(status, d.connected ? linkHandle(d.connected.link) : null);

  useEffect(() => {
    if (!id) return;
    let gone = false;
    api.gearBlackbox(id).then(
      (l) => !gone && setList(l),
      (e) => !gone && toast(errText(e), true),
    );
    return () => {
      gone = true;
    };
  }, [id, devices, status?.jobs?.length]);

  const sel = list?.find((e) => e.pull.id === picked) || list?.[0] || null;

  const run = async (what: () => Promise<string>) => {
    setBusy(true);
    try {
      toast(await what());
    } catch (e) {
      toast(errText(e), true);
    } finally {
      setBusy(false);
    }
  };
  const pull = () =>
    run(async () => {
      const j = await api.gearBlackboxPull(port);
      if (j.result.pull) setPicked(j.result.pull.id);
      return pullText(j.result);
    });
  const erase = async () => {
    const ok = await ask("Erase the blackbox flash?", "This deletes the logs on the FC. QuadCam erases only when it has stored a pull of exactly what the flash holds.", { ok: "Erase" });
    if (ok !== true) return;
    run(async () => {
      await api.gearBlackboxErase(port);
      return "Flash erased. Safe to unplug.";
    });
  };

  if (!id) return <p className={styles.empty}>QuadCam pulls the blackbox once it knows this FC's id.</p>;
  return (
    <div className={styles.segment}>
      <div className={styles.bar}>
        {port && (
          <>
            <Button icon="import" onClick={pull} disabled={busy || !!job}>
              Pull blackbox
            </Button>
            <Button icon="trash-bin-trash" onClick={erase} disabled={busy || !!job || !list?.some((e) => !e.pull.erased)}>
              Erase flash
            </Button>
          </>
        )}
        {job && (
          <span className={styles.job} role="status">
            {job.step}: {jobText(job)}
            <Button size="sm" variant="ghost" onClick={() => api.gearStop(job.handle).catch((e) => toast(errText(e), true))} disabled={job.stopping}>
              Stop
            </Button>
          </span>
        )}
      </div>
      {failure && failure.step === "Blackbox" && (
        <Banner kind="warning" icon="danger-triangle">
          {failure.step} failed: {failure.message}
        </Banner>
      )}
      {list && !list.length && <p className={styles.empty}>No blackbox pulled yet.</p>}
      {list && list.length > 0 && (
        <table className={styles.table} aria-label="Blackbox pulls">
          <thead>
            <tr>
              <th scope="col">Pulled</th>
              <th scope="col">Aircraft</th>
              <th scope="col" className={styles.num}>
                Logs
              </th>
              <th scope="col" className={styles.num}>
                Size
              </th>
              <th scope="col">Flash</th>
            </tr>
          </thead>
          <tbody>
            {list.map((e) => (
              <tr key={e.pull.id} className={e.pull.id === sel?.pull.id ? styles.selected : undefined}>
                <td>
                  <button type="button" className={styles.pick} aria-current={e.pull.id === sel?.pull.id} onClick={() => setPicked(e.pull.id)}>
                    {fmtWhen(e.pull.pulled_at)}
                  </button>
                </td>
                <td>{e.pull.aircraft || "None"}</td>
                <td className={styles.num}>{e.pull.logs.length}</td>
                <td className={styles.num}>{fmtBytes(e.pull.used)}</td>
                <td>{e.pull.erased ? "Erased" : "Kept"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {sel && <PullDetail key={sel.pull.id} e={sel} />}
    </div>
  );
}

/** One pull: its logs, and the flights they were paired with. */
function PullDetail({ e }: { e: BlackboxEntry }) {
  const flightOf = new Map(e.flights.links.map((l) => [l.log, l]));
  return (
    <section className={styles.detail} aria-label={`Blackbox pulled ${fmtWhen(e.pull.pulled_at)}`}>
      {e.pull.erase_note && <p className={styles.note}>{e.pull.erase_note}</p>}
      <table className={styles.table} aria-label="Logs">
        <thead>
          <tr>
            <th scope="col" className={styles.num}>
              Log
            </th>
            <th scope="col">Firmware</th>
            <th scope="col" className={styles.num}>
              Size
            </th>
            <th scope="col">Flight</th>
          </tr>
        </thead>
        <tbody>
          {e.pull.logs.map((l) => {
            const f = flightOf.get(l.index);
            return (
              <tr key={l.index}>
                <td className={styles.num}>{l.index}</td>
                <td>{l.firmware || "Unknown"}</td>
                <td className={styles.num}>{fmtBytes(l.size)}</td>
                <td>{f ? `${f.flight}${f.fits === false ? " (size does not fit)" : ""}` : "None"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className={styles.note}>{e.flights.note}</p>
    </section>
  );
}
