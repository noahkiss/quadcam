# QuadCam architecture plan

Status: approved 2026-10-01; decisions in section 7.
Baseline: `main` at release 0.4.1.

Decided before this plan:

- The UI moves to React, TypeScript and Vite, with tauri-specta bindings.
- The look is "Mocha, tightened": Catppuccin Mocha and Latte, Space Grotesk and JetBrains Mono,
  an 8-pt rhythm, and a Clips | Map segmented control. The toolbar gear and the ⌘, menu item stay.
- Every step preserves behaviour. Tests stay green at every step.
- After the refactor, the next feature is digital sources: DJI O4 / Goggles 3, then HDZero and
  Walksnail.

---

## 1. Current map

### 1.1 Rust modules

Fan-out counts the other `crate::` modules a file names.

| Module | Lines | Job | Fan-out | Notes |
|---|---:|---|---:|---|
| `core_library.rs` | 1337 | `Core` library half: list, rate, edit, redate, relocate, cuts, Trash, Photos, strips, card status | 13 | God module |
| `mcp.rs` | 1303 | MCP stdio server, tool handlers, text tables, hand-written tool schemas | 2 | God module; untyped `Value` everywhere |
| `session.rs` | 1183 | Import session, plan patches, `Defaults` (settings view), `run_import` | 11 | God module; owns settings defaults |
| `bin/quadcam-cli.rs` | 1173 | clap CLI, JSON envelope, calls `Core` methods directly | 8 | Typed, but its own arg shapes |
| `core.rs` | 1093 | `Core` struct, session lock, import, verify, preview, format, `dispatch` | 11 | `dispatch` is 200 lines of local param structs |
| `library.rs` | 1080 | Library index, layout, QuickTime keys, rebuild, filter, groups | 5 | Cohesive |
| `moments.rs` | 1077 | Stick-moment and dead-air detection | 2 | Cohesive, pure, well tested |
| `pipeline.rs` | 900 | Stage, analyse, date plan, convert, verify, session cut export | 8 | Mixes analog source logic with the generic pipeline |
| `media.rs` | 729 | ffmpeg/ffprobe wrappers | 1 | Analog assumptions in args |
| `disk.rs` | 466 | diskutil, card detection, guarded FAT32 format | 1 | Analog card policy baked in |
| `lib.rs` | 451 | Tauri commands, GUI hooks, window, dev eval | all | 19 typed commands plus `core_call` |
| `logs.rs` | 451 | EdgeTX log parsing and clip matching | 0 | Cohesive, pure |
| `geocode.rs` | 439 | Apple, Nominatim, Census, Google search | 2 | Reaches up into `core` for a path |
| `settings.rs` | 437 | Settings file: locked read-modify-write, `KEYS` | 5 | Good design; `Defaults` lives elsewhere |
| `metadata.rs` | 419 | Places, profiles, resolve QuickTime items | 2 | Cohesive |
| `qtmeta.rs` | 395 | Apple `moov/meta` writer and reader | 0 | Cohesive, pure |
| `core_settings.rs` | 390 | `Core` settings, places, profiles | 4 | Fine |
| `menu.rs` | 288 | Native menu bar | 0 | Fine |
| `photos.rs` | 258 | PhotoKit behind a trait | 0 | Fine |
| `scan.rs` | 222 | Find AVI clips, check AVI structure | 0 | Analog only |
| `naming.rs`, `trim.rs` | 217 each | File names; the one cut model | 0, 1 | Fine |
| `watch.rs`, `control.rs`, `trash.rs`, `share.rs`, `main.rs` | 154, 127, 74, 71, 6 | File watch, socket, Trash, Share menu, entry | — | Fine |

Total: 14,957 lines. Tests: 50 unit tests in `src/`, 59 integration tests in `tests/`.

### 1.2 Front ends

| Surface | How it reaches `Core` | Typing |
|---|---|---|
| GUI | 19 Tauri commands plus `core_call(method, params)` (`lib.rs:281`, `lib.rs:426`); the UI uses 38 method strings | Stringly; no TS types |
| Control socket | `Core::dispatch` (`core.rs:846`) | Stringly |
| MCP | `Backend::call(method, Value)` (`mcp.rs:18`) to the socket or a local `Core` | Stringly; schemas by hand |
| CLI | Typed `Core` methods (`quadcam-cli.rs:8-13`) | Typed, but own arg shapes |
| Events | `Hooks::event(name, json!)` (`core.rs:386`, `core.rs:603`) | Stringly |

