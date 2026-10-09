import type { ReactNode } from "react";
import { useStore, type State } from "../../store";
import { screenOf } from "../../store/library";
import { clipsOf, unfinished } from "../../store/session";
import { TASK_LABEL } from "../../store/tasks";
import { Icon, type IconName, type IconTint } from "../../components/Icon";
import { fmtBytes, fmtDay, base, SOURCE_LABEL } from "../../lib/format";
import { sameFilter, type Filter } from "../../lib/library";
import { importFrom, openFolderAction, openImport, setLogDir } from "../../actions/session";
import { deviceName, KIND_ICON, KIND_LABEL, STATE_LABEL } from "../../lib/gear";
import { deviceRefs, pluggedIn, useNow } from "../Gear/refs";
import { GEAR_PAGES } from "../Gear/pages";
import styles from "./Sidebar.module.css";

interface ItemProps {
  icon: IconName;
  tint?: IconTint;
  label: string;
  /** Short text after the label, such as a card's video system. */
  detail?: string | null;
  count?: number | null;
  badge?: string | null;
  current?: boolean;
  muted?: boolean;
  onClick?: () => void;
}

function Item({ icon, tint, label, detail, count, badge, current, muted, onClick }: ItemProps) {
  return (
    <li>
      <button type="button" className={[styles.item, muted && styles.muted].filter(Boolean).join(" ")} aria-current={current ? "true" : undefined} onClick={onClick} disabled={!onClick}>
        <Icon name={icon} tint={tint} />
        <span className={styles.label}>{label}</span>
        {detail && <span className={styles.detail}>{detail}</span>}
        {badge ? <span className={styles.badge}>{badge}</span> : count != null ? <span className={styles.count}>{count}</span> : null}
      </button>
    </li>
  );
}

function Group({ title, children, expanded, onToggle }: { title: string; children: ReactNode; expanded?: boolean; onToggle?: () => void }) {
  return (
    <section className={styles.group}>
      {onToggle ? (
        <h2>
          <button type="button" className={styles.disclose} aria-expanded={expanded} onClick={onToggle}>
            <Icon name={expanded ? "chev-down" : "chev-right"} size={12} />
            {title}
          </button>
        </h2>
      ) : (
        <h2>{title}</h2>
      )}
      {expanded !== false && <ul>{children}</ul>}
    </section>
  );
}

/** The source list: library groups, import sources, days, aircraft, places, smart groups. */
export function Sidebar() {
  const s = useStore();
  const L = s.lib;
  const clips = L?.clips || [];
  const detail = screenOf(s) !== "library" && screenOf(s) !== "first-run";
  const cur = (f: Filter) => !detail && sameFilter(s.filter, f);
  const go = (f: Filter) => () => s.setFilter(f);
  const lastN = clips.filter((c) => c.last_import).length;
  const cards = s.volumes.filter((v) => v.is_card);
  const days = L?.groups?.days || [];
  const shown = days.slice(0, 6);
  const ac = L?.groups?.aircraft || [];
  const pl = L?.groups?.places || [];
  const n = (fn: (c: (typeof clips)[number]) => boolean) => clips.filter(fn).length;
  return (
    <nav aria-label="Library" className={styles.side}>
      <div className={styles.groups}>
        <Group title="Library">
          <Item icon="film" tint="blue" label="All clips" count={clips.length} current={cur({ group: "all" })} onClick={go({ group: "all" })} />
          {lastN > 0 && <Item icon="sparkle" tint="pink" label="Last import" count={lastN} current={cur({ group: "last_import" })} onClick={go({ group: "last_import" })} />}
        </Group>
        <Group title="Import from">
          {cards.map((v) => {
            const st = s.cards[v.mount];
            return <Item key={v.mount} icon="sd-card" tint="green" label={v.info.volume_name || base(v.mount)} detail={v.source ? SOURCE_LABEL[v.source] : null} badge={st?.new ? `${st.new} new` : null} count={st && !st.new ? 0 : null} onClick={() => importFrom(v.mount)} />;
          })}
          {!cards.length && <Item icon="sd-card" label="No card inserted" muted />}
          {unfinished(s.session) && <Item icon="history" label="Unfinished import" count={clipsOf(s.session).length} onClick={() => openImport()} />}
          <Item icon="folder-open" label="Folder…" onClick={openFolderAction} />
          {s.volumes
            .filter((v) => v.is_radio)
            .map((v) => (
              <Item key={v.mount} icon="radio" label={v.info.volume_name || base(v.mount)} onClick={() => setLogDir(v.mount)} />
            ))}
        </Group>
        {days.length > 0 && (
          <Group title="Flying days">
            {shown.map(([d, k]) => (
              <Item key={d} icon="calendar" label={fmtDay(d)} count={k} current={cur({ group: "all", day: d })} onClick={go({ group: "all", day: d })} />
            ))}
            {days.length > 6 && (
              <Item
                icon="calendar"
                label="Older"
                muted
                count={days.slice(6).reduce((a, [, k]) => a + k, 0)}
                current={cur({ group: "all", before: shown.at(-1)![0] })}
                onClick={go({ group: "all", before: shown.at(-1)![0] })}
              />
            )}
          </Group>
        )}
        <Group title="Aircraft">
          {ac.length ? ac.map(([a, k]) => <Item key={a} icon="quad" tint="pink" label={a} count={k} current={cur({ group: "all", aircraft: a })} onClick={go({ group: "all", aircraft: a })} />) : <Item icon="add" tint="blue" label="Add aircraft" muted onClick={() => s.openSettings("aircraft")} />}
        </Group>
        <Group title="Places">
          {pl.length ? pl.map(([p, k]) => <Item key={p} icon="map-point" tint="green" label={p} count={k} current={cur({ group: "all", place: p })} onClick={go({ group: "all", place: p })} />) : <Item icon="add" tint="blue" label="Add place" muted onClick={() => s.openSettings("places")} />}
        </Group>
        <GearGroup s={s} />
        {clips.length > 0 && (
          <Group title="Smart groups">
            <Item icon="flip" tint="mauve" label="Has moments" count={n((c) => c.moments.length > 0)} current={cur({ group: "moments" })} onClick={go({ group: "moments" })} />
            <Item icon="flag" tint="green" label="Picks" count={n((c) => c.flag === "pick")} current={cur({ group: "picks" })} onClick={go({ group: "picks" })} />
            <Item icon="close" tint="red" label="Rejected" count={n((c) => c.flag === "reject")} current={cur({ group: "rejected" })} onClick={go({ group: "rejected" })} />
            <Item icon="photos" tint="yellow" label="Not in Photos" count={n((c) => !c.in_photos)} current={cur({ group: "not_in_photos" })} onClick={go({ group: "not_in_photos" })} />
          </Group>
        )}
      </div>
      <SideFoot s={s} />
    </nav>
  );
}

