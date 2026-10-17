// Gear > Voices > Publish (docs/gear.md, "Publish a pack"): prepares a pack rendered here for
// the QuadCam voices repo, then shows the review: every line with a play button and what the
// speech check heard, accepted lines first, and a spot check to listen to. Approve is the
// person's click (a GUI-only command); nothing is published before it.
import { useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Input } from "../../../components/Field";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { FactList } from "../../../components/dash/FactList";
import { SectionCard } from "../../../components/dash/SectionCard";
import { Stat, StatRow } from "../../../components/dash/StatRow";
import { api, errText, openLink } from "../../../ipc/api";
import type { PublishApproveReport, PublishReview, ReviewLine, VoicePack } from "../../../ipc/types";
import { fmtBytes } from "../../../lib/format";
import { playFile } from "./play";
import styles from "./Voice.module.css";
import { PathText } from "../../../components/PathText";

interface Form {
  id: string;
  version: string;
  voiceName: string;
  license: string;
  attribution: string;
}

const opt = (s: string) => (s.trim() ? s.trim() : null);

function LineTable({ label, rows }: { label: string; rows: ReviewLine[] }) {
  const cols: Column<ReviewLine>[] = [
    {
      key: "play",
      header: <span className={styles.hidden}>Play</span>,
      width: "3rem",
      cell: (l) => <Button size="sm" variant="ghost" icon="play" aria-label={`Play ${l.text}`} onClick={() => playFile(l.file)} />,
    },
    {
      key: "line",
      header: "Line",
      rowHeader: true,
      cell: (l) => (
        <>
          {l.text}
          <span className={styles.sub}>
            <PathText path={l.path} />
          </span>
        </>
      ),
    },
    { key: "heard", header: "Heard", cell: (l) => l.heard || <span className={styles.muted}>Nothing</span> },
    {
      key: "result",
      header: "Result",
      cell: (l) => (
        <span className={styles.cell}>
          {l.ok ? <Badge tone="ok">Heard right</Badge> : <Badge tone="warn">Accepted</Badge>}
          {l.spot && <Badge tone="accent">Spot check</Badge>}
          {l.preview && <Badge>Preview</Badge>}
        </span>
      ),
    },
  ];
  return <DataTable label={label} columns={cols} rows={rows} rowKey={(l) => l.path} compact />;
}

function Done({ r }: { r: PublishApproveReport }) {
  const p = r.published;
  if (p)
    return (
      <Banner kind="info" icon="check-circle">
        Released {r.id} v{r.version}.{" "}
        <button type="button" className={styles.link} onClick={() => void openLink(p.release_url)}>
          Open the release
        </button>{" "}
        ·{" "}
        <button type="button" className={styles.link} onClick={() => void openLink(p.pr_url)}>
          Open the pull request
        </button>
        . Merging the pull request publishes the pack.
      </Banner>
    );
  return (
    <>
      <Banner kind="warning" icon="danger-triangle">
        {r.notes.join(" ")}
      </Banner>
      <pre className={`${styles.commands} selectable`} aria-label="Commands to publish the pack">
        {r.commands.join("\n")}
      </pre>
    </>
  );
}

export function Publish({ pack, onClose }: { pack: VoicePack; onClose: () => void }) {
  const [form, setForm] = useState<Form>({ id: "", version: "1", voiceName: "", license: "", attribution: "" });
  const [review, setReview] = useState<PublishReview | null>(null);
  const [done, setDone] = useState<PublishApproveReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const set = (k: keyof Form) => (e: { target: { value: string } }) => {
    setForm({ ...form, [k]: e.target.value });
    setReview(null);
    setDone(null);
  };
  const prepare = async () => {
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      setReview(
        await api.gearVoicePublish({
          pack: pack.id,
          id: opt(form.id),
          version: opt(form.version),
          voice_name: opt(form.voiceName),
          license: opt(form.license),
          attribution: opt(form.attribution),
          repo: null,
        }),
      );
    } catch (e) {
      setReview(null);
      setError(errText(e));
    }
    setBusy(false);
  };
  const approve = async () => {
    if (!review) return;
    setBusy(true);
    setError(null);
    try {
      setDone(await api.gearVoicePublishApprove({ digest: review.digest, id: review.entry.id }));
    } catch (e) {
      setError(errText(e));
    }
    setBusy(false);
  };
  const e = review?.entry;
  const spot = review?.lines.filter((l) => l.spot) ?? [];
  return (
    <SectionCard
      title={`Publish: ${pack.voice}`}
      label="Publish"
      actions={
        <Button size="sm" variant="ghost" onClick={onClose}>
          Close
        </Button>
      }
    >
      <div className={styles.bar} role="group" aria-label="Pack details">
        <label className={styles.field}>
          Id
          <Input aria-label="Id" placeholder={`${pack.lang}-voice`} value={form.id} onChange={set("id")} />
        </label>
        <label className={styles.field}>
          Version
          <Input aria-label="Version" value={form.version} onChange={set("version")} />
        </label>
        <label className={styles.field}>
          Voice name
          <Input aria-label="Voice name" placeholder={pack.voice} value={form.voiceName} onChange={set("voiceName")} />
        </label>
        <label className={styles.field}>
          License
          <Input aria-label="License" placeholder="By provider" value={form.license} onChange={set("license")} />
        </label>
        <label className={styles.field}>
          Attribution
          <Input aria-label="Attribution" placeholder="By provider" value={form.attribution} onChange={set("attribution")} />
        </label>
        <Button size="sm" disabled={busy} onClick={prepare}>
          Prepare
        </Button>
      </div>
      {error && (
        <Banner kind="error" icon="danger-triangle">
          {error}
        </Banner>
      )}
      {review && e && (
        <>
          <StatRow label="Publish summary">
            <Stat label="Lines" value={e.lines.toLocaleString("en-US")} />
            <Stat label="Accepted" value={review.accepted.toLocaleString("en-US")} tone={review.accepted > 0 ? "warn" : "ok"} sub="Heard otherwise, kept" />
            <Stat label="Download" value={fmtBytes(e.zip_bytes ?? 0)} />
            <Stat label="On the card" value={fmtBytes(e.bytes)} />
          </StatRow>
          <FactList
            label="Release"
            facts={[
              { label: "Id", value: `${e.id} v${e.version}`, selectable: true },
              { label: "License", value: e.license },
              { label: "Attribution", value: e.attribution || "None" },
              { label: "Repo", value: review.repo, selectable: true },
              { label: "Preview lines", value: (e.preview ?? []).map((p) => p.text).join(", ") },
              { label: "Digest", value: review.digest, selectable: true },
            ]}
          />
          {review.notes.length > 0 && <p className={styles.status}>{review.notes.join(" ")}</p>}
          <h3 className={styles.subtitle}>Spot check</h3>
          <p className={styles.muted}>Listen to these {spot.length} lines, picked at random, before you approve.</p>
          <LineTable label="Spot check" rows={spot} />
          <h3 className={styles.subtitle}>Every line</h3>
          <LineTable label="Every line" rows={review.lines} />
          {done ? (
            <Done r={done} />
          ) : (
            <div className={styles.bar}>
              <Button size="sm" variant="primary" disabled={busy} onClick={approve}>
                Approve and publish
              </Button>
              <span className={styles.muted}>{review.gh.ready ? `Creates the release and a pull request on ${review.repo}.` : "gh is not ready: Approve shows the commands to run."}</span>
            </div>
          )}
        </>
      )}
    </SectionCard>
  );
}