### 1.3 Pain points, with evidence

**God modules.** `core_library.rs`, `session.rs`, `mcp.rs` and `core.rs` each pass 1,000 lines
and import 11 to 13 sibling modules. `session.rs:174-318` holds `Defaults`, which is the settings
view, so `settings.rs` tests import `session` (`settings.rs:326`).

**Duplicated logic between `core.rs`/`pipeline.rs` and `core_library.rs`:**

| Logic | Session path | Library path | Drift already present |
|---|---|---|---|
| Write and verify a cut file | `pipeline.rs:789` `export_cuts` | `core_library.rs:833` `library_export_cuts` (150 lines) | Library cuts skip `media::verify_qt` (`pipeline.rs:877` has it). Numbering differs: `i + 1` vs `max + 1` |
| MP4 or MOV from extension | `pipeline.rs:811` | `core_library.rs:860` | Copy-paste |
| Preview proxy | `core.rs:724` with a static `MAKING` lock | `core_library.rs:1119`, no lock | Library may run two ffmpegs on one file |
| Add to Photos | `core.rs:637` | `core_library.rs:1085` | Different album defaults |
| Removed-cut decision | `core_library.rs:1256` `set_session_cuts` | `core_library.rs:742` `library_set_cuts` | Both build `ExportedCut` lists by hand |

**Hand-written MCP schemas.** `mcp.rs:1133-1303` is one `json!` literal (it needs
`#![recursion_limit = "256"]`, `lib.rs:6`). The handlers (`mcp.rs:180-890`) re-parse each argument
with `a.get(..)`. The same shapes exist three times: the MCP schema, the `dispatch` param structs
(`core.rs:853-939`), and the CLI clap structs (`quadcam-cli.rs:25-405`). `quadcam_library_edit`
fans out to three `dispatch` calls (`mcp.rs:256-330`), so one edit is not atomic.

**Analog-AVI assumptions spread through the pipeline:**

| Where | Assumption |
|---|---|
| `scan.rs:9` | `VIDEO_EXTS = ["avi"]` |
| `scan.rs:117` | `PICT` number sort order |
| `pipeline.rs:210-243` | `check_avi` (RIFF, `idx1`) decides "complete"; recovery writes `.recovered.avi` |
| `pipeline.rs:726` | Kept originals always get the `avi` extension |
| `pipeline.rs:254` | Dead-air scan runs on every clip (blue screen and static are analog signals) |
| `media.rs:118` | `recover` forces `-f avi` |
| `media.rs:192-230` | MP4 always re-encodes; MOV stream-copies "MJPEG". A digital H.264/H.265 clip should remux |
| `media.rs:518`, `core_library.rs:1123` | Proxy because the webview "cannot play MJPEG"; only `.mp4` plays directly |
| `media.rs:595` | Frame sampling assumes all-intra MJPEG |
| `disk.rs:8-11`, `disk.rs:149-160` | FAT32, 32 GB warning and a 64 GB format cap: true for analog DVRs, false for goggles cards |
| `pipeline.rs:346` | Dates come from radio logs or import day, because a DVR has no clock |

**Identity hash stability.** `pipeline::fingerprint` (`pipeline.rs:61-79`) and `library::head_id`
(`library.rs:399`) use `std::hash::DefaultHasher`. Its algorithm is unspecified across Rust
releases. The comment at `pipeline.rs:58-60` says the hash "need not be stable", but it is
persisted as `app.quadcam.source` and compared in `card_status` (`core_library.rs:1237`). A
toolchain bump could change every id and the "N new" count.

**Layering.** `geocode.rs:199` calls `core::cache_dir()`. `media.rs` and `moments.rs` import each
other. Path helpers live in `core.rs:176-200`.

**UI state (`ui/app.js`, 2,546 lines):**

