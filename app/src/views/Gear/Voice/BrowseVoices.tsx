// Browse voices…: a sheet over ElevenLabs' Voice Library (search, filters, pages), with a
// preview of each voice and Add to My Voices, which changes the account and so asks first.
// Add by ID looks one voice up and shows it the same way.
import { useCallback, useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Dialog } from "../../../components/Dialog";
import { SelectField, TextField } from "../../../components/Field";
import { Badge } from "../../../components/dash/Badge";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { api, errText } from "../../../ipc/api";
import type { LibraryPage, LibraryQuery, LibraryVoice } from "../../../ipc/types";
import { playPreview } from "./play";
import { LIST_HEIGHT } from "./useStudio";
import styles from "./Voice.module.css";

const GENDERS = ["male", "female", "neutral"];
const AGES = [
  ["young", "Young"],
  ["middle_aged", "Middle-aged"],
  ["old", "Old"],
];
const ACCENTS = ["american", "british", "australian", "canadian", "irish", "scottish", "indian", "south african", "new zealand"];
const LANGUAGES = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["nl", "Dutch"],
  ["pl", "Polish"],
  ["ja", "Japanese"],
];
const USES = [
  ["narrative_story", "Narration"],
  ["conversational", "Conversational"],
  ["characters_animation", "Characters"],
  ["social_media", "Social media"],
  ["entertainment_tv", "Entertainment"],
  ["advertisement", "Advertisement"],
  ["informative_educational", "Informative"],
];
const CATEGORIES = [
  ["professional", "Professional"],
  ["high_quality", "High quality"],
  ["famous", "Famous"],
];
const SORTS = [
  ["", "Newest"],
  ["trending", "Trending"],
  ["usage_character_count_1y", "Most used"],
  ["cloned_by_count", "Most added"],
];
const PAGE = 30;

const word = (s?: string | null) => (s ? s.replace(/_/g, " ") : "");
const languages = (v: LibraryVoice) => (v.languages?.length ? v.languages.join(", ") : (v.language ?? ""));

const blank: LibraryQuery = { search: "", gender: "", age: "", accent: "", language: "en", use_case: "", category: "", descriptives: [], sort: "", page: 0, page_size: PAGE };

function Options({ list, any }: { list: (string | string[])[]; any: string }) {
  return (
    <>
      <option value="">{any}</option>
      {list.map((o) => {
        const [v, t] = Array.isArray(o) ? o : [o, o.charAt(0).toUpperCase() + o.slice(1)];
        return (
          <option key={v} value={v}>
            {t}
          </option>
        );
      })}
    </>
  );
}