/** Gear: what is plugged in, the saved devices, and the pages later packages add. */
function GearGroup({ s }: { s: State }) {
  const now = useNow(s.unmounted.length > 0);
  const refs = deviceRefs(s, now);
  const plugged = pluggedIn(refs);
  const page = s.gearPage;
  const at = (p: NonNullable<State["gearPage"]>) => !!page && JSON.stringify(page) === JSON.stringify(p);
  return (
    <Group title="Gear" expanded={s.gearExpanded} onToggle={() => s.setGearExpanded(!s.gearExpanded)}>
      <Item icon="signal" tint="teal" label="Connected" count={plugged.length} current={at({ page: "connected" })} onClick={() => s.openGear({ page: "connected" })} />
      {plugged.map((r) => (
        <Item
          key={r.key}
          icon={KIND_ICON[r.kind]}
          tint={r.state === "attention" || r.state === "inserted" ? "yellow" : r.state === "safe" ? "green" : undefined}
          label={r.device ? deviceName(r.device) : KIND_LABEL[r.kind]}
          detail={r.state && r.state !== "connected" ? STATE_LABEL[r.state] : null}
          current={at({ page: "device", key: r.key })}
          onClick={() => s.openGear({ page: "device", key: r.key })}
        />
      ))}
      <Item icon="list" label="Devices" count={s.devices.length} current={at({ page: "devices" })} onClick={() => s.openGear({ page: "devices" })} />
      {GEAR_PAGES.filter((p) => !p.preview || s.values[p.preview] === true).map((p) => (
        <Item key={p.id} icon={p.icon} label={p.label} count={p.count ? p.count(s) || null : undefined} current={at({ page: "slot", id: p.id })} onClick={() => s.openGear({ page: "slot", id: p.id })} />
      ))}
    </Group>
  );
}

function SideFoot({ s }: { s: State }) {
  const lines: ReactNode[] = [];
  for (const [task, t] of Object.entries(s.tasks)) {
    if (t.done >= t.total) continue;
    lines.push(
      <div key={task} className={styles.task}>
        <div className={styles.line}>
          <Icon name="refresh-moments" tint="blue" size={14} />
          <span>{TASK_LABEL[task] || task}</span>
          <span className={styles.num}>
            {Math.min(t.done + 1, t.total)} of {t.total}
          </span>
        </div>
        <span className={styles.bar}>
          <span style={{ width: `${(t.done / Math.max(1, t.total)) * 100}%` }} />
        </span>
      </div>,
    );
  }
  const sess = s.session;
  if (sess?.card && sess.analysed) {
    const mounted = s.volumes.find((v) => v.is_card && v.info.volume_uuid === sess.card!.volume_uuid);
    const copied = sess.clips.length && sess.clips.every((c) => c.staged && !c.stage_error);
    if (mounted && copied)
      lines.push(
        <div key="safe" className={styles.line}>
          <Icon name="check-circle" tint="green" size={14} />
          <span>{mounted.info.volume_name || "Card"} copied</span>
          <span className={styles.ok}>Safe to remove</span>
        </div>,
      );
  }
  if (sess?.card && s.cardRelease?.released && !s.volumes.some((v) => v.is_card && v.info.volume_uuid === sess.card!.volume_uuid))
    lines.push(
      <div key="unmounted" className={styles.line}>
        <Icon name="check-circle" tint="green" size={14} />
        <span>{sess.card.volume_name || "Card"} unmounted</span>
        <span className={styles.ok}>Safe to remove</span>
      </div>,
    );
  const card = s.volumes.find((v) => v.is_card);
  const st = card && s.cards[card.mount];
  return (
    <div className={styles.foot}>
      {lines}
      {lines.length > 0 && <hr />}
      <div className={styles.line}>
        <span>Library</span>
        <span className={styles.num}>{fmtBytes(s.lib?.totals?.bytes || 0)}</span>
      </div>
      {st?.free != null && st.size > 0 && (
        <div className={styles.line}>
          <span>Card free</span>
          <span className={styles.num}>
            {fmtBytes(st.free)} of {fmtBytes(st.size)}
          </span>
        </div>
      )}
    </div>
  );
}