- One mutable `state` object (`app.js:41`) with about 23 fields across screens, mutated in 96 places.
- `settings` is a clone of a hand-copied `DEFAULTS` (`app.js:13-37`) that duplicates Rust
  `Defaults` (`session.rs:174`).
- `renderAll` (`app.js:486`) rebuilds the whole screen. `state.renaming` blocks it during a rename,
  a guard against losing input focus.
- `loadLibrary` (`app.js:201`) diffs by `JSON.stringify` of every clip.
- Undo (`history.js`) is a sound command stack, but app.js builds each step inline.
- No UI tests. No types. The only check is a manual run.

---

## 2. Target Rust layout

Keep one package for now; split into a workspace only if build times hurt (see step R9).

```
src-tauri/src/
  paths.rs            support, cache, settings and session paths (from core.rs:176-200)
  api/                shared request and response types: serde + schemars + specta
    mod.rs            the method table macro: name, params, result, doc
    session.rs, library.rs, setup.rs, events.rs
  core/               Core: state, locking, hooks; no surface code
    mod.rs            Core struct, claim/commit, Hooks
    import.rs         stage, analyse, dates, patch, import, verify, format
    library.rs        list, rate, edit, rename, redate, relocate
    cuts.rs           ONE cut writer and ONE removed-cut rule for session and library
    files.rs          Trash, Photos, previews, strips (one proxy lock)
    setup.rs          settings, places, profiles (today's core_settings.rs)
  sources/            footage sources
    mod.rs            trait Source, SourceKind, registry, detect()
    analog.rs         AVI/MJPEG DVR: scan, check_avi, recover, dead air, FAT32 card policy
    dji.rs            later: DJI O4 / Goggles 3 (.mp4 + .srt)
    hdzero.rs, walksnail.rs   later
  pipeline/           source-agnostic: stage, plan dates, convert or remux, verify
  library/            index, layout, keys, rebuild, filter (today's library.rs)
  metadata/           metadata.rs + qtmeta.rs + naming.rs
  analysis/           moments.rs + logs.rs (pure)
  places/             geocode.rs, per provider
  settings.rs         settings file, KEYS, and Defaults (moved from session.rs)
  media.rs            ffmpeg/ffprobe calls only; no format policy
  disk.rs, photos.rs, trash.rs, trim.rs
  front/
    tauri.rs          generated commands + GUI hooks (from lib.rs)
    control.rs        socket
    mcp/              server.rs, tools.rs (schemas derived), render.rs (text tables)
  bin/quadcam-cli.rs
```

### 2.1 The `Source` trait

```rust
pub trait Source: Send + Sync {
    fn kind(&self) -> SourceKind;                       // Analog, Dji, HdZero, Walksnail
    fn detect(&self, root: &Path) -> Option<Detected>;   // card or folder; confidence
    fn list(&self, root: &Path) -> Vec<FoundClip>;       // with sidecars attached
    fn sidecars(&self, clip: &Path) -> Sidecars;         // .srt, .osd; empty for analog
    fn inspect(&self, tools: &Tools, staged: &Path) -> Inspect;  // complete? repairable?
    fn repair(&self, tools: &Tools, staged: &Path, dst: &Path) -> Result<()>;
    fn intrinsic_time(&self, clip: &StagedClip) -> Option<DateTime<Utc>>; // None for analog
    fn encode_plan(&self, probe: &Probe, want: Format) -> EncodePlan; // Transcode | Remux
    fn signal(&self, tools: &Tools, clip: &StagedClip) -> Option<SignalScan>; // dead air, link
    fn card_policy(&self) -> CardPolicy;                 // format allowed? FS? size cap?
    fn original_ext(&self, staged: &Path) -> String;
}
```

- `analog.rs` moves today's code behind it unchanged. That is the behaviour-preserving step.
- The session stores `SourceKind` per clip. The library writes it as `app.quadcam.video_system`
  (the key exists at `metadata.rs:258`).
- `disk::format_card` keeps every guard. `CardPolicy` only adds a "this source allows format"
  check in front of the existing guards. No guard moves out of `format_card`.
- Later features hang off the trait: OSD burn-in reads `sidecars`, corrupt-clip repair extends
  `inspect`/`repair`, weather and sun tags use `intrinsic_time` plus place.