export function BrowseVoices({ onClose, onAdded }: { onClose: () => void; onAdded: () => Promise<void> }) {
  const [query, setQuery] = useState<LibraryQuery>(blank);
  const [byId, setById] = useState("");
  const [page, setPage] = useState<LibraryPage | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [asking, setAsking] = useState<LibraryVoice | null>(null);
  const [added, setAdded] = useState<string | null>(null);
  /** The table shows one voice looked up by id, not a page of the search. */
  const [single, setSingle] = useState(false);

  const load = useCallback(async (q: LibraryQuery, id = "") => {
    setBusy(true);
    setError(null);
    setSingle(!!id);
    try {
      setPage(await api.gearVoiceLibrary({ query: q, id }));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    let gone = false;
    api.gearVoiceLibrary({ query: blank, id: "" }).then(
      (p) => {
        if (gone) return;
        setPage(p);
        setBusy(false);
      },
      (e) => {
        if (gone) return;
        setError(errText(e));
        setBusy(false);
      },
    );
    return () => {
      gone = true;
    };
  }, []);

  const set = (k: keyof LibraryQuery, v: string) => {
    const q = { ...query, [k]: v, page: 0 };
    setQuery(q);
    void load(q);
  };
  const go = (p: number) => {
    const q = { ...query, page: p };
    setQuery(q);
    void load(q);
  };

  const add = async (v: LibraryVoice) => {
    setError(null);
    try {
      const r = await api.gearVoiceAdd(v.voice_id, v.name, true);
      setAsking(null);
      setAdded(r.already ? `${v.name} is in My Voices already.` : `Added ${v.name} to My Voices.`);
      setPage((p) => (p ? { ...p, voices: p.voices.map((x) => (x.voice_id === v.voice_id ? { ...x, added: true } : x)) } : p));
      await onAdded();
    } catch (e) {
      setAsking(null);
      setError(errText(e));
    }
  };

  const cols: Column<LibraryVoice>[] = [
    {
      key: "name",
      header: "Voice",
      rowHeader: true,
      cell: (v) => (
        <>
          {v.name}
          {v.description && <span className={styles.about}>{v.description}</span>}
        </>
      ),
    },
    { key: "gender", header: "Gender", cell: (v) => word(v.gender) },
    { key: "age", header: "Age", cell: (v) => word(v.age) },
    { key: "accent", header: "Accent", cell: (v) => word(v.accent) },
    { key: "language", header: "Language", cell: (v) => languages(v) },
    { key: "use", header: "Use", optional: true, cell: (v) => word(v.use_case) },
    {
      key: "notice",
      header: "Billing",
      cell: (v) => (
        <span className={styles.cell}>
          {v.rate != null && <Badge tone="warn">{v.rate}× credits</Badge>}
          {v.notice_days != null && <span className={styles.muted}>{v.notice_days} days' notice</span>}
        </span>
      ),
    },
    {
      key: "actions",
      header: "",
      cell: (v) => (
        <span className={styles.cell}>
          {v.preview_url && (
            <Button size="sm" variant="ghost" icon="play" aria-label={`Play a preview of ${v.name}`} onClick={() => void playPreview(v.preview_url!).catch((e) => setError(errText(e)))}>
              Preview
            </Button>
          )}
          {v.added ? (
            <Badge tone="ok">In My Voices</Badge>
          ) : (
            <Button size="sm" aria-label={`Add ${v.name} to My Voices`} disabled={!!asking} onClick={() => (setAdded(null), setAsking(v))}>
              Add…
            </Button>
          )}
        </span>
      ),
    },
  ];

  const total = page?.total ?? 0;
  return (
    <Dialog
      open
      kind="sheet"
      title="Browse voices"
      onClose={(v) => {
        // Return in a field submits the sheet's form: search, not close.
        if (v === "") return void load(query);
        onClose();
      }}
      actions={
        <Button type="submit" value="done" variant="primary">
          Done
        </Button>
      }
    >
      <div className={styles.segment} data-component="browse-voices">
        <div className={styles.bar} role="search" aria-label="Voice Library">
          <TextField label="Search" inline value={query.search} onChange={(e) => setQuery({ ...query, search: e.target.value, page: 0 })} />
          <Button size="sm" disabled={busy} onClick={() => void load(query)}>
            Search
          </Button>
          <SelectField label="Language" inline value={query.language} onChange={(e) => set("language", e.target.value)}>
            <Options list={LANGUAGES} any="Any" />
          </SelectField>
          <SelectField label="Accent" inline value={query.accent} onChange={(e) => set("accent", e.target.value)}>
            <Options list={ACCENTS} any="Any" />
          </SelectField>
          <SelectField label="Gender" inline value={query.gender} onChange={(e) => set("gender", e.target.value)}>
            <Options list={GENDERS} any="Any" />
          </SelectField>
          <SelectField label="Age" inline value={query.age} onChange={(e) => set("age", e.target.value)}>
            <Options list={AGES} any="Any" />
          </SelectField>
          <SelectField label="Use" inline value={query.use_case} onChange={(e) => set("use_case", e.target.value)}>
            <Options list={USES} any="Any" />
          </SelectField>
          <SelectField label="Category" inline value={query.category} onChange={(e) => set("category", e.target.value)}>
            <Options list={CATEGORIES} any="Any" />
          </SelectField>
          <SelectField label="Sort" inline value={query.sort} onChange={(e) => set("sort", e.target.value)}>
            {SORTS.map(([v, t]) => (
              <option key={v} value={v}>
                {t}
              </option>
            ))}
          </SelectField>
        </div>
        <div className={styles.bar}>
          <TextField label="Voice ID" inline value={byId} onChange={(e) => setById(e.target.value)} />
          <Button size="sm" disabled={busy || !byId.trim()} onClick={() => void load(query, byId.trim())}>
            Look Up
          </Button>
        </div>
        {error && (
          <Banner kind="error" icon="danger-triangle">
            {error}
          </Banner>
        )}
        {added && (
          <p className={styles.status} role="status">
            {added}
          </p>
        )}
        {asking && (
          <Banner
            kind="warning"
            icon="danger-triangle"
            action={
              <>
                <Button size="sm" variant="primary" onClick={() => void add(asking)}>
                  Add to My Voices
                </Button>
                <Button size="sm" onClick={() => setAsking(null)}>
                  Cancel
                </Button>
              </>
            }
          >
            This adds {asking.name} to your ElevenLabs account and takes one of its voice slots.
            {asking.rate != null ? ` Each character in this voice bills ${asking.rate}× the model's credits.` : ""}
          </Banner>
        )}
        <DataTable label="Voice Library" compact columns={cols} rows={page?.voices ?? []} rowKey={(v) => v.voice_id} empty={busy ? "Searching…" : "No voice matches."} maxHeight={LIST_HEIGHT} />
        {page && !single && (
          <div className={styles.bar}>
            <Button size="sm" disabled={busy || page.page === 0} onClick={() => go(page.page - 1)}>
              Previous
            </Button>
            <span className={styles.status} role="status" aria-label="Page">
              Page {page.page + 1}
              {total ? ` · ${total.toLocaleString("en-US")} voices` : ""}
            </span>
            <Button size="sm" disabled={busy || !page.has_more} onClick={() => go(page.page + 1)}>
              Next
            </Button>
          </div>
        )}
      </div>
    </Dialog>
  );
}
