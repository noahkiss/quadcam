// Gear > Voices > Speech check (design 7.4): what the on-device speech recognizer heard in each
// line of one pack, against the line's text. A line heard wrong in a studio pack is out of the
// pack as a re-take; Accept keeps what was heard and puts the line back. Calls no provider.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { api } from "../../../ipc/api";
import type { HeardLine, VoiceCheckReport, VoicePack, VoiceView } from "../../../ipc/types";
import type { UseVoice } from "./useVoice";
import styles from "./Voice.module.css";

/** A line the check heard wrong, unless the person kept it. */
export const heardWrong = (h: HeardLine) => !h.ok && !h.accepted;

/** The pack's speech check in one badge for the library row; nothing before a check. */
export function SpeechBadge({ pack }: { pack: VoicePack }) {
  const heard = pack.heard ?? [];
  if (heard.length === 0) return <span className={styles.muted}>Not checked</span>;
  const wrong = heard.filter(heardWrong).length;
  return wrong > 0 ? <Badge tone="warn">{wrong === 1 ? "1 heard wrong" : `${wrong} heard wrong`}</Badge> : <Badge tone="ok">Heard right</Badge>;
}

const statusText = (r: VoiceCheckReport) => {
  const head = !r.available ? `Not checked: ${r.reason ?? ""}` : r.lines.length > 0 && r.lines.every((l) => l.accepted) ? `${r.lines.length} accepted.` : `${r.lines.length} lines heard, ${r.mismatches} heard wrong.`;
  return [head, ...r.notes].join(" ");
};

const result = (h: HeardLine) => (h.ok ? <Badge tone="ok">Heard right</Badge> : h.accepted ? <Badge>Accepted</Badge> : <Badge tone="warn">Heard wrong</Badge>);

export function SpeechCheck({ v, view, pack, onClose }: { v: UseVoice; view: VoiceView; pack: VoicePack; onClose: () => void }) {
  const [report, setReport] = useState<VoiceCheckReport | null>(null);
  const [busy, setBusy] = useState(false);
  const speech = view.speech;
  const heard = pack.heard ?? [];
  const wrong = heard.filter(heardWrong).length;
  const run = async (p: { accept?: string[]; install?: boolean }) => {
    setBusy(true);
    const r = await v.act(() => api.gearVoiceCheck({ pack: pack.id, lines: [], accept: p.accept ?? [], install: !!p.install }));
    setBusy(false);
    if (r) setReport(r);
  };
  const cols: Column<HeardLine>[] = [
    {
      key: "line",
      header: "Line",
      rowHeader: true,
      cell: (h) => (
        <>
          {h.text}
          <span className={styles.sub}>{h.path}</span>
        </>
      ),
    },
    { key: "heard", header: "Heard", cell: (h) => h.heard || <span className={styles.muted}>Nothing</span> },
    { key: "result", header: "Result", cell: result },
    {
      key: "actions",
      header: <span className={styles.hidden}>Actions</span>,
      cell: (h) =>
        heardWrong(h) ? (
          <Button size="sm" disabled={busy} aria-label={`Accept what was heard for ${h.text}`} onClick={() => run({ accept: [h.path] })}>
            Accept
          </Button>
        ) : null,
    },
  ];
  return (
    <SectionCard
      title={`Speech check: ${pack.voice}`}
      label="Speech check"
      actions={
        <>
          <Button size="sm" variant="primary" disabled={busy || !speech.available} onClick={() => run({})}>
            {heard.length ? "Check again" : "Check speech"}
          </Button>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Close
          </Button>
        </>
      }
    >
      {!speech.available && (
        <Banner
          kind="warning"
          icon="danger-triangle"
          action={
            speech.assets === "supported" ? (
              <Button size="sm" disabled={busy} onClick={() => run({ install: true })}>
                Install speech assets
              </Button>
            ) : undefined
          }
        >
          The speech check is unavailable: {speech.reason ?? "no reason given."}
        </Banner>
      )}
      {heard.length > 0 && (
        <StatRow label="Speech check summary">
          <Stat label="Lines heard" value={heard.length.toLocaleString("en-US")} />
          <Stat label="Heard wrong" value={wrong.toLocaleString("en-US")} tone={wrong > 0 ? "warn" : "ok"} />
          <Stat label="Accepted" value={heard.filter((h) => h.accepted).length.toLocaleString("en-US")} />
        </StatRow>
      )}
      <DataTable
        label={`Speech check of ${pack.voice}`}
        columns={cols}
        rows={heard}
        rowKey={(h) => h.path}
        compact
        empty={<p className={styles.muted}>Not checked yet.</p>}
      />
      {report && (
        <p className={styles.status} role="status">
          {statusText(report)}
        </p>
      )}
    </SectionCard>
  );
}