### 2.2 One set of shapes for every surface

The method table in `api/mod.rs` is the single source:

```rust
api! {
  /// List and search the library.
  library(LibraryQuery) -> LibraryView;
  library_edit(LibraryEdit) -> LibClip;
  session_patch(SuggestParams) -> Session;
  ...
}
```

The macro generates:

| Output | Used by |
|---|---|
| `Core::dispatch(method, Value)` match arms | Control socket, headless MCP |
| One `#[tauri::command]` per method with `#[specta::specta]` | GUI; replaces `core_call` |
| `bindings.ts` via tauri-specta (commands and typed events) | React UI |
| `schemars::schema_for!(Params)` per method | MCP `inputSchema` |

- Every param and result type derives `Serialize, Deserialize, JsonSchema, specta::Type`.
  Validation hints move onto the type: `#[schemars(range(min = 0, max = 5))]`,
  `#[schemars(regex(pattern = ...))]`, `#[schemars(length(max = 80))]`.
- MCP tools stay coarse (16 tools, action fields). Each tool gets a typed `ToolParams` enum or
  struct deriving `JsonSchema`. Its handler deserializes once and calls one `api` method.
  `quadcam_library_edit` gets one atomic `library_edit` that takes rating, flag and name too.
- Tool descriptions stay hand-written prose in `mcp/tools.rs` as `const` strings; only schemas
  derive.
- The CLI keeps clap for flags but builds the same `api` param structs, and `--plan` reads them
  as JSON. Its `--json` output is the `api` result type.
- `Hooks::event` takes a typed `api::Event` enum. tauri-specta exports it to TS.
- Guard: a snapshot test of `tools()` and of `bindings.ts`. Any shape change shows in review.

Versions: specta and tauri-specta are still release candidates for Tauri 2. Pin exact versions
(`=2.0.0-rc.x`) and record them in AGENTS.md.

---

## 3. Target UI layout

```
app/                         new Vite root (old ui/ stays until cutover)
  vite.config.ts             server.port 4719, strictPort true
  index.html
  src/
    main.tsx, App.tsx
    bindings.ts              generated by tauri-specta; never edited
    ipc/                     thin wrappers, event subscription, mock for tests
    store/                   Zustand slices
      library.ts session.ts settings.ts ui.ts tasks.ts history.ts
    views/
      Library/               grid, list, day groups, selection bar, banners
      ClipDetail/            player, moments, TrimTimeline, Inspector
      ImportSheet/           Load, Review, Add to Library, Finish, Format card
      Settings/              Library, Import, Aircraft, Places, Photos, Advanced
      Map/                   placeholder until the native MapKit view
      FirstRun/
    components/
      Toolbar, Sidebar, SegmentedControl, ClipCard, ClipRow, DayHeader, Stars, FlagButton,
      FlyingBar, TrimTimeline, KeepBar, MomentChip, Inspector fields (TextField, DateField,
      TimeField, Select, TokenField), Sheet, Dialog, Toast, Banner, ProgressRing, Stepper
    theme/
      tokens.css             Catppuccin Mocha + Latte, spacing, radii, type
      fonts.css              local woff2 (CSP font-src 'self'; no Google Fonts)
    menu/useMenu.ts          menu ids -> actions; pushes menu_state
```

### 3.1 State: Zustand

Pick Zustand, with one store split into slices.

- Tauri events arrive outside React. Zustand's `setState` works from any listener; context
  plus `useReducer` needs a dispatch bridge.
- Selectors re-render only the cards whose data changed. That replaces `renderAll` and the
  `state.renaming` guard: a field being edited keeps focus.
- The store is plain TS, so Vitest tests it without a DOM.
- It is small (about 1 kB) and needs no provider.

Rules:

- The core stays the source of truth. Slices cache `LibraryView`, `Session` and `SettingsView`.
  An event (`library-changed`, `session-changed`, `settings-changed`) triggers a refetch.
- No hand-copied defaults. Settings come from `settings` (typed) only; `DEFAULTS` in app.js goes.
- Per-machine view prefs (grid/list, thumb size, sort) stay settings keys, as today.

