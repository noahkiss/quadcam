// Gear > Voices > Speech check (design 7.4): a sheet over the Voices page with what the
// on-device speech recognizer heard in each line of one pack, against the line's text. The lines
// that need review come first: heard wrong, re-takes and flags. Play plays the pack's take (a
// line heard wrong plays its held take). Accept keeps what was heard and puts the line back; Flag
// marks a line with a reason and a note. Re-take selected makes a studio pack's chosen lines
// again: it prices them with a dry run first and pays only after the person confirms.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { Checkbox } from "../../../components/Field";
import { SegmentedControl } from "../../../components/SegmentedControl";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { api } from "../../../ipc/api";
import type { HeardLine, RetakeReport, TakeFlag, VoiceCheckReport, VoicePack, VoiceView } from "../../../ipc/types";
import { FlagControl, reasonText } from "./Gallery";
import { playFile } from "./play";
import { costText } from "./Studio";
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

/** A pack the Voice studio rendered: its re-take lines can be made again. */
export const isStudio = (k: VoicePack) => k.local && k.provider === "elevenlabs";

/** One line of the sheet: what the check heard, and whether it is a re-take or flagged. */
export interface CheckRow {
  path: string;
  text: string;
  /** The heard line; null for a re-take the check never heard (its cut failed). */
  heard: HeardLine | null;
  /** Why the line is out of the pack. */
  retake: string | null;
  flag: TakeFlag | null;
}

/** The pack's lines for the sheet: every heard line and every re-take, the lines that need
 *  review first, each part in card-path order. */
export function checkRows(pack: VoicePack): CheckRow[] {
  const retakes = new Map((pack.retakes ?? []).map((r) => [r.path, r]));
  const flags = new Map((pack.flags ?? []).map((f) => [f.line, f]));
  const rows: CheckRow[] = (pack.heard ?? []).map((h) => ({ path: h.path, text: h.text, heard: h, retake: retakes.get(h.path)?.reason ?? null, flag: flags.get(h.path) ?? null }));
  for (const r of retakes.values()) if (!rows.some((x) => x.path === r.path)) rows.push({ path: r.path, text: r.text, heard: null, retake: r.reason, flag: flags.get(r.path) ?? null });
  const order = (r: CheckRow) => (needsReview(r) ? 0 : 1);
  return rows.sort((a, b) => order(a) - order(b) || a.path.localeCompare(b.path));
}

export const needsReview = (r: CheckRow) => (r.heard !== null && heardWrong(r.heard)) || r.retake !== null || r.flag !== null;
/** Accept applies to a line heard wrong. */
const acceptable = (r: CheckRow) => r.heard !== null && heardWrong(r.heard);
/** Re-take applies to a re-take or a flagged line of a studio pack. */
const retakable = (r: CheckRow) => r.retake !== null || r.flag !== null;

const lines = (n: number) => (n === 1 ? "1 line" : `${n.toLocaleString("en-US")} lines`);

const statusText = (r: VoiceCheckReport) => {
  const head = !r.available ? `Not checked: ${r.reason ?? ""}` : r.lines.length > 0 && r.lines.every((l) => l.accepted) ? `${r.lines.length} accepted.` : `${r.lines.length} lines heard, ${r.mismatches} heard wrong.`;
  return [head, ...r.notes].join(" ");
};

const retakeText = (r: RetakeReport) => {
  const made = (r.cleared ?? []).length;
  const head = made === r.lines.length ? `Made ${lines(made)} again.` : `Made ${made} of ${lines(r.lines.length)} again; the rest still need a re-take.`;
  return [head, ...(r.notes ?? [])].join(" ");
};

function Result({ r }: { r: CheckRow }) {
  if (r.heard && heardWrong(r.heard)) return <Badge tone="warn">Heard wrong</Badge>;
  if (r.retake) return <Badge tone="warn">Re-take</Badge>;
  if (r.flag) return <Badge tone="warn">Flagged</Badge>;
  if (r.heard?.accepted) return <Badge>Accepted</Badge>;
  return <Badge tone="ok">Heard right</Badge>;
}

