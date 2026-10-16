import { useStore } from "../../../store";
import { ask } from "../../../store";
import { libClip } from "../../../store/library";
import { api, errText } from "../../../ipc/api";
import type { Crash } from "../../../ipc/types";
import { Button } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { Disclosure } from "../../../components/dash/Disclosure";
import { EmptyState } from "../../../components/dash/EmptyState";
import { MoreMenu } from "../../../components/dash/MoreMenu";
import { PageHeader } from "../../../components/dash/PageHeader";
import { SectionCard } from "../../../components/dash/SectionCard";
import { SummaryLine } from "../../../components/dash/SummaryLine";
import { stripStyle } from "../../Library/ClipCard";
import { toast } from "../../../components/toastStore";
import { fmtDay } from "../../../lib/format";
import { mmss } from "../../../lib/flights";
import { useGearData } from "./useGearData";
import styles from "./Flights.module.css";

const NO_AIRCRAFT = "No aircraft";

/** Gear > Repairs: the crashes by aircraft, open ones first with Mark repaired, the repaired
 *  ones folded under each aircraft; what broke, the parts, and the clip with its frame. */
export function RepairsPage() {
  const s = useStore();
  const openDetail = useStore((x) => x.openDetail);
  const { data, error } = useGearData(() => api.gearCrashes());
  const list = data ?? [];
  const open = list.filter((c) => !c.repaired).length;
  const last = list.reduce<Crash | null>((a, c) => (!a || c.day > a.day ? c : a), null);
  // Aircraft by name, "No aircraft" last; each aircraft's crashes newest first.
  const groups = [...new Set(list.map((c) => c.aircraft || NO_AIRCRAFT))].sort((a, b) => (a === NO_AIRCRAFT ? 1 : b === NO_AIRCRAFT ? -1 : a.localeCompare(b)));
  const of = (g: string) => list.filter((c) => (c.aircraft || NO_AIRCRAFT) === g).sort((a, b) => b.day.localeCompare(a.day));
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
  const columns: Column<Crash>[] = [
    {
      key: "clip",
      header: "Clip",
      rowHeader: true,
      cell: (c) => {
        const clip = c.clip ? libClip(s, c.clip) : null;
        return c.clip ? (
          <button type="button" className={styles.clipCell} onClick={() => openDetail(c.clip!, c.time_s ?? null, "flight")}>
            {clip && <span className={styles.thumb} style={stripStyle(clip, clip.duration && c.time_s != null ? Math.min(9, Math.floor((c.time_s / clip.duration) * 10)) : 0)} aria-hidden="true" />}
            <span className={styles.rowButton}>{clip?.name || c.clip}</span>
          </button>
        ) : (
          <span className={styles.muted}>No clip</span>
        );
      },
    },
    { key: "day", header: "Day", cell: (c) => `${fmtDay(c.day)}${c.time_s != null ? ` · ${mmss(c.time_s)}` : ""}` },
    { key: "broke", header: "Broke", cell: (c) => c.broke || "–" },
    { key: "parts", header: "Parts", cell: (c) => (c.parts?.length ? c.parts.join(", ") : "–") },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "11rem",
      cell: (c) => (
        <div className={styles.actions}>
          {!c.repaired && (
            <Button size="sm" onClick={() => save(c, true)}>
              Mark repaired
            </Button>
          )}
          <MoreMenu
            label={`More actions for the crash on ${fmtDay(c.day)}`}
            items={[...(c.repaired ? [{ label: "Mark open", run: () => void save(c, false) }] : []), { label: "Delete…", icon: "trash-bin-trash", danger: true, run: () => void del(c) }]}
          />
        </div>
      ),
    },
  ];
  return (
    <>
      <PageHeader
        id="gear-title"
        title="Repairs"
        subtitle={
          data &&
          list.length > 0 && (
            <SummaryLine
              parts={[open > 0 ? { text: `${open} open`, tone: "warn" } : "None open", `${list.length - open} repaired`, last && `Last crash ${fmtDay(last.day)} (${last.aircraft || NO_AIRCRAFT})`]}
            />
          )
        }
      />
      {error && <p className={styles.muted}>{error}</p>}
      {data && !list.length && <EmptyState icon="danger-triangle">No crashes logged. Log one from a clip's Flight tab.</EmptyState>}
      {groups.map((g) => {
        const all = of(g);
        const openOnes = all.filter((c) => !c.repaired);
        const done = all.filter((c) => c.repaired);
        return (
          <SectionCard key={g} title={g}>
            {openOnes.length > 0 ? <DataTable label={`Crashes on ${g}`} columns={columns} rows={openOnes} rowKey={(c) => c.id} /> : <p className={styles.muted}>Nothing open.</p>}
            {done.length > 0 && (
              <Disclosure summary={`${done.length} repaired`}>
                <DataTable label={`Repaired crashes on ${g}`} columns={columns} rows={done} rowKey={(c) => c.id} />
              </Disclosure>
            )}
          </SectionCard>
        );
      })}
    </>
  );
}