### 3.2 Undo and redo

Port `history.js` as `store/history.ts`: the same `{label, undo, redo}` steps, limit 100, a new
edit clears redo, a failed step drops out. Each library action builds its step in one place
(`actions/library.ts`), not inline in views. The menu's Undo/Redo labels read from the slice.

### 3.3 Menu integration

Keep `menu.rs`. `useMenu` subscribes to the `menu` event and maps ids to the same action table
the toolbar and keys use. A store subscription calls `menu_state` when enabled or checked
state changes. Settings keeps both the toolbar gear and ⌘, (menu id `settings`).

### 3.4 Virtualization

Use `@tanstack/react-virtual`. The library grid flattens into rows: day headers and card rows,
with the column count from container width and `--thumb`. List view virtualizes table rows.
Hover-scrub strips load through `IntersectionObserver`, as `stripObserver` does today
(`app.js:332`).

### 3.5 Theming

- `tokens.css` defines semantic tokens (`--bg`, `--mantle`, `--surface`, `--fg`, `--muted`,
  `--primary`, `--success`, `--warning`, `--error`, `--accent`) for Mocha under
  `prefers-color-scheme: dark` and Latte under `light`. The values come from
  `@catppuccin/palette`, not hand-typed hex.
- Spacing: `--s-1: 4px` `--s-2: 8px` `--s-3: 12px` `--s-4: 16px` `--s-6: 24px` `--s-8: 32px`.
  Radii 6, 8, 10. Type sizes 11, 12, 13, 15, 18.
- Fonts stay bundled in `app/public/fonts/`; the canvas loads Google Fonts, the app must not.
- Styles: CSS Modules per component, tokens only, no hex in components.

### 3.6 How the Mocha boards map to components

| Board | Components |
|---|---|
| Library | Toolbar (logo, SegmentedControl Clips/Map, sort, search, thumb size, grid/list, Import…, gear), Sidebar (Library, Import from, Flying days, Aircraft, Places, Smart groups), DayHeader ("Place · Aircraft · N clips · X min flying"), ClipCard (duration, name, time, Stars, FlagButton, FlyingBar), SelectionBar (Rate, Share, Trash), Footer (totals, library path) |
| Library, light | Same components; Latte tokens |
| Clip detail | Player, MomentChip row, TrimTimeline (lanes Video, Signal, Moments, Cuts; In, Out, Add cut, Save cuts), Inspector tabs Details / Flight, prev/next |
| Import, review | Sheet with Stepper (Load, Review, Add to Library, Finish), batch bar (Aircraft, Place, Date, Radio logs), ClipRow list, review panel with TrimTimeline and Inspector, "Apply to all clips" |
| Settings | Sheet with panes Library, Import, Aircraft, Places, Photos, Advanced; aircraft Video system field (Analog, DJI, HDZero, Walksnail) |
| Map | Map view: places list, Map/Satellite toggle; native MapKit later |

`TrimTimeline` replaces `trim.js` (385 lines) and keeps its contract: one component, used by
ClipDetail and the import review, with callbacks into the core.

---

## 4. Migration plan

Two tracks run in parallel: Rust (R) and UI (U). U3 onward needs R5. Each step is one PR,
tests green before merge. Rollback is `git revert` of that PR unless the table says otherwise.

### 4.1 Rust track

