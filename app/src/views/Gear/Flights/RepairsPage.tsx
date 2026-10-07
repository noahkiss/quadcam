import { useStore } from "../../../store";
import { ask } from "../../../store";
import { libClip } from "../../../store/library";
import { api, errText } from "../../../ipc/api";
import type { Crash } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { Checkbox } from "../../../components/Field";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { mmss } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import g from "../Gear.module.css";
import styles from "./Flights.module.css";

/** Gear > Repairs: every crash, by aircraft, with what broke and the parts used. */
export function RepairsPage() {
  const { data, error } = useGearData(() => api.gearCrashes());
  const groups = new Map<string, Crash[]>();
  for (const c of data || []) {
    const k = c.aircraft || "No aircraft";
    groups.set(k, [...(groups.get(k) || []), c]);
  }
  return (
    <>
      <h2 className={g.title}>Repairs</h2>
      {error && <p className={styles.muted}>{error}</p>}
      {data && data.length === 0 && <p className={g.empty}>No crashes logged. Log one from a clip's Flight tab.</p>}
      {[...groups].map(([aircraft, list]) => (
        <section key={aircraft} className={styles.group} aria-label={aircraft}>
          <h3>{aircraft}</h3>
          <CrashTable list={list} />
        </section>
      ))}
    </>
  );
}

export function CrashTable({ list, showClip = true }: { list: Crash[]; showClip?: boolean }) {
  const s = useStore();
  const openDetail = useStore((x) => x.openDetail);
  const save = async (c: Crash, repaired: boolean) => {
    try {
      await api.gearCrashSave({ id: c.id, repaired });
    } catch (e) {
      toast(errText(e), true);
    }
  };
  const del = async (c: Crash) => {
    const ok = await ask("Delete this crash?", c.broke || "", { ok: "Delete", danger: true });
    if (ok !== true) return;
    try {
      await api.gearCrashDelete(c.id);
    } catch (e) {
      toast(errText(e), true);
    }
  };
  return (
    <div className={styles.tableWrap}>
      <table className={styles.table} aria-label="Crashes">
        <thead>
          <tr>
            <th scope="col">Day</th>
            {showClip && <th scope="col">Clip</th>}
            <th scope="col">At</th>
            <th scope="col">Broke</th>
            <th scope="col">Parts</th>
            <th scope="col">Repaired</th>
            <th scope="col">
              <span className="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {list.map((c) => {
            const clip = libClip(s, c.clip);
            return (
              <tr key={c.id}>
                <td>{fmtDay(c.day)}</td>
                {showClip && (
                  <td>
                    {c.clip ? (
                      <button type="button" className={styles.rowButton} onClick={() => openDetail(c.clip!, c.time_s ?? null, "flight")}>
                        {clip?.name || c.clip}
                      </button>
                    ) : (
                      "–"
                    )}
                  </td>
                )}
                <td>{c.time_s != null ? mmss(c.time_s) : "–"}</td>
                <td>{c.broke || "–"}</td>
                <td>{c.parts?.length ? c.parts.join(", ") : "–"}</td>
                <td>
                  <Checkbox label={<span className="visually-hidden">Repaired</span>} checked={!!c.repaired} onChange={(e) => save(c, e.target.checked)} />
                </td>
                <td>
                  <Button size="sm" variant="danger-ghost" onClick={() => del(c)}>
                    Delete…
                  </Button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