type Filter = "review" | "all";

export function SpeechCheck({ v, view, pack, onClose, onFlagged }: { v: UseVoice; view: VoiceView; pack: VoicePack; onClose: () => void; onFlagged?: () => Promise<void> }) {
  const [report, setReport] = useState<VoiceCheckReport | null>(null);
  const [retook, setRetook] = useState<RetakeReport | null>(null);
  const [ask, setAsk] = useState<RetakeReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [picked, setPicked] = useState<string[]>([]);
  const [filter, setFilter] = useState<Filter | null>(null);
  const speech = view.speech;
  const studio = isStudio(pack);
  const all = checkRows(pack);
  const review = all.filter(needsReview);
  const showing: Filter = filter ?? (review.length > 0 ? "review" : "all");
  // Once lines need review, the sheet shows them until the person picks All, even after the
  // last one is accepted or made again.
  if (filter === null && review.length > 0) setFilter("review");
  const shown = showing === "review" ? review : all;
  const chosen = shown.filter((r) => picked.includes(r.path));
  const toAccept = chosen.filter(acceptable);
  const toRetake = studio ? chosen.filter(retakable) : [];

  const run = async <T,>(f: () => Promise<T>) => {
    setBusy(true);
    const r = await v.act(f);
    setBusy(false);
    return r;
  };
  const check = async (p: { accept?: string[]; install?: boolean }) => {
    setRetook(null);
    const r = await run(() => api.gearVoiceCheck({ pack: pack.id, lines: [], accept: p.accept ?? [], install: !!p.install }));
    if (r) setReport(r);
    if (p.accept) setPicked((x) => x.filter((path) => !p.accept!.includes(path)));
  };
  const price = async () => {
    setRetook(null);
    setReport(null);
    const r = await run(() => api.gearVoiceRetake({ sample: "", pack: pack.id, lines: toRetake.map((x) => x.path), dry_run: true, confirm: false, digest: null }));
    if (r) setAsk(r);
  };
  const pay = async (a: RetakeReport) => {
    setAsk(null);
    const r = await run(() => api.gearVoiceRetake({ sample: "", pack: pack.id, lines: a.lines, dry_run: false, confirm: true, digest: a.digest }));
    if (r) {
      setRetook(r);
      setPicked((x) => x.filter((path) => !a.lines.includes(path)));
      await onFlagged?.();
    }
  };
  const play = async (r: CheckRow) => {
    const f = await v.act(() => api.gearVoicePreview({ line: r.path, pack: pack.id, radio: null }));
    if (f) playFile(f);
  };
  const pick = (path: string, on: boolean) => setPicked((x) => (on ? [...x, path] : x.filter((p) => p !== path)));
  const allPicked = shown.length > 0 && shown.every((r) => picked.includes(r.path));

  const cols: Column<CheckRow>[] = [
    {
      key: "play",
      header: <span className={styles.hidden}>Play</span>,
      width: "2.5rem",
      cell: (r) => <Button size="sm" variant="ghost" icon="play" aria-label={`Play ${r.text}`} disabled={busy} onClick={() => void play(r)} />,
    },
    {
      key: "line",
      header: "Line",
      rowHeader: true,
      cell: (r) => (
        <>
          {r.text}
          <span className={styles.sub}>{r.path}</span>
        </>
      ),
    },
    {
      key: "heard",
      header: "Heard",
      cell: (r) =>
        r.heard ? (
          r.heard.heard || <span className={styles.muted}>Nothing</span>
        ) : (
          <span className={styles.muted}>{r.retake}</span>
        ),
    },
    {
      key: "result",
      header: "Result",
      cell: (r) => (
        <>
          <Result r={r} />
          {r.flag && <span className={styles.sub}>{reasonText(r.flag.reason)}{r.flag.note ? `: ${r.flag.note}` : ""}</span>}
        </>
      ),
    },
    {
      key: "actions",
      header: <span className={styles.hidden}>Actions</span>,
      cell: (r) => (
        <span className={styles.rowActions}>
          {acceptable(r) && (
            <Button size="sm" disabled={busy} aria-label={`Accept what was heard for ${r.text}`} onClick={() => void check({ accept: [r.path] })}>
              Accept
            </Button>
          )}
          <FlagControl
            text={r.text}
            flag={r.flag}
            disabled={busy}
            onSave={async (f) => {
              await run(() => api.gearVoiceFlag({ sample: "", pack: pack.id, line: r.path, reason: f?.reason ?? "", note: f?.note ?? "", clear: !f }));
              await onFlagged?.();
            }}
          />
        </span>
      ),
    },
  ];

  return (
    <Dialog
      open
      kind="sheet"
      title={`Speech check: ${pack.voice}`}
      onClose={onClose}
      className={styles.checkSheet}
      actions={
        <>
          <Button size="sm" disabled={busy || toAccept.length === 0} onClick={() => void check({ accept: toAccept.map((r) => r.path) })}>
            Accept selected
          </Button>
          {studio && (
            <Button size="sm" disabled={busy || toRetake.length === 0 || !!ask} onClick={() => void price()}>
              Re-take selected…
            </Button>
          )}
          <span className={styles.grow} />
          <Button type="submit" value="done" variant="primary">
            Done
          </Button>
        </>
      }
    >
      <div className={styles.checkBody}>
        <div className={styles.bar}>
          <SegmentedControl
            size="sm"
            label="Show"
            value={showing}
            onChange={setFilter}
            segments={[
              { value: "review", label: `Needs review (${review.length.toLocaleString("en-US")})` },
              { value: "all", label: `All (${all.length.toLocaleString("en-US")})` },
            ]}
          />
          <Checkbox label="Select all" checked={allPicked} disabled={shown.length === 0} onChange={(e) => setPicked(e.target.checked ? shown.map((r) => r.path) : [])} />
          <span className={styles.grow} />
          <Button size="sm" disabled={busy || !speech.available} onClick={() => void check({})}>
            {(pack.heard ?? []).length ? "Check again" : "Check speech"}
          </Button>
        </div>
        {!speech.available && (
          <Banner
            kind="warning"
            icon="danger-triangle"
            action={
              speech.assets === "supported" ? (
                <Button size="sm" disabled={busy} onClick={() => void check({ install: true })}>
                  Install speech assets
                </Button>
              ) : undefined
            }
          >
            The speech check is unavailable: {speech.reason ?? "no reason given."}
          </Banner>
        )}
        {v.error && (
          <Banner kind="error" icon="danger-triangle">
            {v.error}
          </Banner>
        )}
        {ask && (
          <Banner
            kind="warning"
            icon="danger-triangle"
            action={
              <>
                <Button size="sm" variant="danger" disabled={busy} onClick={() => void pay(ask)}>
                  {(ask.estimate?.chars ?? 0) > 0 ? "Re-take and pay" : "Re-take"}
                </Button>
                <Button size="sm" onClick={() => setAsk(null)}>
                  Cancel
                </Button>
              </>
            }
          >
            {(ask.estimate?.chars ?? 0) > 0 ? `This sends ${lines(ask.lines.length)} to ElevenLabs, which bills it. ${ask.estimate ? costText(ask.estimate) : ""}` : `${lines(ask.lines.length)}: every take comes from the cache, at no cost.`}
          </Banner>
        )}
        <DataTable
          label={`Speech check of ${pack.voice}`}
          columns={cols}
          rows={shown}
          rowKey={(r) => r.path}
          compact
          maxHeight="min(60vh, 640px)"
          selection={{ selected: (r) => picked.includes(r.path), onChange: (r, on) => pick(r.path, on), label: (r) => r.text }}
          empty={<p className={styles.muted}>{all.length === 0 ? "Not checked yet." : "No line needs review."}</p>}
        />
        {(report || retook) && (
          <p className={styles.status} role="status">
            {retook ? retakeText(retook) : statusText(report!)}
          </p>
        )}
      </div>
    </Dialog>
  );
}