| Step | Scope | Tests | Size | Parallel |
|---|---|---|:-:|---|
| R0 Safety net | Snapshot `mcp::tools()` JSON, the `dispatch` method list, CLI `--help`, and JSON of `Session` and `LibraryView` from the test corpus. Add a `fingerprint` test vector | New snapshots (insta) | S | First |
| R1 Paths and settings | Add `paths.rs`; move `Defaults` to `settings.rs`; `geocode` stops importing `core` | Existing suite | S | With R2 |
| R2 One cut writer | `core/cuts.rs`: one `write_cut()` for session and library, one `format_for_ext`, one `ExportedCut` builder. Library cuts gain `verify_qt` (owner decision, Q2) | Existing cut tests; new library cut test that checks QuickTime keys | M | With R1 |
| R3 One proxy, one Photos path | One proxy fn with the lock; one `share_files` helper | Existing; new concurrent-preview test | S | After R2 |
| R4 Split god modules | Pure moves: `core/` dir, `session.rs` loses `run_import` to `pipeline/import.rs`, `mcp.rs` into `server/tools/render` | Existing suite; no snapshot diff | M | After R1-R3 |
| R5 `api` table + specta | Param/result types into `api/`; macro generates `dispatch` and Tauri commands; tauri-specta writes `bindings.ts` (debug build and a test); typed `Event` enum | Snapshots unchanged; `bindings.ts` check in CI | M | After R4 |
| R6 Derived MCP schemas | schemars on tool params; handlers deserialize once; atomic `library_edit` | `tests/mcp.rs`; tools snapshot diff reviewed once, then frozen | M | With U1-U3 |
| R7a `Source` trait | `sources/` with `analog` wrapping `scan` + `check_avi` + recover; pipeline calls the trait | Existing import tests on the synthetic corpus | M | After R4 |
| R7b Encode and signal policy | `EncodePlan`, `signal()`, `original_ext()`, `CardPolicy` through the trait; analog returns today's values | Existing; unit tests per method | M | After R7a |
| R8 CLI on `api` | clap builds `api` params; `--plan` uses them; output uses result types | `tests/cli.rs` unchanged | S | After R5 |
| R9 Workspace split (optional) | `quadcam-core` (no Tauri), `quadcam-app`, `quadcam-cli` | Full suite; CLI builds without Tauri | L | Last; skip if not needed |

### 4.2 UI track

Recommendation: **build the new UI side by side, then cut over once.**

- The new UI lives in `app/` and builds to `app/dist`. The legacy `ui/` stays bundled.
- A debug-only switch picks the entry: `QUADCAM_UI=legacy|next` (default `legacy` until U8).
  Release builds ship only the default.
- Screens move one at a time inside the new UI, but the person switches whole apps, not
  screens. Mixing React islands into app.js would need a state bridge both ways; that costs
  more than it saves.
- Parity specs run against both UIs on the same mocked IPC, so "behaviour preserved" is a test,
  not a judgement.

| Step | Scope | Tests | Size | Parallel |
|---|---|---|:-:|---|
| U0 Scaffold | Vite + React + TS in `app/`, port 4719 `strictPort`, `devUrl` and `beforeDevCommand` in `tauri.conf.json`, the `QUADCAM_UI` switch, Node pinned with fnm, lockfile, CI job (typecheck, lint, test, build) | CI builds both UIs | S | With R0 |
| U1 Mock IPC + parity harness | `ipc/mock.ts` fakes `invoke` and events from recorded core JSON (R0 snapshots). Playwright loads legacy `ui/` with the mock and records parity specs: open library, select, rate, flag, rename, undo, open detail, add cut, import review edits, settings save | Playwright on legacy | M | With R1-R4 |
| U2 Theme + base components | tokens, fonts, SegmentedControl, Button, fields, Stars, FlagButton, Sheet, Dialog, Toast | Vitest + RTL; axe checks | M | After U0 |
| U3 Store, events, menu, undo | Zustand slices, event subscriptions, `useMenu`, `history.ts` on `bindings.ts` | Vitest on slices and history | M | Needs R5 |
| U4 Library | Toolbar, Sidebar, grid/list with virtualization, day headers, selection, banners, first run, Map tab placeholder | Parity specs pass on next; Vitest for selection and sort | L | After U3 |
| U5 ClipDetail + TrimTimeline | Port trim.js behaviour, Inspector, Details/Flight tabs, prev/next | Parity specs; Vitest on trim math | L | After U4 |
| U6 ImportSheet | Load, Review, Add to Library, Finish; agent suggestion badges; format card dialog and `agent-format-request` | Parity specs incl. agent-suggest and removed-cuts flows | L | After U5 or with it |
| U7 Settings | Six panes; saves only changed keys; places search; profile editor | Parity specs | M | With U5/U6 |
| U8 Flip default | `next` becomes default; legacy stays one release behind the switch | Full suites; manual smoke by the owner | S | Last |
| U9 Remove legacy | Delete `ui/`, the switch and `withGlobalTauri` | Suites | S | One release after U8 |

U8 and U9 shipped together in 0.5.0: the new UI is the only UI (decision 7, changed).

Rollback for U8 is flipping the default back. Before U8, the new UI is unreachable in release.

### 4.3 Then the feature track

| Step | Scope | Size |
|---|---|:-:|
| F1 DJI O4 / Goggles 3 | `sources/dji.rs`: detect the goggles card layout, list `.mp4` + `.srt`, intrinsic time, remux plan, card policy (no FAT32 format), SRT parse for link stats | L |
| F2 HDZero | `.ts` files, remux to MP4, its card policy | M |
| F3 Walksnail | `.mp4` with `.srt` and `.osd` sidecars | M |
| Later | Map tab (native MapKit view), Open-Meteo weather, sun tags, OSD burn-in, share-ready export, corrupt-clip repair | — |

### 4.4 Order at a glance

```
R0 ─┬─ R1 ─┐
    └─ R2 ─┴─ R3 ─ R4 ─┬─ R5 ─┬─ R6
                       │      └─ R8
                       └─ R7a ─ R7b ─────────────── F1 ─ F2 ─ F3
U0 ─ U1 ─ U2 ─────────── U3(R5) ─ U4 ─ U5 ─ U6 ─ U8 ─ U9
                                        └ U7 ┘
```

---

## 5. Test strategy

### 5.1 Today

| Layer | What exists | Gap |
|---|---|---|
| Rust unit | 50 tests: logs, moments, naming, disk guards, settings, qtmeta, trim | No test for `dispatch` shapes |
| Rust integration | 59 tests: import on synthetic AVI, FAT32 disk images, library, MCP (headless and app mode), CLI, settings | Library cut export does not check QuickTime keys |
| Schemas | None | MCP schema and CLI drift go unseen |
| UI | None | Every UI change is checked by hand |
| CI | fmt, clippy, tests, release build on macos-26 | No UI job |

### 5.2 Add first

1. **R0 snapshots** (insta): MCP tool list, `dispatch` methods, CLI help, `Session` and
   `LibraryView` JSON. They make every later Rust step provably shape-preserving, and they seed
   the UI mock.
2. **Fingerprint test vector**: fixed bytes in, fixed id out. It fails if a toolchain changes
   the hash.
3. **Playwright parity specs on the legacy UI** (U1), with mocked Tauri IPC.
4. **Vitest** for store slices, history and trim math, as each lands.

### 5.3 UI test rules

- Vitest + React Testing Library in jsdom for components and slices.
- Playwright against the Vite dev server (port 4719) with `ipc/mock.ts` injected before page
  scripts. Mock data comes from the R0 snapshots, so mocks track real core shapes.
- Playwright drives a headless browser it owns. Never real mouse or keyboard events on the
  owner's Mac, never focus-taking windows. An end-to-end run of the real app keeps the existing
  `QUADCAM_NO_FOCUS=1` + `QUADCAM_DEV_EVAL` path with `HOME=<temp>`.
- No test touches the real settings, library, Trash or Photos (existing rules stand).
- CI runs `tsc --noEmit`, lint, Vitest, Playwright, and a check that `bindings.ts` is current.

---

## 6. Risks and open questions

### Risks

| Risk | Mitigation |
|---|---|
| specta / tauri-specta are RC | Pin exact versions; snapshot `bindings.ts`; the `api` macro hides them |
| Generated MCP schemas differ in text from the hand ones | Review the R6 snapshot diff once; keep descriptions as written |
| Node enters a repo whose AGENTS.md says "No Node" | Pin Node with fnm and one package manager; update AGENTS.md and README in U0 |
| CSP blocks the dev server | Add the dev origin to `connect-src` only in dev config |
| Two UIs drift during the side-by-side phase | Freeze UI features in `ui/` after U1; parity specs gate U8 |
| `DefaultHasher` ids change on a toolchain bump | R0 test vector now; Q1 decides the fix |
| Digital cards are exFAT and large | `CardPolicy` per source; format stays analog-only until decided (Q5) |

### Questions for the owner

1. **Identity hash.** Replace `DefaultHasher` with a specified hash (for example xxHash64 or
   BLAKE3 over the same head and tail bytes)? That needs a one-time migration: read the old id,
   write the new one, keep the old as an alias. Or keep it and rely on the test vector?
2. **Library cut verify.** Library cuts skip the QuickTime verify that session cuts run. R2
   would add it. That is a behaviour change (a bad metadata write now fails the cut). Accept?
3. **Node tooling.** pnpm or npm? Node version via fnm `.node-version`?
4. **Map tab before MapKit.** Show "Map" in the segmented control as disabled, or hide it until
   the Map feature ships?
5. **Format for digital cards.** Goggles format their own cards. Offer "Format card" for
   digital sources at all?
6. **Workspace split (R9).** Worth doing, or stay one package?
7. **Legacy lifetime.** Keep `ui/` for exactly one release after U8, then delete?
8. **Off-grid spacing in the canvas.** Boards use 6 px and 10 px gaps. Snap them to 4/8/12, or
   keep 4-px half steps?

## 7. Decisions (2026-10-01)

The owner approved the plan and answered the questions:

1. **Identity hash:** replace `DefaultHasher` with a specified, stable hash. Ship a migration for
   existing libraries; it must work on a real library that already has an index, and a test
   runs it on a library built by the old code.
2. **Library cut verify:** accepted. R2 adds `verify_qt` to library cut export. This bug fix is
   the one allowed behaviour change.
3. **Node tooling:** pnpm, with Node pinned through fnm.
4. **Map tab:** hidden until the map works.
5. **Format for digital cards:** yes, with the same guards, decided per system when each lands.
   The `Source` trait declares whether a source offers formatting.
6. **Workspace split:** no. R9 is skipped.
7. **Legacy lifetime:** `ui/` stays for one release after U8, then goes. *Changed
   2026-10-01:* the owner dropped the extra release. 0.5.0 ships the new UI as the only UI,
   so U8 and U9 land together: `ui/`, the `ui` setting, `QUADCAM_UI`, `withGlobalTauri`, the
   deprecated legacy commands, `core_call` and the legacy `eject`/`format_plan` handler are
   gone. A settings file with `ui` still loads; reads drop the key and the next write
   removes it.
8. **Spacing:** snap to the 8-pt grid.

### Calls made during the Rust track

The plan did not cover these; each took the conservative choice.

- **Identity (Q1).** New ids use XXH64 over the same sampled bytes: `x…` for a source
  fingerprint, `hx…` for a head id (`identity.rs` states the bytes). The 0.4 ids stay
  computable through an explicit SipHash-1-3 (`identity::legacy`). The migration writes no
  media file: a 0.4 source id moves only when the kept original proves it, because the card
  file is gone otherwise; every other clip keeps its 0.4 id, which still matches cards.
  Moved clips keep the 0.4 id in `aliases`, and every lookup accepts it.
- **Cut writer location.** The pure writer is `src/cuts.rs`, not `core/cuts.rs`, because
  `pipeline::export_cuts` uses it and the pipeline must not depend on `core`. `core/cuts.rs`
  holds `Core`'s cut methods.
- **Cut numbering.** Session cuts keep `i + 1` and library cuts keep `max + 1`.
- **Atomic library edit.** `quadcam_library_edit` calls a new `library_update` method
  instead of a wider `library_edit`, because the legacy UI calls `library_edit` with its
  current shape.
- **Command names.** The typed Tauri commands carry the method names and take one `params`
  struct. Two names were already legacy commands (`eject`, `format_plan`). A call without
  `params` went to a small legacy handler in `lib.rs` that answered it as before. It went
  with the legacy UI in 0.5.0 (U9).
- **bindings.ts.** It lives at `app/src/bindings.ts`. specta exports `f64` as
  `number | null`, because the lossless-float option adds runtime transforms that fail
  `tsc --strict`. `serde_json::Value` fields are typed `unknown`; `Result` fields use
  serde's `{Ok}`/`{Err}` shape.
- **Sources (R7).** The library does not yet write `app.quadcam.video_system` from a clip's
  `SourceKind`, because that would change file metadata. `disk::format_card` still erases as
  FAT32; the per-source file system waits for F1.
