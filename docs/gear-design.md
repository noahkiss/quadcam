# Gear: design

Status: draft for review, 2026-10-07.
Baseline: `main` at release 0.6.4.
Target: 1.0.0, built in work packages by several agents in parallel (section 10).

QuadCam imports clips today. Gear adds the rest of an FPV bench to the same app: the radio,
the flight controller, the ELRS link, the goggles and DVR cards, the packs, and the sims. It
backs up each device when it is plugged in, shows how the radio and the quad fit together,
stages every change with a before/after diff, and applies it behind one confirm with a
backup first and a verify after.

This doc is the contract for the build. It names the modules, the data, the `api` rows, the
CLI, the MCP tools, the safety model, the tests, the work packages and the releases.

---

## Contents

1. [Rules this design keeps](#1-rules-this-design-keeps)
2. [Information architecture and screens](#2-information-architecture-and-screens)
3. [Rust core modules](#3-rust-core-modules)
4. [Data model and storage](#4-data-model-and-storage)
5. [Surfaces: `api` rows, CLI, MCP](#5-surfaces-api-rows-cli-mcp)
6. [Device protocol notes](#6-device-protocol-notes)
7. [Feature notes](#7-feature-notes)
8. [Safety model](#8-safety-model)
9. [Testing strategy](#9-testing-strategy)
10. [Build plan: work packages](#10-build-plan-work-packages)
11. [Release plan](#11-release-plan)
12. [Licensing](#12-licensing)
13. [Open questions for the owner](#13-open-questions-for-the-owner)

---

## 1. Rules this design keeps

| Rule | Where it comes from | What it means for Gear |
|---|---|---|
| One `api` method table drives the CLI, the MCP server, the socket and the GUI | `AGENTS.md`, Rules | Every Gear feature is a `Core` method first, then a row in `api/gear.rs` |
| Keep the MCP surface small | `AGENTS.md`, MCP server | Three new tools, split by risk (section 5.3) |
| Publishable repo | `AGENTS.md` | No personal setup in code or fixtures. A user's switch layout, voice lines, binding phrase, packs and model names are user data at runtime |
| Every guard runs again right before the destructive call | `AGENTS.md`, Rules (format) | The same pattern for every device write (section 8) |
| Never touch real devices, settings, Photos or Trash in tests | `AGENTS.md`, Rules | Fail-safes for serial ports, DFU, flash tools, sims folders and TTS (section 9.1) |
| Docs change in the same change | `AGENTS.md` | Each work package updates `docs/` and `README.md` for what it ships |
| Native Mac look | Owner preference | Sidebar, toolbar, sheets, inspector, context menus, standard keys |

## 2. Information architecture and screens

### 2.1 Where Gear sits

The library stays the home screen. Gear is a second section of the same sidebar, under the
library groups, like a second source list in Music or Finder. It collapses with a disclosure
triangle. No new window and no mode switch.

```
Library                 (existing groups)
  All clips, Last import, Flying days, Aircraft, Places, Smart groups
Import from             (existing: cards, Unfinished import, Folder…)
Radio logs              (existing)
Gear
  Connected             one row per device plugged in now, with a "Backed up" or "Backing up" detail
  Bench                 count badge = staged changes not yet applied
  Aircraft              one row per aircraft (an aircraft profile with gear linked)
  Radios                one row per radio
  Packs
  Flights
  Sims                  badge "Out of date" when a sim's rates differ from the quad's
  Firmware              badge = updates available (after a check)
  Storage               total size of backups and logs
```

The Settings window gets a **Modules** section (7.10).

A status bar along the bottom of the main window shows one item per kind of device plugged in
(radio, quad, DJI, DVR card) with its state: working, safe to unplug, still inserted (with
**Dismiss** for the reminder), needs attention. A click opens the device page. The Settings
section list shows the version and the build (commit).

- **Aircraft** in the Library groups filters clips, as today. **Aircraft** under Gear opens
  the aircraft page. Each page links to the other ("Show clips", "Show gear").
- A device row in **Connected** opens its page. A row disappears when the device is
  unplugged; the page stays open on the latest backup.

### 2.2 Pages

Each page has a segmented control under the toolbar, one segment per view. The inspector (a
right-hand panel, toggled with the toolbar button and ⌥⌘I) shows the selected item.

| Page | Segments | Main job |
|---|---|---|
| Aircraft | Overview · Switches · OSD · Rates · Settings · Backups · Changes | One quad: its FC, its radio model and its receiver, joined |
| Radio | Overview · Models · Voice · Checklists · Splash · Backups · Changes | One radio and its SD card |
| Bench | List grouped by device, then by status | What to do at the next session |
| Packs | Table of packs; a pack's history in the inspector; Charging | The pack inventory |
| Flights | Table of flights from radio logs, by day | Hover, sag, mAh, dropouts per flight |
| Sims | One row per sim found, with its rate profiles | Keep sims on the quad's rates |
| Firmware | Table of devices, installed and latest versions | Check and flash |
| Storage | Total size; size per device (snapshots, logs, blobs only it uses); **Prune now**, **Export…**, **Import backups…** | Keep the gear folder small |

**Overview** shows the identity (board, firmware, version, the date of the latest backup),
the staged changes for the device, and the next actions. It never repeats what another
segment shows in full.

### 2.3 Plug-in flow

1. A device appears under **Connected** (radio SD card mounted, FC serial port, DFU device).
2. With **Back up on connect** on (Settings > Gear, default on), QuadCam backs it up. The
   row shows "Backing up", then "Backed up". A backup identical to the latest is not kept.
3. If the device has staged changes, the device page shows a bar at the top: "3 changes
   ready" and **Review…**. Nothing is applied on its own.
4. **Review…** opens the apply sheet (section 2.4).
5. A device QuadCam does not know asks for a name and an aircraft in a small sheet. It can
   be dismissed; the device then shows as "Unnamed FC" and still gets backed up.

When **Connected** is empty, it says: "Nothing found. If macOS asked to allow an accessory,
click Allow." On Apple silicon, macOS keeps a new USB accessory off the bus until the person
allows it, and an app cannot see that prompt (7.11).

An FC on USB gets a session timer in its row ("USB 12 min"), counting while its battery is in
(MSP battery voltage above 1 V, read every 30 s with the port closed between reads). At the
limit QuadCam plays one "Unplug <FC> now." cue (speech and notification): small quads overheat
on USB with a battery and no airflow. The limit is `gearUsbMinutes` (default 20), or the
board's own shorter limit in `bf/boards.rs` (10 min for the proven BetaFPV G473 boards).
`Core::gear_usb_tick` runs from the app's poll; `gear_usb_timers` and `GearStatus.usb_timers`
give the countdown (built in WP2).

### 2.4 The apply sheet

One sheet for every device write: FC settings, radio files, voice packs, restores, firmware,
sims and card prep.

- **Header:** the device, its identity and the backup that will be taken.
- **Changes:** a list. Each item shows its before/after diff: CLI lines for an FC, a line
  diff for each YAML file, a file list for sound files, a version pair for firmware.
- **Checks:** each guard with a pass mark or the reason it fails (section 8.2).
- **Buttons:** **Cancel** and **Apply**. **Apply** is disabled while a check fails. Return
  does not press **Apply**; this matches the format dialog.
- **During:** a progress row per step (Back up, Write, Read back, Verify).
- **After:** "Verified" per change, or the lines or files that failed, with **Restore
  backup** (it stages the restore as a change and opens it in the same sheet).

### 2.5 Native patterns

- Source-list sidebar, unified toolbar, segmented controls, sheets for apply and for naming,
  popovers for small edits, the inspector panel for details.
- Context menus on every list row (Back up now, Show in Finder, Stage restore, Copy as text).
- Standard keys: ⌘Z undoes an edit to a staged change that is not applied. ⌘⌫ discards the
  selected change, after a confirm. Space previews a voice line.
- Tables sort by column. Diffs and CLI lines use the monospace face; nothing else does.
- No status dots. A state is a word in the secondary text ("Connected", "Applied",
  "Verified") or an SF-style symbol with an accessible name.
- Labels say what a control does. No help text in the UI (design skill, section 4).

### 2.6 Settings window

A new **Gear** section:

| Setting | Key | Default |
|---|---|---|
| Gear folder | `gearDir` | `~/Library/Application Support/app.quadcam/gear` |
| Back up on connect | `gearAutoBackup` | on |
| Keep recent plug-in backups | `gearKeepRecent` | 10 per device |
| Keep one a week for | `gearKeepWeeks` | 8 weeks |
| Keep one a month | `gearKeepMonthly` | on (no limit) |
| USB time warning | `gearUsbMinutes` | 20 |
| Voice provider | `ttsProvider` | `say` (macOS) |
| Voice provider key | `ttsKey` | none; read from `QUADCAM_TTS_KEY` first; in `SECRET_KEYS` |
| Check for firmware | `firmwareCheck` | `manual` (`manual` or `daily`) |
| Tools (esptool, ffmpeg) | `modules` | QuadCam's own modules (7.10); a path per tool overrides one |
| Use Homebrew ffmpeg | `ffmpegSource` | `module` (`module` or `homebrew`) |
| Steps on connect | `gearOnConnect` | per device kind: `backup` only (`import`, `apply_ready`, `blackbox` off) (7.11) |
| Erase blackbox after download | `gearEraseBlackbox` | off (7.12) |
| Read the blackbox through USB disk mode first | `gearBlackboxMsc` | off; not proven on a real FC (7.12) |
| Cues | `gearCues` | speech and notifications on, sound off, each cue on, not muted; debounce 30 s; reminder after 60 s, every 300 s, 3 at most; no quiet hours (7.11) |

Each key goes into `settings::KEYS` with a default in `Defaults`, as today.

## 3. Rust core modules

New code lives under `src-tauri/src/gear/`. Every module runs without Tauri, like the
existing logic modules. `core/gear.rs` holds the `Core` methods; `api/gear.rs` the rows.

| Module | Job | Depends on |
|---|---|---|
| `gear/store.rs` | The gear folder: `gear.json` (locked read-modify-write, the `settings.rs` pattern), backups, changes, caches. The only writer of the gear folder | `paths`, `settings` |
| `gear/model.rs` | Shared types: `Device`, `DeviceKind`, `Identity`, `Backup`, `StagedChange`, `Edit`, `ApplyPlan`, `Refusal` | serde, specta, schemars |
| `gear/events.rs` | Device events from one look to the next: connected, identified, unmounted but present, removed; presence after unmount (7.11) | `detect` |
| `gear/cues.rs` | Spoken, sound and notification cues; the "still inserted" reminder (7.11) | `say`, `afplay`, `osascript` |
| `gear/detect.rs` | Finds devices: EdgeTX volumes (extends `disk::looks_like_radio`), serial ports by USB VID/PID, DFU devices, DVR and goggles cards (existing `Volume`). Polls every 2 s while the app runs | `disk`, `serial` |
| `gear/serial.rs` | One `SerialLink` trait (open, write, read until, close, "port gone"). The real one wraps the `serialport` crate; the fake one replays a script. A lock file per port so the app and the CLI never open the same port. The fail-safe for tests | `serialport` |
| `gear/bf/cli.rs` | Betaflight CLI session: enter, command, `diff all`, `dump all`, `get`, `save` (reboot), `exit` | `serial` |
| `gear/bf/msp.rs` | MSP v1/v2 framing and the read-only messages in section 6.2 | `serial` |
| `gear/bf/blackbox.rs` | The blackbox flash over MSP: summary, chunked read, erase and poll, the log header reader, the time estimates (7.12) | `bf/msp` |
| `gear/blackbox.rs` + `core/blackbox.rs` | Pull records, the pairing of logs with flights (a labelled guess), the keep set for blob collection; the `Core` jobs: pull, list, export, erase, the USB disk path and the on-connect step (7.12) | `blobs`, `bf`, `flights` |
| `gear/bf/dump.rs` | Parses `dump all` and `diff all` into a typed config with section context (`profile N`, `rateprofile N`); renders CLI lines back | none |
| `gear/bf/fake.rs` | `FakeFc`: a CLI and MSP simulator seeded from a synthetic dump (tests and the mock core) | `bf/dump` |
| `gear/bf/boards.rs` | Known issues per board and build (shown after a job and on the device page) and each board's USB time limit. Data | `compat` |
| `gear/bf/mod.rs` + `core/fc.rs` | FC jobs: identify (MSP), read (CLI), run (the write engine, for `apply/fc.rs`), the USB heat timer. Each job opens and releases the port and plays one cue | `bf`, `cues` |
| `gear/edgetx/yaml.rs` | Line-level YAML reader and editor for EdgeTX files (section 6.3). Never a generic YAML round-trip | none |
| `gear/edgetx/model.rs` | Typed views of a model file: header, timers, mixes, logical switches, special functions, switch warnings, telemetry sensors and screens, module settings | `edgetx/yaml` |
| `gear/edgetx/card.rs` | The SD card: `RADIO/radio.yml`, `MODELS/`, `SOUNDS/`, `SCRIPTS/`, `LOGS/`; identity (board, `semver`), the selected model, the radio clock check; `Card::plan` (edits to bytes, checks, diff; writes nothing), `write` (per-file temp + rename + `F_FULLFSYNC` + read-back, stop between files, timeouts), `release` (unmount under a timeout) | `edgetx/model` |
| `gear/edgetx/synth.rs` | The synthetic card generator: radio and model files in the 2.12 saved and 2.10 hand-edited layouts, CRLF or LF, made-up names | none |
| `gear/compat.rs` | The table of proven versions: EdgeTX boards and versions, Betaflight versions, ELRS targets, splash layouts, sim file versions. Data, reviewed per release | none |
| `gear/blobs.rs` | The content-addressed blob store: put, get, verify, garbage-collect (section 7.1). Built (WP4) | `store`, `xxhash-rust` |
| `gear/backup.rs` | Takes, lists, reads, diffs, retains and prunes snapshots; imports old backup folders. Built (WP4) | `blobs`, `bf`, `edgetx` |
| `gear/radiologs.rs` | Stores each radio log once, outside snapshots; feeds `flights`. Built (WP4) | `store` |
| `gear/health.rs` | The card check and repair (`diskutil verifyVolume`/`repairVolume`) and its log. Built (WP4) | `store` |
| `gear/changes.rs` | Staged changes: create, edit, order, status; builds an `ApplyPlan` with its digest | `store`, `bf/dump`, `edgetx` |
| `gear/apply.rs` | The one apply engine: guards, backup, write, read back, verify, record. Per-device writers below | `changes`, `backup` |
| `gear/apply/fc.rs`, `apply/card.rs`, `apply/sim.rs` | The writers for an FC (CLI), an SD card (files), a sim (files) | the above |
| `gear/switchmap.rs` | Joins an EdgeTX model with the FC's `aux` and `adjrange` lines into a per-switch table | `edgetx/model`, `bf/dump` |
| `gear/osd.rs` | OSD position encode and decode, element widths, grids, render, overlap and off-screen check | `bf/dump` |
| `gear/rates.rs` | Rate curves (Betaflight, Actual, Quick), sampling for the chart, conversion between types | none |
| `gear/sims/mod.rs` + `liftoff.rs`, `micro.rs`, `uncrashed.rs`, `zone.rs`, `velocidrone.rs` | One `Sim` adapter per game: find, read rate profiles, write, "running" check | `rates` |
| `gear/voice/` (`lines.rs`, `tts.rs`, `render.rs`, `packs.rs`) | Voice lines and spelling rules, TTS providers, the render cache and normalisation, pack index and install | `media` (ffmpeg), `store` |
| `gear/splash.rs` | Image to 1-bit 128x64 with threshold and preview; patch and decode a firmware image | `image` crate (PNG decode) |
| `gear/firmware/` (`check.rs`, `edgetx.rs`, `elrs.rs`) | Version checks; EdgeTX download, splash, DFU flash; ELRS options and flash | `splash`, `serial`, external `dfu-util`, `esptool` |
| `modules/` (`manifest.rs`, `fetch.rs`, `install.rs`, `run.rs`) | The module manager (7.10): pinned manifests, download, checksum, install, run as a subprocess, update check, removal. Used by `media` for ffmpeg too | `paths`, `settings` |
| `gear/dfu.rs` | USB DFU 1.1 with the STM32 DfuSe extensions, written from the USB DFU specification and ST's published DfuSe notes | `nusb` |
| `gear/flights.rs` | Flight analysis from radio logs (section 7.6) | `logs` (extended) |
| `gear/packs.rs` | Packs, pack types, charging sheet, pack history | `store`, `flights` |
| `disk.rs` (changed) | `format_card` takes the file system and the label from the source's `CardPolicy`; a card-prep plan without a session | existing |
| `logs.rs` (changed) | `LogRow` gains TPWR, RSNR, Curr, Capa, Bat%, FM and channel values; still tolerant of missing columns | existing |

**New crates:** `serialport` (serialport-rs, MPL-2.0: USB serial with VID/PID), `image`
(MIT/Apache: PNG decode for the splash), `similar` (Apache-2.0: line diffs for the apply
sheet), `nusb` (Apache-2.0/MIT: pure-Rust USB for DFU). ESP flashing runs `esptool` as a
separate process. QuadCam downloads it as a module on first need (7.10); the `.app` bundles
no GPL tool.

## 4. Data model and storage

### 4.1 Where things live

| Data | Location | Notes |
|---|---|---|
| Gear user data: devices, aircraft links, packs, pack types, voice choices, sim choices | `<gear>/gear.json` | One file, locked read-modify-write; unknown keys kept, as `settings.json` |
| Aircraft profiles | `settings.json` `profiles` (existing) | `Profile` gains optional gear links (below). Old files load unchanged |
| Snapshots | `<gear>/snapshots/<device-id>/<YYYY-MM-DDTHHMMSS>-<trigger>.json` | A manifest: path → hash, size, mtime. A few KB |
| Blobs | `<gear>/blobs/<xx>/<hash>` | Every file stored once by content (SD files, FC `diff all`/`dump all`, sim files). Plain bytes, uncompressed |
| Radio logs | `<gear>/logs/<radio-id>/<file name>.csv` | Each log stored once as a plain file, not part of any snapshot (section 7.1) |
| Card checks | `<gear>/health/<device-id>.jsonl` | One line per verify or repair (7.1) |
| Staged changes | `<gear>/changes/<YYYY-MM-DD>-<device-slug>-<n>/` | `change.json`, `before/`, `after/`, `apply.cli` or file diffs, `report.json`. The same shape as a hand-kept staging folder |
| Bench history | `<gear>/changes/` (applied ones) | The log of what was applied, verified or reverted |
| Downloads | `~/Library/Caches/app.quadcam/firmware/<product>/<version>/` | With a SHA-256 per file |
| Voice renders | `~/Library/Caches/app.quadcam/voice/raw/<provider>/<key>.pcm` | Raw takes; a re-render costs nothing |
| Installed voice packs | `<gear>/voices/<pack-id>/` | WAVs and `pack.json` |
| Blackbox pulls | `<gear>/blackbox/<device-id>/<YYYY-MM-DDTHHMMSS>.json` | One record per pull: the blob ref of the raw flash image (in `blobs/`), aircraft, day, method, used and total size, the logs found in the headers, whether the flash was erased (7.12). A blob a record names is never collected |
| Flight index | `<gear>/flights.json` | A rebuildable cache over the log store and folders the user adds |

`<gear>` is the `gearDir` setting. Every path derives from `paths.rs`, so a test with
`HOME` set to a temp folder touches nothing real.

### 4.2 Types (sketch)

```rust
pub struct Device {
    pub id: String,               // stable: FC MSP UID hash, radio card volume UUID, ELRS target+UID hash
    pub kind: DeviceKind,         // Fc, Radio, ElrsTx, ElrsRx, Goggles, DvrCard
    pub name: String,             // the user's name for it
    pub aircraft: Option<String>, // aircraft profile name
    pub identity: Identity,       // board, firmware, version, build, target
    pub last_seen: Option<DateTime<Utc>>,
    pub last_backup: Option<String>,
}

// Existing Profile (metadata.rs) gains optional links; serde(default) keeps 0.3.0 files loading.
pub struct ProfileGear {
    pub fc: Option<String>,           // device id
    pub radio: Option<String>,        // device id
    pub edgetx_model: Option<String>, // model file, e.g. "model01.yml"
    pub rx: Option<String>,           // device id
    pub pack_type: Option<String>,
}

pub struct Backup {
    pub id: String,
    pub device: String,
    pub trigger: Trigger,             // Connect, Manual, BeforeApply, BeforeFlash
    pub taken_at: DateTime<Utc>,
    pub identity: Identity,
    pub files: Vec<BackupFile>,       // path, size, xxh64
    pub pinned: bool,                 // the user's pin; apply and flash snapshots are always kept
}

pub struct StagedChange {
    pub id: String,
    pub device: String,
    pub title: String,
    pub status: ChangeStatus,         // Draft, Ready, Try, ReadFirst, Applied, Verified, Failed, Reverted, Discarded
    pub edits: Vec<Edit>,
    pub base_backup: String,          // the backup the "before" was read from
    pub editor: Editor,               // User or Agent (existing type)
    pub note: String,
    pub order: u32,
    pub history: Vec<ChangeEvent>,
}

pub enum Edit {
    FcLines { lines: Vec<String> },                       // raw CLI, checked against the dump
    FcSet { section: Section, name: String, value: String },
    FcAux { slot: u8, mode: u16, aux: u8, start: u16, end: u16 },
    FcAdjrange { slot: u8, range_aux: u8, start: u16, end: u16, function: u8, select_aux: u8 },
    OsdElement { element: String, x: u8, y: u8, profiles: Vec<u8> },
    RateProfile { index: u8, rates: Rates },
    Model { file: String, op: ModelOp },                  // timers, mixes, LS, SF, screens, checklist
    CardFiles { put: Vec<CardFile>, delete: Vec<String> },// sounds, scripts, splash assets
    Restore { backup: String, paths: Vec<String> },
}

pub struct ApplyPlan {
    pub change: String,
    pub device: Identity,
    pub checks: Vec<Check>,           // name, ok, reason
    pub diff: Vec<DiffItem>,          // CLI lines or file line diffs
    pub digest: String,               // xxh64 of identity + before state + edits
}
```

### 4.3 Relation to the library

- The aircraft profile stays the join point. Clips name a profile; the profile names its FC,
  radio, model and pack type.
- The log store (7.1) holds every radio log once. The flight index reads it, and the
  existing log matching can use it instead of a mounted radio.
- A flight matched to a library clip shows its pack and analysis in the clip's Details tab.
  The clip file gets no new QuickTime item in 1.0 (open question 7).

## 5. Surfaces: `api` rows, CLI, MCP

### 5.1 `api` rows (`api/gear.rs`)

The rows join the one `api!` table through the `with_gear_rows!` macro in `api/gear.rs`, so
Gear packages add rows without editing `api/mod.rs`. Each new row also gets its typed command
in `specta_builder` (`lib.rs`).

| Row | Params → Result | Writes |
|---|---|---|
| `gear_status` | – → `GearStatus` (connected devices, staged count, sims out of date) | no |
| `gear_devices` | – → `Vec<Device>` | no |
| `gear_device_save` | `DeviceSaveParams` → `Device` | gear.json |
| `gear_device_forget` | `IdParams` → `Device` (backups stay) | gear.json |
| `gear_fc_identify` | `FcPortParams { port }` → `FcJob<FcInfo>` (identity, id, read-only reason, notes) | no (MSP read) |
| `gear_fc_read` | `FcReadParams { port, commands }` → `FcJob<FcRead>` (read-only CLI commands; the FC reboots) | no (CLI read) |
| `gear_board_notes` | `BoardNotesParams { board, version }` → `Vec<BoardNote>` | no |
| `gear_usb_timers` | – → `Vec<UsbTimer>` | no |
| `gear_poll_pause` | `PollPauseParams { port, paused }` → `Vec<String>` (the paused ports; in memory until quit) | no |
| `gear_card` | `CardParams { mount or device, model }` → `GearCard` (card view, selected model's aircraft, `radio_usb`) | no |
| `gear_card_preview` | `CardPreviewParams { mount or device, edits }` → `CardPreview` (checks, diff, files, bytes, ETA) | no |
| `gear_backup` | `BackupParams { device }` → `Backup` | device read, gear folder |
| `gear_blackbox_pull` | `BlackboxPullParams { port, keep, mode, force }` → `FcJob<BlackboxPullResult>` | device read; **device erase** only when `gearEraseBlackbox` is on and the pull verified; blobs, gear folder |
| `gear_blackbox` | `BlackboxFilter { device }` → `Vec<BlackboxEntry>` (a pull and its guessed flights) | no |
| `gear_blackbox_export` | `BlackboxExportParams { id, to, split }` → `BlackboxExported` | files in `to` |
| `gear_blackbox_erase` | `BlackboxEraseParams { port, confirm }` → `FcJob<BlackboxErased>` | **device erase** |
| `gear_backups` | `BackupFilter` → `Vec<Backup>` | no |
| `gear_backup_read` | `BackupReadParams { id, path }` → `BackupContent` | no |
| `gear_backup_diff` | `BackupDiffParams { a, b, path }` → `Vec<DiffItem>` | no |
| `gear_backup_pin` | `BackupPinParams { id, pinned }` → `BackupSummary` | gear folder |
| `gear_card_check` / `gear_card_checks` | `CardCheckParams { device or mount }` / `{ device }` → `CardCheck` / `Vec<CardCheck>` | health log |
| `gear_card_repair` | `CardRepairParams { check, confirm }` → `RepairResult` | **card file system** |
| `gear_stop` | `StopParams { handle }` → `bool` | no |
| `gear_storage` | – → `StorageView` (totals, per device, snapshot counts) | no |
| `gear_prune` | `PruneParams { dry_run }` → `PruneReport` | gear folder |
| `gear_export` | `ExportParams { device or snapshot, to }` → `ExportReport` | a folder the user picked |
| `gear_import_backups` | `ImportBackupsParams { folder, device, dry_run }` → `ImportBackupsReport` | gear folder |
| `gear_switch_map` | `AircraftParams { aircraft, live }` → `SwitchMap` | no |
| `gear_osd` | `OsdParams { paths or device, grid, staged }` → `OsdView` (`staged`: the device's backup with its staged OSD edits on top) | no |
| `gear_osd_edit` | `OsdEditParams { device, moves, copy }` → `StagedChange` (joins the one "OSD layout" change) | no |
| `gear_rates` | `RatesParams { device or backup or change }` → `RatesView` | no |
| `gear_sims` | – → `Vec<SimStatus>` | no |
| `gear_changes` | `ChangeFilter` → `Vec<StagedChange>` | no |
| `gear_change_stage` | `StageParams { device, title, edits }` → `StagedChange` | gear folder |
| `gear_change_update` | `ChangeUpdateParams { id, edits, status, note, order }` → `StagedChange` | gear folder |
| `gear_change_discard` | `IdParams` → `StagedChange` | gear folder |
| `gear_restore_stage` | `RestoreParams { backup, paths }` → `StagedChange` | gear folder |
| `gear_apply_plan` | `IdParams` → `ApplyPlan` (runs every guard, writes nothing) | no |
| `gear_apply` | `ApplyRequest { id, digest, confirm }` → `ApplyReport` | **device** |
| `gear_sim_sync_plan` | `SimSyncParams { sims, rates }` → `ApplyPlan` | no |
| `gear_sim_sync` | `SimSyncRequest { digest, confirm }` → `ApplyReport` | **sim files** |
| `gear_voice` | `VoiceParams` → `VoiceView` (lines, packs installed and available, card state) | no |
| `gear_voice_edit` | `VoiceEditParams { line, text, pack }` → `VoiceLine` | gear.json |
| `gear_voice_render` | `VoiceRenderParams { voice, lines, dry_run }` → `RenderReport` | cache; network |
| `gear_voice_pack_install` | `PackInstallParams { pack }` → `VoicePack` | gear folder; network |
| `gear_voice_choose` | `VoiceChooseParams { radio, pack, keep_overrides }` → `StagedChange` | gear folder |
| `gear_splash` | `SplashParams { image, threshold, invert, board }` → `SplashPreview` | cache |
| `gear_firmware_check` | `FirmwareCheckParams { devices }` → `Vec<FirmwareStatus>` | network |
| `gear_flash_plan` | `FlashParams { device, product, version, splash, options }` → `ApplyPlan` | downloads |
| `gear_flash` | `FlashRequest { digest, confirm }` → `ApplyReport` | **device** |
| `card_prep_plan` | `CardPrepParams { mount, label }` → `FormatPlan` | no |
| `card_prep` | `FormatRequest` → `FormatPlan` | **card erase** |
| `gear_flights` | `FlightFilter { day, aircraft, pack, place, logs }` → `FlightsView` (flights, days, sources, range trend per place) | flights cache |
| `gear_flight_set` | `FlightSetParams { flight, pack, place }` → `FlightReport` | gear.json |
| `gear_packs` | `PacksParams { target_v }` → `PacksView` (packs, types, charging sheet, history) | no |
| `gear_flight_folders` | `FlightFoldersParams { add, remove }` → `Vec<PathBuf>` | gear.json |
| `gear_pack_type_save` / `gear_pack_type_delete` / `gear_pack_notes` | `PackType` / `NameParams` / `NotesParams` → the saved value | gear.json |
| `gear_session_report` | `ReportParams { day }` → `SessionReport` (with `markdown`) | no |
| `gear_preflight` | – → `Preflight` (rows: pass, warn, unknown) | no |
| `gear_crashes` / `gear_crash_save` / `gear_crash_delete` | `CrashFilter` / `CrashSaveParams` / `IdParams` → `Crash` | gear.json |
| `modules` | – → `Vec<ModuleStatus>` (installed, pinned, newest, license) | no |
| `module_install` | `ModuleParams { name, confirm }` → `ModuleStatus` | modules folder; network |
| `module_remove` | `NameParams` → `ModuleStatus` | modules folder |
| `modules_check` | – → `Vec<ModuleStatus>` | network |
| `gear_pack_save` / `gear_pack_delete` | `PackSaveParams` / `NameParams` → `Pack` | gear.json |

Events (`api/events.rs`): `DeviceChanged` (connect, disconnect, backup state), `ApplyProgress`,
`RenderProgress`, `FlashProgress`.

### 5.2 CLI

`quadcam-cli gear <command>`, with `--json` and the existing exit codes (3 = refused by a
guard, 4 = no device). The subcommands live in `bin/quadcam-cli/gear/*.rs`, one file per area, so
work packages do not edit one shared file.

```bash
quadcam-cli --json gear devices
quadcam-cli --json gear fc identify|read|check|notes|usb          # WP2 (read: --cmd, --out STEM)
quadcam-cli --json gear blackbox pull|list|export|erase           # BB (7.12; pull: --keep, --mode, --force; erase: --yes)
quadcam-cli --json gear card [--mount M | --device ID] [--model model01.yml]
quadcam-cli --json gear card preview --edits edits.json             # checks and diff; writes nothing
quadcam-cli --json gear backup --device <id>|--port /dev/cu.usbmodemX|--mount /Volumes/RADIO
quadcam-cli --json gear backups [--device <id>]
quadcam-cli --json gear backup show <backup> [PATH]
quadcam-cli --json gear backup diff <a> <b> [PATH]
quadcam-cli --json gear storage [--prune [--dry-run]] [--export <device|snapshot> DIR]
quadcam-cli --json gear import-backups FOLDER [--device <id>] [--dry-run]
quadcam-cli --json gear map --aircraft NAME [--live]
quadcam-cli --json gear osd <FILE ...|device> [--grid PAL|NTSC|HD|WxH] [--text]
quadcam-cli --json gear rates --device <id>
quadcam-cli --json gear changes [--device <id>] [--status ready]
quadcam-cli --json gear stage --device <id> --title T --cli FILE      # raw CLI lines
quadcam-cli --json gear stage --device <id> --set name=value [--profile N|--rateprofile N]
quadcam-cli --json gear stage --device <id> --model model01.yml --op FILE.json
quadcam-cli --json gear restore <backup> [--path P]
quadcam-cli --json gear apply <change> --plan                          # guards, diff, digest
quadcam-cli --json gear apply <change> --digest D --yes
quadcam-cli --json gear sims [--sync --rates <device> --digest D --yes]
quadcam-cli --json gear voice [render --voice V [--lines a,b] [--dry-run] | choose --radio R --pack P]
quadcam-cli --json gear voice build-pack --voice V --out DIR          # maintainer tool (7.4)
quadcam-cli --json gear splash IMAGE [--threshold 128] [--invert] --board pocket --out preview.png
quadcam-cli --json gear firmware [--check] [--plan --device <id> --version X [--splash IMAGE]]
quadcam-cli --json gear flash --digest D --yes
quadcam-cli --json gear card-prep --plan --mount /Volumes/CARD [--label NAME]
quadcam-cli --json gear card-prep --device /dev/diskN --volume-uuid U --yes
quadcam-cli --json gear flights [--day 2026-10-07] [--aircraft NAME] [--pack LABEL]
quadcam-cli --json gear packs [save LABEL --type T [--charged] | delete LABEL | type save T | notes TEXT]
quadcam-cli --json gear flights set <flight> --pack LABEL | folders --add DIR
quadcam-cli gear report [--day D] --markdown
quadcam-cli --json gear preflight
quadcam-cli --json gear crashes [--clip ID save --time S --broke T --parts a,b | delete ID]
quadcam-cli --json modules [check | install NAME --yes | remove NAME]
```

### 5.3 MCP: three new tools

The server has 16 tools. Gear adds three, split by what they can change. An agent harness
can then allow the read tool freely and gate the other two.

| Tool | Changes | Actions |
|---|---|---|
| `quadcam_gear` | Nothing | `status`, `devices`, `fc_identify`, `board_notes`, `usb_timers`, `card`, `card_preview`, `storage`, `backups`, `backup_read`, `backup_diff`, `blackbox`, `switch_map`, `osd`, `rates`, `sims`, `changes`, `apply_plan`, `voice`, `firmware_check`, `flights`, `packs`, `session_report`, `preflight`, `crashes` |
| `quadcam_gear_edit` | QuadCam's own data only: never a device, a sim or a card | `device_save`, `device_forget`, `fc_read` (a CLI read; the FC reboots), `stage`, `update`, `discard`, `restore_stage`, `voice_edit`, `voice_render`, `voice_choose`, `pack_save`, `pack_delete`, `pack_type_save`, `pack_type_delete`, `pack_notes`, `flight_set`, `flight_folders`, `crash_save`, `crash_delete`, `backup` (a read of the device; writes only to the gear folder), `blackbox_export` (files in a folder), `import_backups`, `prune`, `export` |
| `quadcam_gear_apply` | A device, a sim or the radio firmware | `apply`, `sim_sync`, `flash`. Each needs the `digest` from a plan and `confirm=true`. `blackbox_pull` and `blackbox_erase` need `confirm=true` only: a pull may erase the FC's flash when the person's `gearEraseBlackbox` setting is on, so it sits in the tool a harness gates, though it has no plan or sheet (like `card_repair`) |

Modules are setup, so `quadcam_settings` gains the actions `modules`, `module_install`
(needs `confirm=true`, after the agent shows the user the license) and `module_remove`.

Card prep extends the existing `quadcam_format_card` (`prep=true`, `mount`, `label`) rather
than a new tool: every erase stays behind one tool with one confirm pattern.

The Gear tools, their argument types and their handlers live in `mcp/gear.rs`, not in
`params.rs` and `server.rs`, so Gear packages edit one file.

**Why not fewer:** one Gear tool would mix reads with device writes, so a harness could not
allow one without the other. **Why not more:** each tool takes an `action` and a typed
params struct in `mcp/params.rs`, as `quadcam_library_files` does today. New Gear features
add an action, not a tool.

`quadcam_gear_apply`, like `quadcam_format_card`: with the app running, the person must
also click **Apply** in the app's apply sheet; Cancel, closing it or 3 minutes refuses.
`voice_render` with a paid provider returns the character count and the quota first and
renders only with `confirm=true`.

## 6. Device protocol notes

### 6.1 Betaflight CLI (the write path)

Proven by a hand-written serial runner over many bench sessions:

| Item | Rule |
|---|---|
| Port | USB CDC ACM (`/dev/cu.usbmodem*`), 115200 baud. Several candidates: ask; never guess |
| Enter | Send `#`. Wait for the prompt: the buffer ends with `\r\n# ` and the line is quiet 300 ms |
| Command | Send one line, wait for the prompt (5 s; 30 s for `dump all`, extended while bytes arrive) |
| Error | A reply with `###ERROR`, `Invalid` or `Parse error` |
| `save` | Writes and reboots. The port vanishes; wait up to 30 s for it to return |
| `exit` | Leaves the CLI and reboots. Unsaved changes are discarded |
| Port return | After `save` or `exit` the port is gone for several seconds; any next step waits |
| Plug order | The FC enumerates only when USB is plugged in before the battery. The UI says so when no port appears |

**Backup:** `version`, `status`, `diff all`, `dump all`, then `exit`.

**Built (WP2):** `bf/cli.rs` (`CliSession`, `run_lines`, `Timing`), `bf::read` (read-only
commands only), `bf::run`. `run_lines` refuses `save`, `exit`, `defaults`, `bl`, `dfu`,
`flash_erase`, `msc`, `serialpassthrough` and `batch` in a change: the engine saves and exits
itself. Before the first write, `bf::run` reads `version` and `mcu_id` in the CLI, re-runs the
`compat` guard on what the FC says now and checks the planned device id (`device_changed`).
Proven pairs are keyed by board and build (`compat.rs`); every other FC is read-only with the
reason. Dumps from 2026.6 add `battery_profile N` sections (`Section::BatteryProfile`).

**Apply:** backup, enter CLI, send each line, stop at the first error. On any error: no
`save`, `exit` (discards everything). Otherwise `save`, wait for the port, enter CLI,
`dump all`, `exit`. Then verify (below).

**Verify** reads `dump all`, not `diff all`. A `set` equal to its default never shows in
`diff all`, so a diff-based check cannot prove it. `bf/dump.rs` parses with section context
(`profile N`, `rateprofile N`), so a line shared by two rate profiles is checked in each.
Every edit must match; every line the change says must not appear must be absent.

**Validation at stage time:** a `set` name must exist in the device's latest `dump all`.
With the FC connected, `get <name>` reads the allowed range or values, and the plan checks
the new value against it.

### 6.2 MSP (read-only)

MSP serves identity and the live view. It never writes configuration in 1.0. The one MSP message that changes the FC is the blackbox flash erase (`MSP_DATAFLASH_ERASE`, section 7.12), and it runs only after a verified pull.

| Need | Messages |
|---|---|
| Identity without entering the CLI (no reboot) | `MSP_API_VERSION`, `MSP_FC_VARIANT`, `MSP_FC_VERSION`, `MSP_BOARD_INFO`, `MSP_BUILD_INFO`, `MSP_UID` |
| Live switch map | `MSP_RC` (channel values), `MSP_BOXIDS` + `MSP_MODE_RANGES` + `MSP_ADJUSTMENT_RANGES`, the active-modes flags in `MSP_STATUS_EX` |
| Live overview | `MSP_ANALOG`, `MSP_BATTERY_STATE` |
| Blackbox flash | `MSP_DATAFLASH_SUMMARY` (70), `MSP_DATAFLASH_READ` (71, v2, compression flag off), `MSP_DATAFLASH_ERASE` (72) (7.12) |

- v1 frames (`$M<`, XOR checksum) for the classic messages, v2 (`$X<`, CRC8 DVB-S2) where
  needed. Message ids come from the public MSP protocol description and are checked against
  a recorded transcript per supported version.
- CLI and MSP do not run at once. Entering the CLI ends the MSP session; leaving it reboots.
  `gear/bf` holds one state machine per port: Idle, Msp, Cli, Rebooting.
- The device id is a hash of `MSP_UID`. The raw UID never leaves the machine and never goes
  into a fixture. `MSP_UID` and the CLI's `mcu_id` are the same value (three 32-bit words as
  hex), so an id read over MSP and one read from a dump agree. An FC with no UID gets a hash of
  its board name and USB serial number.
- Not yet checked on hardware: the shape of `MSP_FC_VERSION` on calendar-versioned builds
  (2025.12 and later). The parser takes a trailing version string when there is one, else
  `major.minor.patch`; the write guard reads the CLI's `version` line, so it never depends on it.
- The battery probe is `MSP_ANALOG` (0.01 V at byte 7, else 0.1 V at byte 0).

**What each feature needs:**

| Feature | CLI | MSP |
|---|---|---|
| Backup, restore | yes | – |
| Staged FC changes (rates, OSD, battery, modes, adjustments, launch control, any `set`) | yes | – |
| Verify | yes (`dump all`) | – |
| Identity, version guard | `version` as a fallback | yes |
| Switch map, live | – | yes |
| Blackbox pull and erase | `msc` (the optional USB disk path, unproven) | yes |
| ELRS RX flash | `serialpassthrough` | – |

### 6.3 EdgeTX SD card and YAML

Proven by a hand-written card patcher on EdgeTX 2.12 (128x64 B&W radio), revisions 1 to 9.
QuadCam implements these rules from the file format; it copies no EdgeTX code.

**Reading and writing**

- Edit lines, not a parsed tree. A generic YAML library reorders keys, changes quoting and
  drops layout, and EdgeTX files differ by version and by who wrote them.
- Keep every untouched line byte for byte, CRLF included.
- Read the field order and the quoting style from the file's own first entry (2.10-style
  hand edits put `weight` first and leave `srcRaw` unquoted; 2.12 saves put `destCh` first,
  add `delayPrec`/`speedPrec` and quote `srcRaw`). New entries copy that layout.
- Parse a block into typed items only when every line is understood. A line the reader does
  not know stops the edit with the file and the line in the reason.
- Before any edit, parse and render the unchanged file. If the bytes differ, refuse.
- After an edit, set `checksum: 0` on a file that has a checksum line (EdgeTX accepts 0 or no
  line).
- Write a file only when its bytes change (FAT writes are slow on macOS). Write to a
  temporary name, rename, read back, compare bytes, `F_FULLFSYNC`.

**Encodings that bit before**

| Item | Rule |
|---|---|
| Logical switches | `L1` is index 0. EdgeTX omits empty switches. `delay` and `duration` are in 0.1 s |
| Telemetry sources | `tele(N)` is slot N of `telemetrySensors`. Find the slot by the sensor's label; refuse when the label is missing ("discover sensors first") |
| Switch sources | Names `SA0`..`SA2` (index × 3 + position); trims by name (`TrimRudRight`) |
| Stick sources in a comparison | -100..100 |
| Switch warnings | Write the 2.12 `switchWarning:` list. Never write the legacy `switchWarningState:` line; replace it if found (2.12.4 mis-decodes it) |
| Special functions | 64 at most. `SET_SCREEN` repeats while its switch is on, so it belongs on an edge pulse |
| `!1x` | "once, not at power-on" for `PLAY_TRACK` |
| Track names | 8 characters at most (file stem in `SOUNDS/<lang>/`) |
| Telemetry script names | 6 characters at most |
| Checklist | `MODELS/<model name>.txt`; EdgeTX looks with spaces kept, then with `_`. On 128x64, 20 characters per line, `=` for a tick box. Needs `displayChecklist` and `checklistInteractive` |
| `header.modelId` | Index 0 is the internal module. EdgeTX omits zero, so ID 0 means no block |
| `radio.yml` | `currModel`, `hapticMode` (`mode_alarms` drops special-function haptics) |
| Model identity | Check the model's `header.name` before an edit; a different name refuses ("wrong card?") |
| Timer swap | Swapping two timers trades every field, the stored value included, and every `TmrN` reference outside the timers block |

**Version guard:** `radio.yml` carries `board` and `semver`. Only pairs in `compat.rs` are
writable. Others are read-only, with the reason "EdgeTX X on board Y is not proven; QuadCam
reads it but does not write it."

**Built (WP3).** `gear/edgetx/` implements these rules:

- `yaml.rs` reads bytes as Latin-1 (one `char` per byte), keeps each line's own ending, and
  classifies every line as `key: value`, a block header (`key: ` or `N:`) or a ` -` item. A
  line outside that subset, a tab, or an indent that fits no block refuses (`shape_unknown`,
  with the file and line); a file that does not render back to its bytes refuses
  (`round_trip`).
- `model.rs` holds the typed view and `ModelOp`: `rename`, `set_model_id`, `set_flags`,
  `set_checklist` (on/off: `displayChecklist` and `checklistInteractive`), `set_mixes`,
  `set_logical_switch`, `special_functions` (remove what a change owns, add what is absent),
  `move_special_function`, `set_timer` (never the stored `value`), `remove_timer`,
  `swap_timers`, `set_switch_warnings`, `set_screen`. `{Label}` in a definition becomes the
  sensor's slot.
- Card edits are `Edit` variants: `model` (with the expected header name), `radio`
  (`set_scalar`; `select_model` is the only way to change `currModel`), `checklist`,
  `model_copy` (timer values 0, no model id), `model_delete` (never the selected model).
  `Card::plan` runs the checks in order: known version, shape understood (or values in range,
  model identity), selected model kept. It adds `.metadata_never_index` (and, when the caller
  passes one, the `.quadcam-id` marker) when it writes anything. Ops are idempotent: the same
  edits on the result plan no files.
- `card::write` re-reads every file and refuses a change since the plan (`before_mismatch`),
  hands every touched file to a backup callback (a failure is `no_backup`, nothing written),
  then writes one file at a time: a temporary name, `F_FULLFSYNC`, a rename, a folder sync,
  a read-back, and the removal of any `._<name>` AppleDouble file beside it. A read-back
  mismatch writes the old bytes back at once. A stop request takes effect between files
  (progress shows "stopping"); a file's write is never cut short. Each write runs under a
  timeout (a base plus its size at a floor speed: 30 s + 1 s per 100 KB over the radio's
  USB); a timeout says "a reboot may be needed" and leaves the write's thread to finish.
- `card::release` runs `diskutil unmountDisk` under a timeout. `Core::gear_release_card`
  calls it (through `Env.unmount`) and only then plays "safe to unplug"; a failed or stuck
  unmount plays "failed" instead.
- Fail-safes: a process started by cargo writes no card under `/Volumes` unless
  `QUADCAM_CARD_WRITE=real`, and unmounts nothing unless `QUADCAM_SERIAL=real`.
- Read-only extras for the UI: `CardView.selected_model` and `selected_name` with
  `GearCard.selected_aircraft` (the profile whose `gear.edgetx_model` is that file or whose
  `edgetx_models` holds its name), for "Radio is set to X, but the quad is Y"; and
  `CardView.clock` from log names: a newest log before 2020 means the clock reset, an older
  2000-01-01 log means it reset once, a log after today means it runs ahead.

**Built (WP9 model editors):** `gear/edgetx/editors.rs` adds the editors' `ModelOp`s to WP3's:
`set_screen_values` (up to 4 lines of 3 sources; a `{Label}` source becomes `tele(N)`),
`set_logging` (the `LOGS` special function, `def` `"<period in 0.1 s>,1"`, one at most),
`set_sensor_logs` (a sensor's `logs` line), `set_rf_alarms` (`rfAlarms`, critical not over
warning), `set_callout` and `remove_callout`. `set_timer` now checks each field's range. A callout is a
`PLAY_TRACK` special function owned by its track name: `set_callout` replaces the function with
that track, keeps its place, and edits the logical switch it made (a switch anything else
mentions stays; a new one takes the first free). A sensor value is typed as shown (`3.5`) and
stored at the sensor's `prec`; `FUNC_VNEG` is "below", `FUNC_VPOS` "above". `editor_view` reads all of it
back. `Core::gear_model` (`core/model_edit.rs`) reads a radio's model from the mounted card, else the
latest backup, with the device's staged model edits on top; `Core::gear_model_edit` joins ops and
checklist text into the one open "Model edits" change: `editors::merge_ops` keeps the last op
per setting (`op_key`; timer fields and sensor logs merge), an edit that leaves the model as
the radio has it drops out, an empty change is discarded, and a refusal stages nothing.
Staging a checklist turns the checklist on. Rows `gear_model` and `gear_model_edit`; CLI
`gear model`, `gear model-edit`; MCP `quadcam_gear model`, `quadcam_gear_edit model_edit`; UI
Models and Checklists segments (`views/Gear/Models/`). Acceptance: `tests/model_editors.rs` (each
op in both file layouts, ownership, idempotence, the limits), `tests/model_edit.rs` (read from card
and backup, merge, drop-out, apply with read-back on the synthetic card, row and MCP),
`e2e/model-edit.spec.ts`.
Deviations: the design's "telemetry screens" editor covers value and script screens, not bars or
the other types (they show and cannot be edited). Checked only against the shape of real model
files, not on a radio: the `LOGS` encoding, 4 lines by 3 sources, the timer mode names, and the
99-line checklist cap (QuadCam's own).

**Device notes (RadioMaster Pocket, EdgeTX 2.12.4, measured 2026-10-07):**

| Mode | USB id | Strings | Notes |
|---|---|---|---|
| USB Storage | `0483:5720`, `bcdDevice` `0x0212` (the firmware's major.minor) | vendor `OpenTX`, product `<Radio> Mass Storage`, serial `00000000001B` | The serial is the ST USB library's default, the same on every radio: not an id. Disk: external, physical, FAT32, `MediaName` `<Radio>Radio`. `ioreg -a -r -c IOUSBHostDevice -l` ties the USB device to its BSD disk (`detect::parse_ioreg_usb`) |
| USB Serial (VCP) | `0483:5740` | product `<Radio> Serial Port`, serial as above | With `serialPort: VCP: mode: CLI` the EdgeTX CLI answers at 115200, prompt `>`: `ls`, `play`, `reboot`, `set`, `serialpassthrough` (the ELRS passthrough), `beep`, `readsd`, `testsd`. `read` is a speed test only: the CLI cannot move file contents, so writes stay on USB Storage or a reader. Its product names no firmware, so `detect::classify_port` takes the vendor or a radio maker in the product as a radio, not an FC |
| DFU (powered off, USB) | `0483:df11`, `bcdDevice` `0x2200` | product `STM32  BOOTLOADER` (two spaces), vendor `STMicroelectronics`, serial from the chip's unique id | A stable hardware id. Storage and serial modes do not expose it, so the person links the DFU device to the radio once (WP10). Alt 0 `@Internal Flash /0x08000000/04*016Kg,01*064Kg,07*128Kg` (1 MB F4), alt 1 option bytes `0x1FFFC000`, alt 2 OTP `0x1FFF7800`, alt 3 device feature |

- Speeds over the radio's USB: write 0.30 MB/s (20 MB in about 65 s), read about 0.5 MB/s,
  20 small files in 5.5 s. A 37 MB voice pack takes about 2 minutes; reading a 70 MB card 2-3
  minutes. Backups over the radio must skip unchanged files by size and mtime before hashing
  (WP4); big writes need progress, an ETA and Cancel (`WriteProgress`, `WriteOptions.stop`).
- `diskutil verifyVolume` on the card needs no admin, takes about 30 s over the radio's USB,
  and mounts the volume again after.
- macOS leaves AppleDouble files (`._model00.yml`), `.fseventsd` and
  `.metadata_never_index` on a FAT card it writes. QuadCam removes `._` files beside what it
  writes and adds `.metadata_never_index`.
- Pulling the radio mid-write once wedged `diskarbitrationd` until a reboot (2026-09-27).
- Bootloader route: hold both horizontal trims inward while powering on; the bootloader
  shows the SD card over USB even when the firmware stops at an error.

**Ownership:** QuadCam does not add marker keys to EdgeTX files. Which special functions and
logical switches a change owns is recorded in the change, so a later change can replace them.

### 6.4 ELRS

| Path | When | How |
|---|---|---|
| TX through EdgeTX passthrough | Internal ESP32 modules | The radio's USB-VCP set to CLI and USB Serial chosen at plug-in; QuadCam starts the passthrough over serial, then runs `esptool` (ESP32, 460800) |
| RX through Betaflight passthrough | Serial ESP82xx receivers on an FC UART | FC not in CLI mode first; `serialpassthrough <uart> 420000`, then `esptool` (ESP8266) |
| WiFi | Any ESP target | Out of scope for 1.0: needs joining the device's access point |

- **Firmware source:** the ELRS artifactory index (`index.json` maps a tag to a build;
  `firmware.zip` holds the per-target binaries). Each download is checked against its hash.
- **Options:** the binding phrase (hashed into the 6-byte UID), the regulatory domain, and
  the auto-WiFi delay are written into the binary's options block. QuadCam implements this
  from the published format, then reads it back and checks the UID. The phrase is user data
  (`gear.json`, in `SECRET_KEYS`-style redaction for CLI and MCP reads).
- **Version read:** a CRSF device-info ping over the same passthrough gives name and version.
  Unverified on hardware; until then the version is user-entered (open question 9).
- **Order trap:** TX and RX on different major versions cannot link. The flash plan warns
  when a flash would leave a linked pair on different majors, and suggests RX first.

### 6.5 EdgeTX firmware and DFU

- Releases come from the EdgeTX GitHub releases (`edgetx-firmware-vX.Y.Z.zip`, one binary per
  board). The plan names the binary for the device's board.
- Radio off, then USB: the STM32 ROM DFU device (`0483:df11`). Exactly one must be present.
- Write the full image (bootloader included) at `0x08000000`, then leave DFU (the same
  operation as `dfu-util -a 0 -s 0x08000000:leave`). `gear/dfu.rs` does it natively: there is
  no official macOS `dfu-util` binary to download, and DFU is a published USB class. It erases
  the pages it writes, writes, reads back and compares before it leaves.
- Size guard per board (for the B&W Pocket: 400-1000 KB), board name check in the binary.

### 6.6 Sim file formats

Every sim adapter refuses while the game runs (process check by bundle id), and backs up the
file before it writes.

| Sim | File | Format | Rates |
|---|---|---|---|
| Liftoff | `~/Library/Application Support/LuGus Studios/Liftoff/Saves/Player/UserData.xml` | XML; each rate profile its own `<rateProfiles>` sibling: `<name>`, then `<rates xsi:type="BetaFlight">` with `<Roll>`, `<Pitch>`, `<Yaw>`, each `<Rate>` (RC rate), `<Expo>`, `<SuperExpo>` (super rate) | Betaflight style, the CLI's whole numbers (127, not 1.27), no throttle curve |
| Liftoff: Micro Drones | Inside the game's app bundle: `Contents/Saves/Player/UserData.xml` | The same XML; one `<rateProfiles>` wraps a `<FlightRatesProfile>` per profile, each with `<name>` and `<rates>` | As Liftoff |
| Uncrashed | `~/Library/Application Support/Uncrashed/<id>/rates/<NAME>.sav` | Unreal GVAS. After `FloatProperty\0` and one byte, an i32 count (12) and 12 little-endian f32 | Per axis (roll, pitch, yaw): super, RC rate, expo, as fractions; then rates type (0 = Betaflight), throttle mid, throttle expo. The name is the file name. An empty profile has no floats |
| The Zone | `~/Library/Application Support/Godot/app_userdata/The Zone/settings.cfg` | Godot config; each `[rate_profile_N]` holds one dictionary, `rates={ "roll": Vector3(…), "pitch": …, "yaw": …, "type": "betaflight" }`, one entry per line | Each axis `Vector3(RC rate, super rate, expo)` as fractions |
| Velocidrone | Unknown | – | Adapter ships disabled until a sample save exists (open question 1) |

**Checked against a real install (WP8 sync, 2026-10-09).** The WP8 read half inferred Liftoff's
and The Zone's shapes from this section's earlier text; the real files differ, and the
adapters now follow the files. The table above is the real shape. Confirmed: Liftoff,
Micro Drones, Uncrashed and The Zone, each read from a real player's files (reading only; the
tests keep synthetic fixtures of the same shapes). Still inferred: the XML `xsi:type` and the
Zone `type` of the other rate models (those profiles are listed, not read), the Uncrashed
rates-type float for other models, and any file shape of a game version other than the one
checked. A written file has been seen loading in the game only for Uncrashed (the reference
script `uncrashed_rates.py` writes the same bytes); the other three sims' plans carry an
"Unverified" warning until a write is flown.

- Sims take Betaflight-style rates. An FC on Actual or Quick rates is converted by a
  least-squares fit of the Betaflight curve; the preview shows both curves and the largest
  difference in degrees per second.
- Only Uncrashed has a throttle curve. The others get rates only, and the sync report says so.
- Writing inside a game's app bundle (Micro Drones) is what the game itself does. The adapter
  checks the folder is writable and says so if it is not.

## 7. Feature notes

### 7.1 Backups and the backup store

**What a backup holds**

- **Radio:** every file on the card except `LOGS/` (`MODELS/`, `RADIO/`, `SCRIPTS/`,
  `SOUNDS/`, `FIRMWARE/` and the rest).
- **FC:** `version`, `status`, `diff all`, `dump all`. Taken through the CLI, so the FC
  reboots after (open question 4).
- **Goggles and DVR cards:** the card's identity and file system. Clips belong to the
  library. Non-clip files (settings, `MISC/`) are copied when the source's policy lists them.
- **Sims:** the rate file before each sync.

**Sizes today (hand-made backups):** a full radio-card copy is 40-72 MB, almost all of it
sound files that repeat in every copy. An FC `diff all` and `dump all` pair is 100-300 KB.
Radio logs are CSV files that only grow. A store that copies whole folders grows by a card
per plug-in; Gear must not.

**Store options**

| | (a) Git repo per device, through gitoxide | (b) Content-addressed blobs and manifests |
|---|---|---|
| Size of a new snapshot | Changed blobs only; packfile deltas make YAML and dump changes almost free | Changed files only, whole. A changed model file is a few KB; a changed `dump all` is about 200 KB |
| Size over a year | Smallest, but only after a repack. gitoxide writes loose objects; delta packing needs the `git` binary (`git gc`), which is another external tool | About 200 KB per FC change and a few KB per radio change. Sound files once. Tens of MB for a year of weekly sessions |
| Retention | Keep all history; pruning means rewriting history, which git makes hard | Drop a manifest, then collect unreferenced blobs. Simple and exact |
| Diff view | `git diff` for free | `similar` on two blobs; the same view the apply sheet needs anyway |
| Restore | Checkout of a tree | Copy blobs named by the manifest |
| Robustness | A repo can be left locked or half-written by a crash; recovery needs git knowledge | Write blob to a temp name, fsync, rename; write the manifest last. A crash leaves at most an unreferenced blob, which the next collection removes |
| Inspect without QuadCam | Plain `git log`, `git show` | Manifests are JSON; blobs are plain files named by hash |
| Dependency weight | gitoxide is large (many crates, build time); libgit2 brings a C library and GPL-2 with a linking exception, which the license review would have to clear | `xxhash-rust` (already used) and `similar` |
| Fit with QuadCam | A second storage model next to the library's | Same pattern as the library: files are the truth, the index is a cache |

**Choice: (b), the content-addressed store.** The data that changes is small. The data that
is large (sounds) never changes, and both options store it once. Git's delta win is a few
hundred KB a session, and it needs either the `git` binary for packing or a heavy
dependency. (b) keeps retention exact, crash recovery trivial, and the code small. The
**Export…** button writes a plain folder per snapshot, so a person can still use git or
any diff tool on an export.

**Rules**

1. **Blobs:** each file is stored once under its XXH64 (the identity hash QuadCam already
   specifies) plus its size. A hash and size match is checked against the bytes before a
   blob is reused, so a collision cannot merge two files.
2. **Snapshots:** a manifest of path → hash, size, mtime. A card snapshot reads each file's
   size and mtime first and hashes only files that changed since the last snapshot.
3. **No empty snapshots:** when the manifest equals the device's latest one, nothing is
   written; the device row says "No changes since <date>".
4. **Retention** (settings in 2.6):

   | Snapshot | Kept |
   |---|---|
   | Taken before or after an apply, a restore or a flash | Always |
   | Pinned by the user | Always |
   | Plug-in and manual | The last `gearKeepRecent` (10); then one a week for `gearKeepWeeks` (8); then one a month (`gearKeepMonthly`) |

   Pruning runs after each new snapshot. It then collects blobs no manifest, log or staged
   change references.
5. **Radio logs are flight data, not snapshot content.** Each `LOGS/*.csv` is stored once in
   `<gear>/logs/<radio-id>/`. A log that grew (the new bytes start with the stored bytes)
   replaces the stored one. A log that changed any other way is kept as a second file.
   Logs are never pruned; the flight index (7.6) reads them.
6. **Storage view** (Gear > Storage): total size, size per device (its manifests, its logs,
   and the blobs only it references), the number of snapshots by kind, **Prune now**,
   **Export…** (a folder per snapshot, or a whole device), and **Import backups…**.
7. **Import of existing backups** (`quadcam-cli gear import-backups FOLDER`, and the same in
   the Storage view): QuadCam walks the folder and finds
   - EdgeTX card copies (a folder with `RADIO/radio.yml` or `MODELS/`): one snapshot each,
     dated from a `YYYY-MM-DD` in the folder name, else from the newest file. Their `LOGS/`
     go to the log store.
   - FC pairs `<stem>.diff_all.txt` + `<stem>.dump_all.txt` (either alone also works): one
     FC snapshot each, dated the same way.
   - A plain `LOGS/` folder: logs only.

   The board and version in each import pick the device; when two devices could match, the
   import asks. Duplicates cost nothing (same blobs). The import never deletes the source
   folder; the report lists what it took and what it skipped and why.

**Browse:** a file tree per snapshot, text files shown, a diff between any two snapshots of a
device. **Restore** stages a change (section 8), never a direct write.

**Built (WP4).** `gear/blobs.rs` (blob key `<xxh64>-<size>`, so files of different sizes never
share a name; a key with other bytes refuses; temp + fsync + rename; `store.lock` held by every
snapshot from first blob to manifest and by collection), `gear/backup.rs` (`Snapshots`: take a
card with the size+mtime skip, progress and stop; take FC files; read, diff, pin, `thin`,
prune, storage, export, `import`), `gear/radiologs.rs`, `gear/health.rs` (the card check
below), `core/backup.rs` (`backup_hooks`: "Card check" and "Backup", registered by the app;
jobs with progress in `GearStatus.jobs` and `gear_stop`). Choices made in the build:

- Radio logs are plain CSV files in `logs/<radio>/` (not blobs), so the flight index reads a
  folder. A second copy is `<name> (2).csv`. The stored file takes the source's mtime, so an
  unchanged log is not read again.
- A snapshot before an apply or a flash is always written, even when its files equal the
  latest (its blobs are shared). `Trigger::Import` is thinned like plug-in snapshots.
- An FC's `status` never makes a snapshot new (`backup::VOLATILE`).
- Pruning runs after a new plug-in or manual snapshot, not after an import.
- An import item equal to a snapshot of the same day, or the newest one before it, is
  `same`. A card copy goes to a QuadCam marker's device, else the one saved radio with its
  board, else the `device` passed; an FC to its `mcu_id` (saved as a new device), else the same
  board rule.
- Goggles and DVR cards get the card check, not a backup.
- Mount, work, unmount: every card job ends in `Core::gear_finish_card`, which unmounts the
  whole disk (`Env.unmount`, `diskutil unmountDisk`) whatever the job did. "Done, safe to
  unplug" plays only after the unmount worked; a failed unmount is the step "Unmount", its cue
  plays and its reason goes to `GearStatus.failures`. An on-connect step that had nothing to
  do returns `core::Skip` (counted as off), so an unknown card is not unmounted or cued.

**Card check (WP4, on connect).** Before the on-connect backup of a card QuadCam knows,
`diskutil verifyVolume` runs on it (read-only; tested on a FAT32 disk image 2026-10-07: no
admin for verify or repair; a damaged FAT gives fsck exit 206 and `-69845`). The result goes to
`<gear>/health/<device>.jsonl` and `GearStatus.card_checks`; a failure fails the "Card check"
step (its cue plays) and the backup still runs. A verify can be stopped (QuadCam then runs
`mountDisk`); a repair cannot. `gear_card_repair` needs the failed check's id and `confirm`,
backs the card up first (`BeforeApply`, always kept) when it reads, repairs, and checks again.
Every `diskutil` run goes through `health::DiskRunner`; a cargo process gets `NoDisk` unless
`QUADCAM_SERIAL=real`.

### 7.2 Switch map

One table per aircraft. Rows: each physical control (switches, trims used as switches,
sticks with logical switches). Columns per position: the channel value in microseconds, the
FC effect (`aux` modes, `adjrange` selections such as rate or OSD profile), the radio effect
(logical switches that turn on, special functions, sounds, timers).

- Channel value: from the model's mixes, weights and `REPL` lines, at each switch position.
  `AUXn` is `CH(n+4)`.
- `adjrange` select positions: position = (value − 900) / 400 for a 3-position select.
- Live: with the FC connected and the radio linked, `MSP_RC` highlights the current
  position and the active modes.
- Without a device: the latest backups of the radio and the FC, with their dates.
- Conflicts are listed: two modes on one range, a switch with no effect, a sound file the card
  does not have.

**Built (WP6).** `gear/switchmap.rs` builds the map; `core/switchmap.rs` reads the files.

- Rows: the switches in `radio.yml` (else the ones the model uses, as 3-position), trims used as
  switches, and sticks a logical switch reads (low, centre, high).
- A position is worked out by running every channel's mixes in file order with that control
  there and the rest at rest (other switches in position 0, sticks centred, throttle low):
  inputs, mix switch conditions, `ADD`, `MUL`, `REPL`, offsets and `limitData` (min, max,
  offset, revert). Logical switches are three-valued: telemetry, timers and sticky state are
  unknown and light nothing (they are listed in `unmapped`, below); `FUNC_EDGE` follows its
  switch and shows as a pulse. A row lists
  the channels, logical switches, special functions and timers that differ between its
  positions. A mode on in every position of a row moves to the notes.
- Combinations (0.7.2): groups of controls that feed one channel (sticks left out), one logical
  switch, one special function or one timer, looking through logical switches. For each position
  of a control, every other setting of the other controls in a group of at most 4 is run; a
  result that differs from the same companions with this control at rest, and from the
  position alone, is a `Combo` (`with`, the changed `channels`, `fc`, `radio`), the smallest
  first and without repeats. Conflicts ("does nothing", "never reached") and live matching count
  combos. A group over 4 controls adds a note.
- Unmapped (0.7.2): a logical switch that is unknown in any run is listed in `SwitchMap.unmapped`
  with `kind` (`telemetry`, `timer`, `sticky`, `depends`), its condition as text, the controls it
  reads and what uses it. A control that feeds one is not called "does nothing".
- "Two modes on one range" means the same AUX and the same start and end; overlapping ranges
  are normal (AIR MODE 900-2100 under ARM).
- Live: `MSP_RC` (105) through `bf::rc_channels` (one MSP exchange, no cue, the port released),
  or the radio's joystick, or channel values given. A row is at the position whose channel
  values all match within 60 µs. The UI matches the joystick stream itself
  (`app/src/lib/controls.ts`, the same rule).
- Pocket buttons: in Classic joystick mode EdgeTX sends CH9-CH32 as buttons 1-24 (its manual,
  USB Joystick); Advanced mode sets this per channel. Confirmed from the manual, not from the
  radio.
- The radio as a USB joystick: `gear/radio_hid.rs` (hidapi, shared open; VID:PID `1209:4f54`;
  19-byte reports: 24 button bits, 8 axes 0..2048 that are CH1-8). `gear_radio` is one look;
  `gear_radio_watch` streams `radio-input` events (at most one per 16 ms, a heartbeat every
  500 ms, reopen after a replug). A process started by cargo reaches no HID device unless
  `QUADCAM_HID=real`. The web view's Gamepad API never saw the radio, so the app does not use it.
- Surfaces: rows `gear_switch_map`, `gear_radio`, `gear_radio_watch`; CLI `gear map`, `gear
  radio`; MCP `quadcam_gear` actions `switch_map` and `radio`. The UI: the Switches segment on FC
  and radio pages, the Controls page under Gear (sticks in Mode 1-4, the `stickMode` setting;
  channels; buttons; the map marked live), and the shared stick widget
  (`components/gear/Sticks.tsx`) the built-in sim can reuse.
- Sources: files, or the latest backups of saved devices (`devices`), or of the radio and FC
  saved with an `aircraft`. From a radio backup the model is the one named, else the one whose
  header name the profile's `edgetx_models` lists, else the selected one. Each source names its
  backup's date.

### 7.3 OSD

- Read `set osd_<element>_pos`, `osd_profile_N_name` and `vcd_video_system` from a dump, a
  backup or a staged change.
- Position value: `x & 31`, bit 10 = `x >> 5`, `(y & 31) << 5`, profile flags 2048, 4096,
  8192 for profiles 1-3, variant `<< 14`.
- Grids: NTSC 30 × 13, PAL 30 × 16, HD 53 × 20, or a given W × H.
- Element widths from a table per Betaflight version, with sample text (`B4.20V`, `L2:99`,
  `T00:00`). Unknown elements draw 5 wide and are listed.
- The horizon sweeps rows y..y+9, columns x−4..x+4; stick overlays are 7 × 5.
- The editor: a grid canvas per profile, drag to move, checkboxes per profile, the check
  (overlap, off screen) live. Each element has ONE position for all profiles on analog
  chips; a profile only turns it on or off. The editor shows that rule by moving the element
  in every profile.
- A move stages `OsdElement` edits. The apply sheet shows the CLI lines.
- **Built (WP7 read half):** `gear/osd.rs` (decode, element table, render, check),
  `core/osd.rs` (`Core::gear_osd` on files; `device` refuses until WP4 backups or the WP2 live
  read feed it the same text), `gear osd` in the CLI, `quadcam_gear` action `osd`. The view is
  `app/src/views/Gear/Osd/OsdSegment.tsx` (`OsdScreen.tsx` draws one view). The Gear page
  frame (WP13) mounts `<OsdSegment />` as the FC page's OSD segment (`views/Gear/segments.tsx`),
  on files for now; it passes the FC's device id once a device source exists (WP4, WP2).
- **Built (WP7 editor):** on a saved FC with a backup the segment is the editor.
  `OsdParams.staged` draws the backup with the device's staged `OsdElement` edits on top
  (`OsdConfig::apply_edits`, the same bits the FC writer keeps). `gear_osd_edit`
  (`Core::gear_osd_edit`) takes moves (`OsdMove`: element, optional x, y, profiles), or a
  profile copy (`OsdCopy`), resolves them against that working layout and joins them into one
  open change titled "OSD layout" (updated in place, made if none): the last edit of an element
  wins, an element back at the FC's value drops out, and an empty change is discarded.
  Staging and the apply go through the WP5 path unchanged. The CLI has `gear osd-edit` and
  `gear osd --staged`; MCP has `quadcam_gear_edit osd_edit` and `quadcam_gear osd` with `staged`.
  The UI (`OsdScreen`, `OsdSegment`) drags, moves with the arrow keys (Shift for five), toggles
  per profile in a list with X and Y boxes, copies a profile, and opens WP5's Copy settings with
  OSD ticked for a copy from another quad.
- **Deviations:** the editor does not fork copy settings: copying a layout between quads is the
  Copy settings `osd` part, so it carries the alarms and units too, not only positions.
  Elements draw as sample text, not Betaflight font glyphs. Variant bits are kept, never
  edited. Edits to an element a diff-only backup leaves out are refused (the core cannot see
  its value). Acceptance: the exhaustive `Pos` round trip, an edit-to-FC-line round trip, the
  NTSC, PAL and HD goldens (`tests/osd.rs`), overlap and off-screen checks, and
  `tests/osd_edit.rs` plus `e2e/osd-edit.spec.ts` for the staged line.
- **Built (WP8 read half):** `gear/rates.rs` re-exports `quadcam_sim::rates` (the one
  implementation of the curves), reads every `rateprofile` of a dump or diff, samples the curves
  (51 points), and fits one model onto another (`fit`, `to_betaflight`). `gear/sims/` holds one
  adapter per game (`liftoff`, `micro`, `uncrashed`, `zone`, `velocidrone`, off) that reads the
  rate profiles with byte spans (`Doc`): a parsed file renders back to the same bytes, and a
  replaced span changes only its bytes (the sync package writes through it). `core/rates.rs`
  holds `gear_rates` and `gear_sims`; the CLI has `gear rates` and `gear sims`; `quadcam_gear`
  has the actions `rates` and `sims`. The Rates segment is `app/src/views/Gear/Rates/`.
  Deviations and decisions:
  - `gear_rates` takes `device`, `backup` or `paths`; `change` (a staged change) joins with WP5.
    The FC's latest backup stands in for a live read: a fresh backup reads the FC.
  - `gear_sims` takes the quad (`device`, `backup`, `paths`, `profile`), which the design's row
    left out, so the comparison runs in the core for the CLI and MCP too. A sim profile is
    "the same" when it is within 1 deg/s of the quad's rates as the Betaflight model (a quad on
    Actual or Quick is fitted first), so a synced sim reads as matching.
  - The fit is least squares over 101 stick points on whole-number settings. Stated bound: 6 %
    of the maximum rate for Actual profiles with a centre of 40-200 deg/s, a maximum of 300-1000
    deg/s and expo 0-60 (tested over that grid); a centre under 40 or expo above 60 reaches
    about 12 %. Every view reports its actual error.
  - Reads do not refuse while a sim runs; `running` is reported and the write package refuses
    (`sim_running`). Under cargo the process list comes from `QUADCAM_SIMS_RUNNING`, a comma
    list of process names, never from `ps`.
  - The shapes were first inferred from 6.6 and checked against a real install by the sync
    package (see the note under the table in 6.6): Liftoff's and The Zone's were wrong and are
    corrected. The fixtures in `tests/fixtures/sims/` are synthetic files of the real shapes.
  - The Sims page and the sidebar "Out of date" badge: built, see the sync half below.

- **Built (WP8 sync half):** `gear/apply/sim.rs` (the plan: targets, checks, diff, warnings,
  digest; `write_atomic`; the test fail-safe), `core/sim_sync.rs` (`gear_sim_sync_plan`,
  `gear_sim_sync`, the sheet's `gear_sim_sync_click`), `sims::write_profile` and the adapters'
  `slots`/`encode` (a replaced span changes only its bytes), `gear_rates_preview` (an edited
  profile redrawn by the one implementation of the math, or fitted onto another model), the
  rate editor in the Rates segment (`RateEditor.tsx`), and **Sync** on each sim profile. Rows
  `gear_sim_sync_plan` and `gear_sim_sync`; CLI `gear sims --sync --to SIM[:PROFILE][@FILE]`
  with `--digest` and `--yes`; MCP `quadcam_gear sim_sync_plan` and `quadcam_gear_apply
  sim_sync`. Deviations and decisions:
  - **No sim device kind.** A sim is not a device with an identity, so the backups live under
    the pseudo device `sim-<id>` (`sim-liftoff`) in the same store, `BeforeApply`, always kept,
    one file named by its path from the home folder. `gear backups --device sim-liftoff` lists
    them. A sim restore reads them back (see "Built (radio over USB)" under 7.5).
  - **`SimSyncRequest` carries the params** (the sims and the quad) with the digest and
    `confirm`: the apply plans again from them, as the FC apply does from the change. A sim sync
    is not a staged change: it overwrites sim files at once through the sheet. The sheet and an
    agent's confirm request use a stand-in change with the id `sim-sync` and the device `sims`.
  - **Targets** are `SIM[:PROFILE][@FILE]`; the profile defaults to the one named like the
    quad's rate profile; `all` takes every sim that has such a profile. QuadCam overwrites an
    existing sim profile and never creates one.
  - **`ApplyPlan` gained `warnings`** (the "warning, not a refusal" row of 8.2): the fitted
    model and its gap, a sim without a throttle curve, the unverified shapes. New refusal code
    `not_writable`.
  - **Checks:** sim closed (`sim_running`, again right before the write), file understood
    (`shape_unknown`), rewrites unchanged byte for byte and reads back as wanted (`round_trip`),
    writable (`not_writable`), something differs (`incompatible`). A changed file after the plan
    is `before_mismatch`. Under cargo a write outside the temporary folder is refused
    (`disabled`), so no test reaches a real player's files.
  - **Write:** all backups first, then each file through a temporary name beside it with the
    file's permissions, a full sync and a rename; the read back compares the bytes and parses
    the file. A failure puts every written file back (the report says whether that worked).
  - **Throttle compare:** a sim profile and the quad count as the same when the throttle
    **mid and expo** agree (what a sync writes). A sim has no hover value, so the quad's
    `thr_hover` no longer makes a synced Uncrashed profile read as different.
  - **Not covered:** the input settings of the sims (deadband, input expo, channel map). Design
    6.6 describes rates only, and QuadCam has no source for those values to sync. Uncrashed's
    deadzone is one `DeadZone` float per controller file under `RC/` (shape checked against a
    real install, one of five files holds none); a later package can add it.
  - **The Sims page and the sidebar "Out of date" badge** are built (shell gaps). The Sims
    list is one component (`Sims/SimList.tsx`) in the Rates segment and on the page. The badge
    comes from `GearStatus.sims_out_of_date`: the count of enabled sims whose rates differ from
    the quad's profile in use. The quad is `GearStatus.sims_quad`, the saved FC seen last that
    has a backup; the page's **Compare with** picks another. The count is cached on the quad's
    backup id and the sim files' times. A process started by cargo reads no real sim file
    unless `QUADCAM_SIMS_HOME` names a folder. A sim reads "Out of date" or "Matches the quad";
    Velocidrone reads "Off" with the note "Not supported yet. QuadCam needs a sample
    Velocidrone save to read its rate file, so this sim stays off." (decision 12).
  - **Rate editor:** edits are `FcSet` values in the profile's rate section (`*_rc_rate`,
    `*_srate`, `*_expo`, `*_rate_limit`, `rates_type`, `thr_mid`, `thr_expo`,
    `throttle_limit_type`, `throttle_limit_percent`, `rateprofile_name`), only the ones that
    changed, as one change the sheet applies. A dump file is not editable. `thr_hover` is shown,
    not edited.

### 7.4 Voice packs

**Line text is repo data, written by QuadCam.** `src-tauri/resources/voice/lines.csv` holds
every line QuadCam knows. It is not a copy of EdgeTX's voice-list file (that file is GPL-2.0):
the file names are the names the firmware plays, a fact of the interface, and the text is
written for QuadCam from what each sound means. Columns: `path,text,group,why`, where `path` is the file on the card (`SOUNDS/en/armed.wav`,
`SOUNDS/en/SYSTEM/0042.wav`), `text` is the spelling that renders well, and `group` is
`system`, `numbers`, `units`, `callouts`, `extras`. `spelling.toml` holds the rules:

- Dot acronyms that must be spelled out: `D.V.R.`, `V.T.X.`, `T.X.`, `P.I.D.`, `G.P.S.`,
  `L.Q.`, `R.F.`, `D.B.M.`, `D.B.`. Leave plain the ones a voice reads well (`OSD`, `3D`).
- Abbreviate only when context makes the meaning obvious; otherwise spell it out
  ("decibels").
- Track names are 8 characters at most.

The rules apply at render time, so the CSV keeps the readable text and the rules give the
spoken text. A user's own lines (custom callouts) live in `gear.json`, not in the repo
file, and never ship in a pack.

**Built (WP9 voice):** `gear/voice/` (`lines.rs` the lines and spelling rules, `tts.rs` the
providers, `render.rs` the cache and normalisation, `packs.rs` build, index and install,
`wav.rs`), `core/voice.rs`, `resources/voice/{lines.csv,spelling.toml}`. Rows `gear_voice`,
`gear_voice_edit`, `gear_voice_render`, `gear_voice_pack_install`, `gear_voice_choose` and one
the design did not list, `gear_voice_preview` (copies a take into the cache, which the asset
protocol serves; the gear folder is outside its scope). `build-pack` is CLI only
(`Core::voice_build_pack`). The Voice segment is `views/Gear/Voice/`.

- Providers: `say` (`/usr/bin/say`, the text on stdin, a WAV file out) and `openai` (an
  OpenAI-compatible `/v1/audio/speech`, asked for a WAV; a local Kokoro-FastAPI server is the
  model). Both sit behind `Tts`, take their outside world through `Run` and `Post`, and are
  built by `Env.tts`, which is off under cargo (`QUADCAM_TTS=real`, `QUADCAM_FETCH=real`), so
  tests use fakes. The key goes to curl on stdin in a config file. No ElevenLabs adapter: the
  design asks for one, and a paid adapter waits for open question 6.
- Settings: `tts_provider`, `tts_key`, and new `tts_base_url`, `tts_model`, `tts_voice`,
  `voice_index` (in `settings.json`, not in `GearSettings`). `QUADCAM_TTS_KEY` beats `tts_key`.
- Render: `Cache` keys a take by provider, voice, model, provider speed, spoken text and seed
  (sha-256) and stores it as a WAV, not a `.pcm`, so the rate travels with it. `normalise` runs
  ffmpeg only for a tempo or a rate other than 32 kHz; the trim, pads and fades are integer
  math, so `tests/fixtures/voice/golden.wav` pins the bytes. The fades act on the speech, and
  the lead and tail pads are silence added after.
- Packs: `build` renders the lines to a folder and zips it with `/usr/bin/zip` (no zip crate),
  then writes the entry into `voices.json`. `install` checks the zip's hash, lists its members
  with `unzip -Z1` and refuses a path outside `pack.json` and `SOUNDS/`, unpacks, and checks the
  manifest's id and files. The index is read from `voice_index` (a path, or an address through
  `Fetch`) into the cache; a pack zip sits next to its index.
- Choose voice: `Edit::CardFiles` is now applied. `card_work` reads each `CardFile` from the blob
  store and plans it like a restore (`Card::plan_files`), so backup, write, read back and
  rollback are WP5's. A change of card files stands alone. Overrides live in `gear.json`
  (`voice.overrides.<radio>`, a `text` override holds its WAV as a blob; `backup::change_keys`
  keeps those blobs from collection), as does the chosen voice and the person's custom lines.
  Choosing with Keep my overrides off clears that radio's overrides when it stages.
- Not built, or different from the design: the render hook for carrier sentences is a
  documented seam in `tts.rs` only. `lines.csv` holds 45 lines, not about 745: the callouts
  QuadCam names, the numbers 0 to 20 and six system sounds; the rest of EdgeTX's set is a data
  task. Units and their file names are not in it. The ElevenLabs adapter and the quota report
  are not built; the report gives characters and whether the provider may charge. A render
  runs inside the call, with no progress events. `lang` is `en` only.
- Acceptance: `tests/voice.rs` (the spelling golden, a cache hit with no provider call, the
  normalisation golden WAV, the providers' requests, `build-pack`'s zip and index entry, the
  install checks, a hostile zip) and `tests/voice_core.rs` (render and cost, confirm, install,
  Choose voice staging one change and keeping overrides, apply with read-back),
  `e2e/voice.spec.ts`.

**Packs are release assets, not git files.** A pack is a zip per voice
(`voice-<id>-<version>.zip`) attached to a GitHub release, plus one `voices.json` index:

```json
{ "id": "en-callum-v1", "voice": "Callum", "lang": "en", "provider": "elevenlabs",
  "model": "eleven_turbo_v2_5", "settings": { "speed": 1.2, "tempo": 1.1, "trim_db": -55,
  "tail_ms": 150, "fade_out_ms": 30, "seed": 1 }, "lines": 745, "bytes": 37000000,
  "sha256": "…", "license": "…", "attribution": "…", "lines_csv_sha": "…" }
```

The app downloads the index on request, lists the packs, and installs one into the gear
folder after a hash check.

**In the app (Radio > Voice):**

- A table of lines: path, text, and one play button per installed voice, side by side.
  Packs from the index that are not installed show a download button in their column.
  Space plays the selected line in the current voice.
- **Choose voice** (a pop-up of installed voices) stages one `CardFiles` change that
  replaces every pack line on the card with that voice. A checkbox **Keep my overrides**
  (on by default) keeps the lines the user rendered or picked per line. Apply: backup first,
  write, read back, compare hashes.
- Per-line override: pick another voice's take, or render the user's own text with the
  user's provider, for that line only.
- Files on the card that no pack knows (other custom clips) are never deleted.

**Render pipeline** (`gear/voice/render.rs`; the maintainer builds packs with
`quadcam-cli gear voice build-pack`, and users render their own lines through the same code):

1. Text from `lines.csv` with the spelling rules applied.
2. Provider call: `say` (macOS, offline, free), ElevenLabs (raw PCM at 32 kHz, a fixed seed
   per take), others behind the `Tts` trait. Keys from the environment first, then the
   setting; sent on stdin to curl or in a header, never in argv or logs.
3. Raw cache key: provider, voice, model, settings, spoken text, seed. A re-render with other
   trim or tempo settings costs nothing.
4. Normalise with ffmpeg: `atempo` first (so the pads keep their length), then trim
   (5 ms windows, speech within −55 dB of the clip peak), 20 ms lead, 150 ms tail, 5 ms fade
   in, 30 ms fade out.
5. Write 32 kHz 16-bit mono PCM WAV, plain RIFF (the rate the official packs use).
6. `build-pack` writes the zip and the index entry with the provider, model and settings.

Before an uncached paid render, the report gives the character count and the account's
remaining quota; the render runs only when confirmed.

### 7.5 Splash screen and radio firmware

Flow on the Radio > Splash segment:

1. Pick an image. The preview shows it at the radio's resolution and bit depth (128 × 64,
   1 bit for B&W radios) with a threshold slider and **Invert**, at 4× with nearest-neighbour
   scaling.
2. **Make firmware…** opens the flash plan: the board, the EdgeTX version (default: the one
   installed), the download, the patch.
3. Patch: find the marker `SPS\0` followed by width 0x80 and height 0x40; the 1,024 bytes
   after it are the image (8 vertical pixels per byte, column-major per 8-row band, bit set =
   dark); `SPE` must follow. Write the new bytes, then decode them back and compare with the
   preview. Refuse when a marker is missing, appears twice, or `SPE` is not where expected.
4. Flash through DFU (section 6.5), backup of the SD card first.

**Proven on:** RadioMaster Pocket, EdgeTX 2.12.4 (B&W 128 × 64). `compat.rs` lists that
pair. Other B&W boards are added when a test binary from that release shows the same
markers. Colour radios are refused with the reason "This radio's splash format is not
supported yet."

The EdgeTX cloud build service is an alternative source; 1.0 uses the GitHub release
binaries only (open question 10).

- **Built (WP10, parts 1 to 3):** `gear/splash.rs` (image to 1-bit 128 x 64, the preview, patch
  and decode with markers and refusals), `gear/dfu.rs` (USB DFU 1.1 with DfuSe: layout string,
  erase, write, read back, leave; `NusbUsb` the real transport, `FakeDfu` a device in memory
  with faults), `gear/firmware/` (`mod.rs` the `Flasher` trait, its recorder, `FwEnv` and the
  fixture fetcher; `check.rs` the version check; `edgetx.rs` the release download, the board
  binary and the image checks), `core/firmware.rs` (the `Core` methods). Rows `gear_firmware`,
  `gear_splash`, `gear_flash_plan`, `gear_flash` (and the GUI-only `gear_flash_click`); CLI
  `gear firmware` and `gear splash`; MCP `quadcam_gear` `firmware_check`, `splash`,
  `flash_plan` and `quadcam_gear_apply` `flash`; the Firmware page, the radio's Splash segment
  and the flash in the apply sheet. Deviations and decisions:
  - **`png`, not `image`.** The splash needs PNG decode only, and `png` is already in the
    dependency tree. A JPEG source would need `image`.
  - **DFU over a transport trait.** `Usb` is two control transfers and the layout string. The
    real one uses `nusb` 0.2; every test flashes `FakeDfu`, which follows the DfuSe state
    machine and models flash (a program only clears bits, an erase fills a sector). The real
    transport has not touched a device. `Flasher` hands out a `Usb`; a process started by cargo
    gets a recorder unless `QUADCAM_FLASH=real`, and `dfu::list` (the DFU devices in
    `gear status`) follows `QUADCAM_SERIAL`. `FwEnv` (the fetcher and the flasher) is a `Core`
    field with `with_firmware_env`, not part of `gear::Env`, so existing tests that build an
    `Env` stay as they are.
  - **Check.** EdgeTX and Betaflight from the GitHub release lists (newest stable by version,
    not by order), ExpressLRS from the artifactory `index.json` (the highest tag without a
    suffix). The saved answer is `<cache>/firmware/latest.json`; a failing source keeps its old
    value and adds an error. `gear_firmware` takes `check`: true reads the network, false the
    saved answer, unset follows `firmwareCheck` (`daily`: only when the answer is a day old).
    The page asks with unset; the sidebar badge never reads the network.
  - **The plan gate is the target version.** The flash checks `compat::check_writable` for the
    version to flash, not the installed one: EdgeTX 2.12 on the Pocket, and for a splash the
    `Splash` pair (2.12.4). The plan downloads only after those pass. The default version is
    the installed one, so a splash on 2.12.3 is refused until the radio updates.
  - **Which file is the board's.** Verified against the official 2.12.4 zip (2026-10-09): the
    `.bin` files are named `<board>-<7 hex of the commit>.bin` (`pocket-def35ad.bin`), next to
    `.uf2` files for newer boards, `fw.json` (display name to `pocket-` prefix) and `LICENSE`.
    The first version of this code guessed `fw-radiomaster-pocket-v2.12.4.bin` and would have
    refused every real release; the match is now the name less `.bin` and the hash, against the
    board's names (`edgetx::BOARDS`). Exactly one must match.
  - **Full image only.** Verified: the Pocket's 2.12.4 `.bin` is 519,580 bytes (507 KB) and a
    full image. The bootloader's vector table is at offset 0 (stack `0x10010000`, reset
    `0x08007bd9`), the firmware's at `0x8000` (reset `0x0807ea7d`), so the image is written at
    `0x08000000` and replaces the EdgeTX bootloader with the release's. `check_image` requires
    both tables, the size band, and that the image names `edgetx-<board>-<version>`
    (`edgetx-pocket-2.12.4 (def35ad3)` in the real one) so a file of another release cannot
    pass.
  - **A verified copy before any erase.** `dfu::read_verified` reads the whole flash twice
    through `dfu::ReadOnly`, a transport that refuses erase, write and leave, and returns a
    `VerifiedCopy` only when both reads are equal and the flash is not all zeros. `dfu::flash`
    takes a `&VerifiedCopy` that covers the range, so an erase without a copy does not
    compile. The apply saves the copy (`gear/fwcopy.rs`, plain files under
    `<gear>/firmware/<device>/`, read back from disk) before the erase. A blank flash is a
    valid copy to flash over (an interrupted flash must be recoverable) and is not saved.
    The copy is not a card snapshot: a snapshot of only `firmware.bin` would become the radio's
    `latest` backup, which `model_edit`, the switch map and the apply engine read cards from,
    and the pruner would collect its blob. (The first version did that.)
  - **Each block checked.** The write goes in 16 KB segments; each is read back and compared
    before the next, then the whole image is read back. The device's `wTransferSize` is read
    from its DFU functional descriptor (`NusbUsb`); DfuSe addresses blocks with it, so any size
    other than 2,048 refuses before the erase. The flash size must equal the board's
    (`BoardSpec::flash_bytes`, 1 MB for the Pocket).
  - **Read-only trial.** `gear_firmware_read` (CLI `gear firmware --read`, MCP `quadcam_gear`
    `firmware_read`, the Firmware page's Read firmware, event `firmware-read-progress`) reads
    through `ReadOnly`, saves a copy and compares the version the image names with the radio's.
    The Firmware page lists the steps: radio off, USB in, no buttons, wait, Read.
  - **Entering DFU.** The EdgeTX manual says: radio off, USB cable in (the ROM bootloader,
    `0483:df11`); both trims held with power on starts the EdgeTX bootloader (mass storage), a
    different mode. The first version told the person to hold the trims, which is wrong for
    DFU. Fixed in the plan, the page and the docs.
  - **The DFU device has no identity.** The STM32 bootloader reports a chip serial, not the
    radio. The plan flashes the one STM32 DFU device present (zero or two refuse) and warns.
    Linking a DFU serial to a saved radio is built: see "Built (radio over USB)" below.
  - **After the flash.** The apply ends at "Leave DFU" with every byte compared. Reading
    `semver` in USB Storage mode (8.4) is left to the person; the report says so.
  - **A flash is a stand-in change.** The sheet and an agent's confirm request carry a change
    with the id `flash`, like the sim sync.
  - **Splash layout verified** (2.12.4, Pocket, 2026-10-09). `SPS\0` appears once in the
    image, followed by `0x80 0x40`, 1,024 bytes and `SPE`; `SPE` also appears at five other
    places, so only the start marker is counted. The bytes are 8-row vertical bytes (byte
    `band * 128 + x`, top row in bit 0) and decode to the upright EdgeTX logo with a set bit as
    a drawn pixel. The tests build a synthetic image with that layout; no GPL bytes are in the
    repository.
  - **Still unverified on hardware** (WP10): the `nusb` transport on a real DFU device; that
    the Pocket enters ROM DFU the way the EdgeTX manual says; the DfuSe set-address, erase and
    upload sequence against the real ROM bootloader (it follows ST's notes and `dfu-util`);
    the 1 MB layout string the Pocket reports; the DFU functional descriptor's
    `wTransferSize`; that the image the release ships boots after QuadCam writes it. The first
    real step is the read-only trial. Any mismatch there fails safe: the read refuses.
  - **Not built, deferred to 1.1 (decided 2026-10-09, both wanted):** the Betaflight flash
    plan, and ELRS options and flashing (needs `esptool`, serial passthrough and the options
    block). 1.0 keeps the version check for Betaflight. The CRSF device-info ping is open as
    before.

- **Built (radio over USB):** four follow-ups, branch `radio-usb`.
  - **One radio, two ids.** `Device` gained `aliases` and `dfu_serial`; `Connected` gained
    `also`. `detect` lists every id a radio card answers to (hardware serial in the built-in
    slot, marker, volume UUID); `Core::gear_connected` takes the saved radio whose id or alias
    matches, shows it under the saved id, and records the ids it saw as aliases. The identity
    stays the card's, not the USB serial (all radios report the same generic one). A radio
    never seen in the slot first has one id only, so there is nothing to match, and a brand
    new second radio of the same board is never merged by board alone.
  - **Storage-mode rules.** Already in `card.rs` (timeouts, `stuck_message`, the unmount
    after every job, the `._` removal beside each write). Added: the apply sheet's "keep the
    radio plugged in" line while an over-USB write runs, and **Clean ._ files**
    (`gear_card_clean`: `find_apple_double` lists files that start with the AppleDouble header,
    `remove_apple_doubles` deletes them under a timeout; `confirm` is required; the card
    unmounts after). QuadCam never prompts to unplug during a write.
  - **EdgeTX CLI link.** `gear/edgetx/cli.rs` (`RadioCli`, `FakeRadioCli`) over `Ports::open`,
    so it takes the port lock and refuses a port another process holds, as the FC link does.
    The commands are a closed list: `ver`, `ls`, `play`, `beep`, `reboot`. A path is checked
    before it is sent. `core/radio_cli.rs` holds `gear_radio_cli` (identify, ls, play, beep,
    reboot with `confirm`, verify). **Verify** `ls`es each folder of a saved radio's latest
    backup (not `LOGS/`) and reports missing files and size differences; the card must be back
    in the radio, because storage and serial modes are exclusive. The reply formats of `ver`
    and `ls` are parsed tolerantly (key: value lines; a name with an optional size) because the
    real replies are not recorded yet. WP9's `gear_voice_preview` plays a pack's take on the Mac;
    `radio-cli play` plays the line the radio holds, and takes a voice line's card path as it
    is (`SOUNDS/en/SYSTEM/hello.wav`), so a line can be heard on the Mac before an apply and on
    the radio after it.
  - **DFU link.** `Link::Dfu` carries the chip serial. `gear_dfu_link` stores it in the picked
    radio (`dfu_serial`), or in the radio seen most recently when none is picked, and says
    which. `gear_connected` shows a linked DFU device under that radio's id. The flash plan
    has the check "The radio in DFU mode is this radio" (`device_changed`) and, for an unlinked
    device, a warning that names the radio seen most recently; a verified flash links it. The
    plan's digest includes the DFU serial. There is no Firmware page yet, so there is no UI for
    the link: the CLI and MCP carry it.
  - **Sim restore.** `core/sim_restore.rs`: `gear_sim_restore_plan`, `gear_sim_restore`, the
    sheet's `gear_sim_restore_click`; a stand-in change `sim-restore` on the device `sims`. The
    backup is the one named, else the newest that differs from the file now; the current file
    is backed up first (kept), so a second restore undoes the first. Checks: backup found and
    readable, sim closed, path inside the home folder, file present and writable, something
    differs. Under cargo a write outside the temporary folder is refused. UI: **Restore backup**
    in the Rates segment's Sims list (the section already lists sims, profiles and sync state;
    no separate Sims page).
  - **Needs a real radio:** `ver` and `ls` reply formats and the prompt on a real CLI; `play`
    with a real sound path; `reboot` coming back; the alias match with a real card moved from
    the slot into the radio; a DFU link with a real chip serial; `Clean ._ files` on a card
    macOS has written to.

### 7.6 Flight analysis

`logs.rs` gains the columns QuadCam does not read yet: `TPWR(mW)`, `RSNR(dB)`, `Curr(A)`,
`Capa(mAh)`, `Bat%(%)`, `FM`, `CHn(us)`, `TxBat(V)`. Flights are the existing armed segments.

| Measure | Definition |
|---|---|
| Hover throttle | Median throttle (0-100 %) over windows of 2 s or more where the throttle varies under 3 % and the roll and pitch sticks sit within 5 % of centre. Reported per flight, early third and late third |
| Sag | RxBt under load: the 5th percentile and the minimum while armed |
| Resting voltage | RxBt median over the seconds after disarm while telemetry still streams; else the first reading of the next arm |
| mAh at landing | `Capa` at disarm |
| Threshold | For a mAh threshold (the user's warning value), the flight second it was crossed and the resting voltage that pack reached. The pack page suggests a threshold for a target resting voltage |
| Dropouts | Spans of 0.3 s or more while armed where `FM` is blank and RxBt reads 0. For each: start, length, and the last and first `RQly`, `1RSS`, `TPWR` at its edges. "Downlink only" when the flight goes on and no failsafe mode shows |
| Per-pack history | Flights assigned to a pack (by hand; QuadCam suggests the next label in order). Per pack: cycles, mAh, sag and resting voltage over time. A pack whose resting voltage or flight time falls well below its type's median is marked |
| Range trend per place | Per place (from the matched clip, else the profile's place), the worst `1RSS` and `RQly` per flight over time |

**Built (WP12):** `logs.rs` reads the new columns into `LogRow` (`tx_power_mw`, `snr_db`,
`current_a`, `capacity_mah`, `bat_pct`, `flight_mode`, `channels`, `tx_bat`) and
`LogRow::armed` (`FM` not ending in `*`). `gear/flights/` splits a log into flights (armed
runs; with no `FM` column the same as `logs::segments`), measures each one, and caches them per
log file in `flights.json`; the 5th percentile is nearest rank. `gear/flights/synth.rs` is the
synthetic log with its known values (`KNOWN`). `core/flights.rs` joins each flight to its
aircraft (profile `edgetx_models`), its clip (date and time of day, 90 s slack), its place and
the pack type's warning. Flights read `<gear>/logs/`, folders the person adds
(`flight_folders` in `gear.json`) and a radio plugged in. Additions approved 2026-10-07: the
session report (`gear/report.rs`, `gear_session_report`, Markdown; `gear_session_report_save` writes the file), the Pack up check
(`gear/preflight.rs`, `gear_preflight`; a radio not plugged in gives its selected model from the
latest backup's `radio.yml` and its card space from `Device.last_space`, recorded at each radio
card backup; both rows say "from backup" and its age) and the crash and repair log
(`gear/crashes.rs`, `gear_crashes`, `gear_crash_save`, `gear_crash_delete`).

### 7.7 Bench queue

The Bench page replaces a hand-kept list. Each staged change has a status:

| Status | Meaning |
|---|---|
| Draft | Being edited |
| Ready | Agreed; apply it |
| Try | Apply, fly, then keep or revert |
| Read first | Read the real value on the device before changing anything |
| Applied / Verified / Failed / Reverted | After the apply sheet |

The page groups by device and shows "Next session" per device: the first Ready or Try item.
A Try item that is applied gets **Keep** and **Revert** buttons; Revert stages a restore of
the backup the apply took (for an FC, only the inverse of that change's lines). Applied items move to History. **Copy as Markdown** exports the
queue. Built in WP5b (section 8): the page lists devices that have waiting changes, shows
"Plug in the radio in USB Storage mode to apply 3 changes." for a device that is away, and
disables Review for a Draft or a Read first change.

### 7.8 Packs and charging

Packs are user data: label, pack type, received date, retired. Pack types hold chemistry
(LiPo, LiHV), cells, capacity, connector, and charge settings (full, storage, rate). The
Charging view is a table of pack types with their settings, plus free notes. Nothing in the
repo names a real pack or a shop.

**Built (WP12):** `gear/packs.rs`: `Pack` (label, type, received, retired, `charged_at`,
note), `PackType` (chemistry, cells, capacity, connector, full and storage volts a cell,
charge current, `warn_mah`), the pack set on each flight (`flight_sets`), and `view`: per
pack its history, cycles, charge state and a mark when its median resting voltage is 0.05 V a
cell under its type's or its flights are 20 % shorter; per type the charging sheet and a
suggested `warn_mah` (a least-squares line of resting volts a cell against mAh, at
`target_v`, default 3.7 V). `suggest_pack` names the next label in order.

### 7.9 Card prep

Formats a DVR or goggles card that has no session: a new card, or a card whose clips are all
in the library. Guards: every existing `disk::format_card` guard, plus "no clip on the card is
missing from the library" (the sidebar's "N new" is 0). The file system and the label come
from the source's `CardPolicy`: FAT32 for an analog DVR card, exFAT for a removable DJI
goggles card (decided 2026-10-07, open question 3). A DJI device over USB (an air unit,
goggles storage) is never erased. A card's size never refuses; the policy turns it into
advice in the confirm.

Built in WP11 before WP1 landed: the `Core` methods live in `core/prep.rs`, the two rows in
`api/mod.rs`, and the CLI is `format --prep`. `core/gear.rs`, `api/gear.rs` and
`bin/quadcam-cli/gear/` exist now; card prep moves there later.

Built (shell gaps): the app has the button. **Prepare card…** is on the device page of a DVR
card or goggles card (mounted, or unmounted but still in) and in the Finish step of a DJI
import, which offers no Format card. The plan comes from `card_prep_plan` (`CardPrepParams`:
a `mount` point or a `device` id); the Erase click calls the GUI-only command
`card_prep_click`, which is `card_prep` with `from_gui_button` true. Every guard stays in
`disk::format_card`; the button moves none of them. The CLI takes `--card <id>` and the MCP
tool `card`, for a card that is unmounted.

### 7.10 Modules

Tools and firmware that QuadCam does not ship become **modules**: QuadCam downloads them from
their upstream, on the user's request, into its own folder. A user needs nothing installed
elsewhere, and the `.app` bundles nothing GPL. QuadCam fetches; it does not redistribute.

**Manifest.** `resources/modules.toml` pins each tool module per QuadCam release. A sketch
(the file itself holds the real pins and fields, `docs/modules.md`):

```toml
[esptool]
version = "5.4.0"
url = "https://github.com/espressif/esptool/releases/download/v5.4.0/esptool-v5.4.0-macos-arm64.tar.gz"
sha256 = "…"
license = "GPL-2.0-or-later"
source = "https://github.com/espressif/esptool"
run = "esptool"            # the executable inside the archive
```

Only official release assets of the upstream project are allowed. A release of QuadCam
also attaches `modules.json` (the same data), so a running app can learn of a newer pin
without an app update; it applies a newer pin only after the user's **Update**.

**Candidates**

| Module | Kind | Source | Decision |
|---|---|---|---|
| esptool | Tool | Espressif's standalone macOS release binary | Module |
| dfu-util | Tool | No official macOS binary | Not a module: `gear/dfu.rs` does DFU natively |
| ffmpeg, ffprobe | Tool | A static macOS arm64 build from a maintained, signed source; LGPL build preferred (QuadCam's encoders are VideoToolbox; x264 needs GPL) | Module by default, Homebrew as the fallback (`ffmpegSource`); open question 12 |
| EdgeTX firmware | Data | EdgeTX GitHub releases | Downloaded per version at flash time |
| ExpressLRS firmware | Data | The ELRS artifactory index | Downloaded per version at flash time |
| FC vendor Betaflight builds | Data | The vendor's or Betaflight's release pages | Version check only in 1.0 |

Data modules are never run. Their hash comes from the upstream index when it publishes one;
otherwise QuadCam records the hash at first download and shows it in the flash plan.

**Install flow**

1. A feature needs a module that is missing (or the user selects **Install** in Settings >
   Modules).
2. A sheet names the module, its version, its size, its license with a link to the full
   text and the source, and the upstream URL. Buttons: **Cancel**, **Download**.
3. QuadCam downloads to the cache, checks the SHA-256 (a mismatch deletes the file and
   refuses with "The download does not match the expected checksum."), unpacks into
   `~/Library/Application Support/app.quadcam/modules/<name>/<version>/`, and writes
   `installed.json` (hash, date, license, source).
4. The feature continues.

**Running a downloaded binary on macOS**

- QuadCam downloads with `/usr/bin/curl` (`modules::fetch`), so the files carry no quarantine attribute.
  If one is present (a user copied a module in by hand), QuadCam removes it only after the
  checksum matches.
- Apple Silicon runs only signed code. Prefer upstream binaries that are signed and
  notarized. An unsigned upstream binary gets a local ad-hoc signature after the checksum
  passes; the installed hash then records the signed file.
- A module runs only as a child process: an absolute path inside the modules folder, arguments
  as an argv list (never a shell), a fixed working folder in the cache, a minimal environment,
  a timeout, and output captured for the log. QuadCam's hardened runtime does not limit a
  child process; nothing is loaded into QuadCam itself.
- Before each run, the file's hash is checked against `installed.json`. A changed file is
  refused with "esptool was changed after install; reinstall it."

**Updates and removal.** **Check for updates** (Settings > Modules, or with
`firmwareCheck` = `daily`) compares installed versions with the newest pins. **Update**
installs the new version next to the old one, then removes the old. **Remove** deletes the
module folder. A feature that needs a removed module asks again.

**Settings > Modules:** a table with name, version, size, license (a link), source (a
link), and **Update** and **Remove** per row; **Check for updates**; the ffmpeg source
pop-up (QuadCam module or Homebrew).

### 7.11 Device events, on-connect steps and cues

**Device events** (`gear/events.rs`). Each poll compares what is plugged in with the last look
and reports:

| Event | When |
|---|---|
| `connected` | A device appeared |
| `identified` | A device on the same link got its id (an FC after MSP identity) |
| `unmounted_present` | A volume unmounted, and its device is still plugged in |
| `removed` | A device is gone |

The app sends them in `device-changed` with the connected and the unmounted devices.

**Release cards with `diskutil unmountDisk`, not `eject`.** Tested on a USB reader
(2026-10-07): after `eject` the card's disk node disappears and the reader shows no
card-present flag, so an ejected card cannot be told from a pulled one. After `unmountDisk`
the whole-disk node (`/dev/diskN`) stays while the card is in, and goes within seconds of the
pull. Presence (`events::presence`, read-only) checks:

| Case | Sign of presence |
|---|---|
| Any card, after `unmountDisk` | Its whole-disk node in `/dev` (`Link::Volume.whole_disk`) |
| The built-in SD slot | Its whole-disk node after `unmountDisk`, as for a USB reader (tested 2026-10-07). `AppleSDXCSlot` also reports `Card Present` in the IORegistry |
| A DJI air unit or goggles | The USB device (vendor `0x2ca3`) stays on the bus after its volume unmounts |

**The built-in SD slot and full-size adapters** (tested 2026-10-07, microSD cards in
full-size adapters):

- `diskutil` lists the slot as internal and physical, with protocol `Secure Digital`
  (`Internal` true, `RemovableMedia` true). Detection must select cards by protocol and the
  removable flag, never by "external" (`DiskInfo::is_slot_card`, which `disk::is_removable`
  takes).
- `system_profiler SPCardReaderDataType` shows the card's hardware identity: product name,
  manufacturer id, serial number, manufacturing date and capacity. USB readers hide it.
- After `unmountDisk` the whole-disk node stays while the adapter is in. Pulling the adapter
  removes it at once.
- Pulling only the microSD out of its adapter is invisible: the adapter holds the card-detect
  switch, so the node and the serial number stay. A USB reader with a microSD in a
  full-size adapter does the same (tested 2026-10-07 on a second USB reader, which shows no
  card serial). The one probe that tells, in the slot and in USB readers alike, is
  `diskutil mountDisk diskN`: with the card in, a volume mounts (QuadCam unmounts it again);
  without it, `diskutil` still says "Volume(s) mounted successfully" but no volume and no
  mount point appear. `events::probe_media` runs it for any whole disk. QuadCam runs this
  probe only on demand (once when it shows "still
  inserted", or when the person asks), never in a poll loop.

**Card identity**, in order: the card's hardware serial (manufacturer id and serial number,
from the card reader, built-in slot only); else a QuadCam marker file on the card (a write to
the card, so it comes with the backup or import package that first writes there); else the
volume UUID, which a format changes. WP1 reads the hardware identity
(`detect::parse_card_reader`) and uses it for a card in the built-in slot. WP3 reads the
marker (`.quadcam-id`, one line: the raw value the id hashes, so writing it keeps the id) for
a radio card; `Card::plan` adds it when the caller passes the value. Other cards use the
volume UUID.

**USB hubs and the accessory prompt.** A card reader behind a USB-C dock (a USB 3 hub)
behaves as when it is plugged in directly (tested 2026-10-07). On Apple silicon, macOS asks
"Allow accessory to connect?" for a new USB device, and the device does not enumerate at all
until the person allows it. Approvals are remembered. No app can see a pending prompt, so
every empty "Connected" state and `quadcam_gear status` say "Nothing found. If macOS asked to
allow an accessory, click Allow." (`mcp::gear::NOTHING_FOUND`).

A disk number can be reused by the next disk; a new disk with the same number reads as still
present until the next look shows its own volume. "Safe to remove" (`disk::safe_remove`: the
`eject` method, after a format or card prep) runs `unmountDisk` for a card, so every such
release leads to `unmounted_present` and the "safe to unplug" cue.

**On-connect steps.** `Core::gear_add_hook` registers an `OnConnectHook`: a name, an
`Automation` (`backup`, `import`, `apply_ready`, `blackbox`), the device kinds, and the function. The app
runs `Core::gear_on_connect` on its own thread for each `connected` and `identified` event.
A hook runs only when `gearOnConnect` lists its automation for the device's kind; `backup`
also needs `gearAutoBackup`. Defaults: `backup` for every kind, the others off. WP4 registers
backup, a later package import, WP5 `apply_ready` (which still goes through the plan, the
checks and the confirm in section 8; an automatic apply never skips them), and BB `blackbox`
(FCs only; off until the person lists it; it skips a port whose reads are paused, and the erase
inside it still needs `gearEraseBlackbox`). A failed step plays
the `step_failed` cue.

**Cues** (`gear/cues.rs`) are quiet by design. QuadCam mounts, unmounts and opens ports all
the time: every job mounts, works and releases, and a quad is released after every job. Rules:

1. **Outcomes, not transitions.** A cue marks an outcome the person cares about. A job holds
   its device's link (`Core::gear_hold`, by whole disk or port); events on a held link, and
   for `HOLD_GRACE` (6 s) after, are `app_initiated`: they run no hook and play no cue.
   Device events never cue on their own.
2. **One cue per job, at its end** (`Core::gear_job_done`). FC jobs (`core/fc.rs`) hold the
   port, open it, release it before the cue, and add the board's known issues to their answer.
   A refusal (a busy port, several FCs, an unproven board) plays no cue. The one cue outside a job
   is "Unplug <FC> now." (`unplug_now`), once per battery session (2.3).
   The job cue: "<device> done, safe to unplug."
   or "<step> failed on <device>.". A batch or an automation run over several devices gets
   one cue for the whole run (`Core::gear_batch_done`: "3 devices done, safe to unplug.",
   "Backup failed on 1 of 3 devices."). `gear_on_connect` is one job.
3. **Debounce and queue.** The same cue for the same device within `debounce_s` (30 s) is
   dropped. Cues play one at a time from one queue (`CueService`); `say` finishes before the
   next cue starts.
4. **The reminder** ("<device> is still inserted.") starts only after a job's "done" on a
   card, waits `reminder_grace_s` (60 s), repeats every `still_inserted_every_s` (300 s) at
   most `reminder_max` (3) times, and stops when the card is removed or the person dismisses
   it (`Core::gear_dismiss_reminder`).
5. **Toggles.** Each cue (`safe_to_unplug`, `still_inserted`, `step_failed`, `unplug_now`) and each channel
   (`speech`, `sound`, `notification`) has a toggle, and `mute` silences all. `quiet_hours`
   (`{start, end}`, `HH:MM`, may cross midnight) silences speech and sound. Notifications
   follow macOS Focus on their own: macOS holds them back. An app cannot read the Focus
   state without Full Disk Access, so speech and sound use quiet hours instead.

Channels: speech (`/usr/bin/say`, an optional voice), a system sound (`/usr/bin/afplay`), a
notification (`UNUserNotificationCenter` when the process runs from an app bundle, else
`/usr/bin/osascript` with the text passed as arguments, never in the script). Speech and sound
are child processes with an argv list. A process started by cargo gets the silent
`RecordedCues` unless `QUADCAM_CUES=real`; the gate, the reminders and the batch cue are
tested on a fake clock. `gear_status` reports the links a job holds (`working`) and the armed
reminders (`reminders`); `gear_dismiss_reminder` stops one by link. A hold, its release, a
job's end and a dismiss send `gear-changed`.

### 7.12 Blackbox pull and erase (task BB)

The FC's flash fills with logs and Betaflight then stops logging. QuadCam pulls the logs off,
keeps them, and clears the flash, so the next flights are logged. Replaces the radio's
BLACKBOX ERASE switch habit.

**Read.** `MSP_DATAFLASH_SUMMARY` gives ready, supported, total and used bytes. The read asks
for the used bytes only, as `MSP_DATAFLASH_READ` over MSP v2: address (u32), size (u16, 4096),
compression flag (u8, always 0); the reply is address, size sent, a compressed flag and the data.
A reply with the flag set is an error, not data: QuadCam has no decompressor, and the real one
is Huffman-coded with a table it would have to reproduce. A short reply continues where it ended;
a failed chunk is asked again twice; a gone port fails the job at once. Measured 2026-10-07: about
84 KB/s, so 16 MB takes 3.3 minutes. API 1.40 or later is required; older FCs refuse.

**Order of the job** (`core/blackbox.rs`, one `fc_job`, one cue at the end):

1. Refuse a port another process holds (`Env.holders`) and any FC that is not Betaflight.
2. Identify over MSP, read the summary. 0 used: nothing to pull.
3. Heat check. The USB timer (7.11) gives the seconds left with a battery in. The read takes
   `used / 84 KB/s`. If it does not fit, refuse (`usb_heat`) unless `force`.
4. Read: MSP, or the USB disk path when `gearBlackboxMsc` allows (below).
5. Verify (`bf::blackbox::check_image`): size equals used, the data starts with a log header,
   each log has a firmware line. A failure stores nothing and erases nothing.
6. Store: under the blob store's lock, `put` the image, read it back and compare, write the
   record. The same bytes as the device's latest unerased pull add no record.
7. Erase, only if `gearEraseBlackbox` is on and `keep` is not set, and only after 1 to 6:
   - the USB timer must hold the erase estimate (4 s per MiB of flash, 20 to 120 s) plus 10 s,
     else the erase is skipped and the result and the record say why (a forced pull ends here);
   - the summary is read again; if used changed since the read, skip;
   - send `MSP_DATAFLASH_ERASE`, poll the summary until ready with 0 used, for at most twice
     the estimate (capped by `Timing::erase_max`); a timeout fails the job and the record keeps
     the reason. The pull stays stored.
8. Release the port. The cue "done, safe to unplug" plays after step 7 ends.

**Consent.** Two switches, like deleting clips after import. `gearOnConnect` lists `blackbox`
for a kind to pull on plug-in (off by default). `gearEraseBlackbox` (off by default) allows the
erase. A call can only turn the erase off for its run (`keep`); nothing on the CLI or in MCP turns
it on. A manual erase (`gear_blackbox_erase`, `confirm=true`) needs a stored pull whose used size
equals the flash's now (the flash only grows) and whose blob reads back. The owner's request was an
automatic pull then wipe. The setting defaults to off because the request names the
`delete_clips_after_import` pattern, which is off, and a wipe cannot be undone. Turn it on in
Settings > Gear to get the request as asked.

**Dates and flights.** FCs have no clock: every log reads `0000-01-01`. A pull's day is the day it
ran. `gear/blackbox.rs::link` pairs a pull's logs with the flights of the FC's aircraft (radio
logs, 7.6) that ended before the pull and started after the device's last erased pull, by order:
the newest flight-sized log (32 KB or more) with the newest flight, and so on back. Extra logs or
flights stay unpaired. Each pair carries `fits`: bytes per second of flight within half to double the
median pair (three pairs or more). The result carries a note that this is a guess. Matching the
blackbox RC traces with the radio's 10 Hz logs needs the decoder and is not done here.

**Header reader, not a decoder.** `bf/blackbox.rs` reads `H name:value` lines: firmware revision,
craft name, start date, loop time. It decodes no frames (sim-design BB keeps that). Written from
the public format description; the GPL decoder was not read.

**USB disk path (needs a real-FC trial).** With `gearBlackboxMsc` (or `mode=msc`): enter the CLI,
`help` must list `msc`, send `msc` (the FC reboots as a disk), wait for a new volume that holds
`.bbl` or `.bfl` files, copy them in name order into one image, unmount the disk, wait for the
port. The same verify (step 5) applies, so a file layout that does not add up to the used size
fails and nothing is erased. If the port does not come back the pull is stored and the erase is
skipped with the reason. Without `msc` in the CLI the pull falls back to MSP (`mode=msc` fails
instead). Unknown on real hardware: the disk's layout and names, its speed, whether eject or a
replug returns the FC to serial, and the extra reboot's cost against the heat timer. Tests use
`FakeFc::with_msc` and a temporary folder as the disk.

**Pause and other programs.** The on-connect step skips a port whose reads are paused
(`gear_poll_pause`). Every pull checks `Env.holders`, and a real open refuses a port another
process has (`serial::other_holders`). A manual pull of a paused port runs.

**Surfaces.** App: the FC page's Blackbox segment (Pull blackbox, Erase flash with a prompt, the
pulls and their logs); Settings > Gear. CLI: `gear blackbox pull|list|export|erase`. MCP:
`quadcam_gear blackbox`, `quadcam_gear_edit blackbox_export`, `quadcam_gear_apply blackbox_pull|
blackbox_erase`. Rows: `gear_blackbox_pull`, `gear_blackbox`, `gear_blackbox_export`,
`gear_blackbox_erase`.

## 8. Safety model

**Built (WP5a, FC side).** `gear/changes.rs` (the change store under `<gear>/changes/<id>/`,
`render_fc`: edits to CLI lines, the diff and the before state, and `restore_lines`),
`gear/apply.rs` (types), `gear/apply/fc.rs` (the plan, its checks, the digest, the range
check), `core/apply.rs` (stage, update, discard, restore, plan, apply as one FC job). Rows:
`gear_changes`, `gear_change_stage`, `gear_change_update`, `gear_change_discard`,
`gear_restore_stage`, `gear_apply_plan`, `gear_apply`. The sheet's own click is the Tauri command
`gear_apply_click`; `Hooks::confirm_apply` asks the app's sheet for any other caller (events
`agent-apply-request` and `agent-apply-closed`, the command `answer_apply_request`). Deviations:

- **The plan reads no CLI.** The before state is the touched settings in the device's latest
  `dump all` backup, so a plan needs no reboot. The apply takes a fresh `before_apply` backup
  and recomputes the digest from it; a difference is `before_mismatch`. The digest covers the
  device id, the dump's board, firmware, version and build, the before values and the lines.
- **Range check at apply, not plan.** `get NAME` needs the CLI, so the check runs in the
  apply's first CLI session before the first line (`bf::run_with`'s `precheck`). The plan checks
  that names exist.
- **USB heat** is a check of its own (`usb_heat`): the FC has a battery in and its USB timer
  has run out. No device is `no_device`. These two codes are new.
- **`gear_apply_plan` and `gear_apply` take an optional `port`** for the several-FC case.
- **After an apply** the engine stores a `Trigger::AfterApply` backup from the read-back
  session, so the next plan compares with the state the FC holds now.
- **Profile selection.** A line in a profile gets its `profile N` select, and the FC's own
  selection is put back at the end (a saved selection changes the active profile).
- **Restore** works on FC backups: it sets back every `set`, and the list-like commands
  (modes, adjustments, features, beepers, LEDs, VTX and mixers) that differ from the latest
  dump.
- **Not in 5a** (built in 5b, below): card apply, the Bench page and statuses (Try, Read
  first, Keep/Revert), the mount cycle, copying settings between quads, the `apply_ready`
  automation, the `OsdElement` writer.

**Built (WP5b, card side and Bench).** `gear/apply/card.rs` (the card plan and checks, the
digest, `roll_back`, `verify`), `core/apply_card.rs` (locate and mount a card, plan, apply,
the user's Mount), `core/bench.rs` (Keep, Revert, copy, the `apply_ready` step),
`gear/copy.rs` (copy between quads, pure), `Card::plan_files` and `card::attach`
(`diskutil mountDisk`) in the EdgeTX engine. Rows: `gear_change_keep`, `gear_change_revert`,
`gear_copy_plan`, `gear_copy_stage`, `gear_card_mount`, `gear_card_unmount`;
`gear_apply_plan` and `gear_apply` take a radio change too. UI: the Bench page, the copy
dialog, Mount and Done on a radio's page. Deviations and choices:

- **One path, two plans.** `gear_apply_plan` and `gear_apply` look at the change's device:
  an FC goes the 5a way, a radio goes `plan_card`. The card digest covers the card's id, board
  and version, and for each file the path and the hashes of the bytes before and after.
- **Card checks** (`gear/apply/card.rs`, in order): Read first, one card plugged in, same card
  as planned (`device_changed`), card check state (`card_check`), nothing else writing
  (`port_busy`, a job runs on the link), then the engine's own: known version
  (`unknown_version`, `unknown_board`), shape understood, round trip, model identity (the
  expected name: "wrong card?"), values in range, selected model kept, and something to write.
  New codes: `card_check`, `read_first`, `incompatible` (copy).
- **Backup.** A full card snapshot (`BeforeApply`, always kept; WP4's size and mtime skip keeps
  it cheap), then the writer's backup callback checks that the snapshot holds each touched
  file's exact bytes. A failed snapshot is `no_backup`, nothing written.
- **Write, verify, roll back.** The engine writes each file (temporary name, `F_FULLFSYNC`,
  rename, read-back). The engine puts back only the file whose read-back failed, so on any write
  failure, a stop, or a failed second read (`verify`), `roll_back` puts every touched file back
  to the planned bytes and removes new ones. Steps shown: Back up, Write, Roll back (when it
  ran), Read back, Verify. After a verified apply an `AfterApply` snapshot is taken. The card
  `ApplyReport.files` lists the files written and deleted: a Revert restores those.
- **Mount cycle.** `Env.mount` (`diskutil mountDisk`) joins `Env.unmount`. Cards stay unmounted
  between jobs. `locate_card` finds a card mounted, else mounts one that `gear_released` holds
  while the system still shows its disk (`gear_released` is fed by every release and by the app
  poll's unmounted list). A plan that mounted a card unmounts it quietly (no cue); an apply ends
  in `gear_finish_card` like every card job (one cue, "safe to unplug" only after the unmount;
  a refusal plays no cue). The person's Mount (`gear_card_mount`) keeps a card mounted for 10
  minutes (`MOUNT_MINUTES`, a constant; `gear_mount_tick` in the app's poll unmounts it) or
  until `gear_card_unmount`. `GearStatus.mounted` lists those cards.
- **Mount cycle for every card job (shell gaps).** Decided 2026-10-09: every operation on any
  card or card-like volume mounts it if needed, works, and unmounts it when done. One helper:
  `locate_volume` (`locate_card` is its radio-only form) with `unmounted_cards`,
  `mount_picked` and `card_for_job` in `core/apply_card.rs`. Backup, card check, repair,
  `gear_card`, `gear_card_preview` and `gear_card_clean` resolve their target through it (a
  card unmounted since its last job is mounted; a view or preview unmounts it quietly). The
  import session's card follows the same cycle in `core/session_card.rs`: `stage` and `load`
  take a `device` id, `import` unmounts the card after "Delete clips after import" and reports
  `ImportOutcome.card`, and `format_plan`, `format`, `eject` and card prep mount it first.
  A DJI device over USB is not unmounted. Lists (`gear_connected`, Pack up, flight sources) never
  mount a card.
- **Restore on a card** names the files (`Edit::Restore { paths }`) and stands alone: the plan
  makes each path read as in the backup (a path the backup lacks is removed), through
  `Card::plan_files`. A whole-card restore is not offered.
- **Statuses.** Try, Read first and Draft are set by hand (`gear_change_update`). A Read first
  change refuses at plan time (`read_first`) until the person marks it Ready. A Try change that
  verifies becomes `Applied` (the report still says verified); Keep makes it Verified; Revert
  stages a restore (`StagedChange.reverts` names the original) and the original becomes
  Reverted when that restore verifies. **An FC revert stages only the inverse of that change's
  lines** (WP8 sync): each `set` the change sent takes its value from the `dump all` taken
  just before its apply (`inverse_lines`, as `FcLines`), and each list-like line its old line.
  A line the pre-apply dump does not hold is left alone and named in the note. A later applied
  change that set the same lines is named in a warning in the revert's note: the revert undoes
  those values too. A card revert still restores the files the apply wrote.
- **Copy settings** (`gear/copy.rs`): parts rates, pid, osd, modes, adjustments, vtx and
  features, plus settings by name, from a device's latest backup or a named backup to an FC's
  latest backup. Checks: same firmware, same year.month release, something picked, something
  differs (`incompatible`). Per-quad values (names, accelerometer trims, battery and current
  calibration) are never copied; with different boards the board-bound settings are left out;
  settings and lines the target lacks are listed. The result is `FcSet` and `FcLines` edits on
  one staged change, so the FC plan and apply run as usual.
- **`apply_ready`** is the on-connect step "Apply ready changes" (kinds fc and radio), off
  unless `gearOnConnect` lists it. It plans each Ready change and, when every check passes,
  calls `gear_apply`, which asks the sheet (`Hooks::confirm_apply`). With no window
  (`has_gui` false) the step skips: the headless `confirm_apply` accepts, so an unattended
  apply would skip the click.
- **`OsdElement`** now renders to `set osd_<element>_pos = N` (`render_fc`, keeping the
  variant bits), x 0-63, y 0-31, profiles 1-3; an empty list turns the element off. The WP7
  editor only has to stage it.
- **Not done:** a radio's Bench item cannot be staged from the app (no card editor yet:
  CLI and agents stage card edits); `CardFiles` (sound packs) apply since WP9; the mount time is
  not a setting.

### 8.1 One path for every write

Every write to a device, a sim or a card goes: stage → plan → checks → confirm → backup →
write → read back → verify → record. A restore is a staged change. A firmware flash is a
plan with the same checks. Nothing writes outside `gear/apply.rs`, `gear/firmware/` and
`disk::format_card`, and each runs every guard again right before its write, as
`format_card` does.

### 8.2 Checks

| Check | Refusal code | Reason shown |
|---|---|---|
| Known version | `unknown_version` | "Betaflight 4.3.2 is not proven; QuadCam reads it but does not write it." |
| Known board | `unknown_board` | "Board X is not proven." |
| Same device as planned | `device_changed` | "This is not the FC the change was planned for." |
| Same state as planned | `before_mismatch` | "The FC changed since the plan; plan again." (the digest covers identity, the before state and the edits) |
| Shape understood | `shape_unknown` | "model01.yml line 212 is not understood; nothing was written." |
| Round trip | `round_trip` | "QuadCam cannot rewrite model01.yml unchanged; nothing was written." |
| Backup taken | `no_backup` | "The backup failed: <why>. Nothing was written." |
| One port or DFU device | `several_devices` | "Two FCs are connected; pick one." |
| Sim closed | `sim_running` | "Quit Uncrashed first." |
| Firmware image | `bad_image` | "The file is not a firmware for this radio (size, board name or splash markers)." |
| Name exists, value in range | `bad_setting` | "`osd_cap_alarm` takes 0-20000." |
| Mixed ELRS majors | warning, not a refusal | "The receiver would run 4.x and the radio 3.x: they will not link." |

Refusals are exit code 3 on the CLI and an error with the code and the reason over MCP.

### 8.3 Confirm

| From | Confirm |
|---|---|
| The app | The apply sheet's **Apply** click. Return does not press it |
| The CLI | `--digest` from `--plan` and `--yes` |
| An agent | `digest` and `confirm=true`. With the app running, the person also clicks **Apply** |

### 8.4 After a write

- **FC:** nothing is saved unless every line succeeded. Verify reads `dump all`. A failed
  verify offers the restore.
- **Card:** each file read back and compared. On a mismatch QuadCam writes the backed-up bytes
  back at once and reports both.
- **Sim:** the file read back and parsed.
- **Flash:** after the radio reboots, QuadCam asks for USB Storage mode and reads `semver`;
  for ELRS, the device-info ping when available.

## 9. Testing strategy

### 9.1 Fail-safes (no real device in tests)

| Resource | Fail-safe |
|---|---|
| Serial ports | `serial::real_ports()` returns none in a process started by cargo unless `QUADCAM_SERIAL=real`; `serial::system()` gives `NoPorts`, which refuses every open |
| Presence (`/dev`, `ioreg`) | `events::presence()` returns none under cargo unless `QUADCAM_SERIAL=real` |
| Cues | `cues::system()` gives the silent `RecordedCues` under cargo unless `QUADCAM_CUES=real` |
| Modules (downloads, running tools) | `Fetch` and `Runner` traits; tests serve fixture archives from a temp folder |
| DFU, esptool | Calls go through a `Flasher` trait; a cargo process gets the recorder unless `QUADCAM_FLASH=real` |
| Sims | Paths derive from `HOME`; tests set a temp `HOME` |
| TTS | A cargo process gets the fake provider (a tone per text hash) unless `QUADCAM_TTS=real` |
| Network (firmware index, voice index) | A `Fetch` trait; tests serve files from fixtures |
| Card prep | The existing disk-image rule: only images the test made, `BusProtocol == "Disk Image"` asserted |

### 9.2 Fakes and fixtures

- **`FakeFc`:** answers `#`, `set`, `get`, `aux`, `adjrange`, `diff all`, `dump all`,
  `save` (the port vanishes for a set time), `exit`, and the MSP messages in 6.2. It injects
  errors: a rejected line, a lost port, a slow dump.
- **Recorded transcripts:** a dev-only recorder saves real sessions. A scrubber removes the
  UID, craft and pilot names, serial numbers and any `name` value before a transcript can
  be committed; a test fails when a fixture holds a UID-shaped value.
- **Synthetic SD cards:** a generator writes an EdgeTX card tree (radio and model files in
  both the 2.10 hand-edited layout and the 2.12 saved layout, LOGS, SOUNDS) with made-up
  model names. Disk-image tests mount one for detection.
- **Synthetic logs:** generated CSVs with known hover, sag, dropouts and mAh, so each measure
  has an exact expected value.
- **Golden files (insta):** YAML edits byte for byte, OSD renders as text, switch maps, rate
  curves, sim files (GVAS bytes, XML), analysis reports, MCP tool schemas.
- **Properties:** OSD encode/decode round trip; YAML parse-render identity over every fixture.
- **UI:** mock-core scenarios (`?mock=gear-connected`, `gear-apply`, `gear-refused`), Playwright
  specs by role and label, axe in both themes.

### 9.3 By hand, on real gear

Each release in section 11 lists what the owner tests. Nothing in CI needs a device.

## 10. Build plan: work packages

Each package owns the files listed; another package changes them only through a small,
agreed edit (one line in a module list or a table). Every package ends with green CI, its
docs, and its rows in `api`, CLI and MCP.

| Id | Title | Owns | Depends on | Group |
|---|---|---|---|---|
| WP1 | Gear foundation | `gear/mod.rs`, `store.rs`, `model.rs`, `compat.rs`, `serial.rs`, `detect.rs`, `events.rs`, `cues.rs`; `api/gear.rs`, `core/gear.rs`, `mcp/gear.rs` (three tools, empty action sets), `bin/quadcam-cli/gear/mod.rs`; settings keys; paths | – | 0 |
| WP2 | Betaflight link | `gear/bf/` (`cli.rs`, `msp.rs`, `dump.rs`, `fake.rs`) | WP1 | 1 |
| WP3 | EdgeTX card engine | `gear/edgetx/` (`yaml.rs`, `model.rs`, `card.rs`), the synthetic card generator | WP1 | 1 |
| WP4 | Backups and the store | `gear/blobs.rs`, `backup.rs`, `radiologs.rs`; retention, import of old backup folders, auto backup on connect; Backups segment and Storage page | WP1, WP2, WP3 | 2 |
| WP5 | Staged changes and apply. **Done (5a, 5b):** `changes.rs`, `apply.rs`, `apply/fc.rs`, `apply/card.rs`, `copy.rs`, the apply sheet, the Changes segment, the Bench page, the mount cycle, copy between quads, `apply_ready` | `gear/changes.rs`, `apply.rs`, `apply/fc.rs`, `apply/card.rs`; the apply sheet; Bench page | WP1, WP2, WP3, WP4 | 3 |
| WP6 | Switch map | `gear/switchmap.rs`, Switches segment | WP2, WP3 | 2 |
| WP7 | OSD | `gear/osd.rs`, OSD segment (view and editor) | WP2 (parse); WP5 to stage | 1 (pure part), 3 (editor) |
| WP8 | Rates and sims. **Done:** the Sims page and the sidebar "Out of date" badge | `gear/rates.rs`, `gear/sims/`, `apply/sim.rs`, Rates segment, Sims page | WP2, WP5 (plan/confirm pattern) | 2 (read), 3 (sync) |
| WP9 | Radio extras: voice and model editors. **Done except the ElevenLabs adapter, the carrier-sentence render and the full line list (see 7.4)** | `gear/voice/`, `resources/voice/`, Voice segment, `build-pack` and the pack index; `ModelOp` editors for checklists, telemetry screens, logging, timers, alarms and callouts; Checklists segment | WP3, WP5, WP14 | 4 |
| WP10 | Firmware and splash. **1.0 holds the version checks, the EdgeTX flash and the splash. The Betaflight flash plan and ELRS options and flashing are deferred to 1.1 (both wanted)** | `gear/firmware/`, `gear/splash.rs`, `gear/dfu.rs`, Firmware page, Splash segment | WP2, WP3, WP4, WP5, WP14 | 4 |
| WP11 | Card prep. **Done, with the app button** | `disk.rs` changes, `card_prep*` rows, `quadcam_format_card` `prep`, the Prepare card button | – (existing code) | 1 |
| WP12 | Flights and packs | `logs.rs` columns, `gear/flights.rs`, `gear/packs.rs`, Flights and Packs pages | WP1 (reads log folders; the log store once WP4 lands) | 1 |
| WP13 | Gear shell UI. **Done** | Sidebar Gear section, page frame and segments, Connected rows, plug-in bar, shared components (`DiffView`, `ChecksList`, `DeviceHeader`), mock-core scenarios | WP1 (types) | 1 |
| BB | Blackbox pull and erase (7.12). **Built; the USB disk path and every real-FC behaviour need a trial** | `gear/bf/blackbox.rs`, `gear/blackbox.rs`, `core/blackbox.rs`, Blackbox segment, `gear blackbox` | WP2, WP4, WP12 | 2 |
| WP14 | Modules and third-party notices | `modules/` (the module manager, 7.10), the Modules section in Settings, `media.rs` finding ffmpeg through it; `THIRD_PARTY_NOTICES` generated at build (`cargo about` or equivalent for crates, the pnpm license list for `app/`, the OFL text for the bundled fonts) and shipped in the `.app`; an About window entry; a CI check that fails on a dependency with no license or a license outside the allow list (MIT, Apache-2.0, BSD, ISC, MPL-2.0, OFL-1.1, Unicode, Zlib) | – | 1 |

**Parallel groups:** 0 → 1 → 2 → 3 → 4. WP14 (modules) runs in group 1 because firmware
(WP10) and the voice render (WP9, ffmpeg) need it. Inside a group, packages run at once. WP7 and WP8
split: their read-only halves run early; their write halves wait for WP5.

### Acceptance criteria

| Id | Accepted when |
|---|---|
| WP1 | `gear.json` passes the settings-file tests' equivalents (unknown keys kept, locked writes, CLI change seen by the app). `detect` lists a synthetic radio volume and a fake serial port. The three MCP tools exist and the schema snapshot is updated. `real_ports()` is empty under cargo. Events: an unmounted card reads `unmounted_present` until its disk node goes; QuadCam's own transitions are `app_initiated`; hooks run only when their automation is on; one cue per job, debounced, queued; the reminder waits, repeats, caps and stops; mute and quiet hours; all on a silent sink and a fake clock |
| WP2 | Against `FakeFc`: backup, a run that stops at an error and discards, a run with `save` that waits through the reboot, a `dump all` parse with section context. MSP identity read. Transcript scrubber test |
| WP3 | Parse-render is byte-identical on every fixture (both layouts, CRLF). Each encoding in 6.3 has a test. Unknown lines refuse with file and line |
| WP4 | A second snapshot of an unchanged card writes nothing. A changed model file adds one blob. FC backup through `FakeFc`. Retention keeps apply and pinned snapshots and thins the rest by the settings; collection removes only unreferenced blobs. A grown log replaces the stored one. Import of a synthetic folder of card copies and FC pairs dedupes. A crash between blob and manifest (simulated) leaves a consistent store |
| WP5 | Every check in 8.2 has a refusing test. FC apply verifies against `dump all`; a default-valued `set` verifies. Card apply rolls back on a read-back mismatch. MCP apply needs digest and confirm; with the app running, the click. Bench statuses and Keep/Revert |
| WP6 | Golden switch maps for two synthetic aircraft (3-position select, `REPL` lines, `adjrange`). Live highlight from `FakeFc` `MSP_RC` |
| WP7 | Round-trip property test; NTSC, PAL and HD golden renders; overlap and off-screen checks; an editor move stages the right CLI line |
| WP8 | Curves match reference values for Betaflight, Actual and Quick; Actual-to-Betaflight fit within a stated error; each sim adapter reads and writes a synthetic file byte-exact; refuses while "running" (faked) |
| WP9 | Spelling rules golden test; cache hit renders with no provider call; normalisation golden WAV; `build-pack` writes a zip and index entry; Choose voice stages one change that keeps overrides when asked. Each `ModelOp` golden on both layouts; checklist length and name rules; ownership replaces a previous change's items |
| WP10 | Splash patch and decode on a synthetic binary with markers; refusals for missing or doubled markers and for boards and versions not in `compat.rs`; EdgeTX flash plan picks the board binary; (1.1: ELRS options block written and read back, the Betaflight flash plan); flashes go to the recorder in tests; DFU against a fake `nusb` device: erase, write, read back, compare |
| WP11 | Disk-image test: prep refuses with a clip not in the library, passes otherwise; DJI refused |
| WP12 | Each measure in 7.6 matches the synthetic log's known values; pack history; old `LogRow` tests still pass |
| WP13 | Mock scenarios render; axe passes in both themes; Gear section collapses; plug-in bar appears for a device with staged changes |
| BB | Against `FakeFc`: the pull reads only the used bytes and verifies; the blob reads back; the erase runs only after verify and only with the setting on; a lying summary, junk data or a compressed reply stores and erases nothing; a stuck erase fails and keeps the pull; the heat timer refuses a long pull and skips an erase it cannot finish; a held or paused port is left alone; the USB disk path copies files and falls back; logs pair with flights by order and say guess; export and manual erase refuse as 7.12 says; a prune keeps the blobs |
| WP14 | Against a fixture server: download, checksum match and mismatch, install, run, update, remove, the license prompt (mock core). ffmpeg from the module and from Homebrew both pass the import tests. The built `.app` holds the notices file with every crate, npm package and font; the CI license check passes and fails on a planted bad license |

## 11. Release plan

Each release is tagged after its packages merge and CI is green. The owner tests on real
gear before the next release starts.

| Release | Packages | Owner tests on real gear |
|---|---|---|
| 0.7.0 Read-only gear | WP1, WP2, WP3, WP4, WP12, WP13, WP14 | Install the ffmpeg module in Settings > Modules and import a card with it; switch back to Homebrew. Import the old backup folders; check the Storage view's sizes. Plug the radio in (USB Storage): a backup appears, a second plug-in adds none. Plug the FC in: backup appears, the FC reboots normally, the dump reads right. Browse and diff backups. Flights for a recent day match the hand analysis (hover, sag, resting, mAh, dropouts) |
| 0.8.0 Map, OSD, rates, sims | WP6, WP7 (read), WP8 | Switch map matches the radio and the quad, position by position; live highlight while moving switches. OSD render matches the goggles for each profile. Sim sync to every installed sim; fly each and compare feel |
| 0.9.0 Staged changes | WP5, WP7 (editor), WP11 | Stage and apply one harmless FC change (an OSD move), check the goggles, revert it. Edit a radio timer name, apply, check on the radio, restore. Prep a spare DVR card |
| 0.10.0 Radio extras and voice | WP9 | Install a voice pack, Choose voice, hear the callouts on the radio. Override one line. Edit a checklist and a battery callout; check them on the radio |
| 0.11.0 Firmware | WP10 | Firmware check. Make firmware with a splash for the installed EdgeTX version, flash it, see the splash. Re-flash the installed ELRS version to the receiver and the radio; the link comes back |
| 1.0.0 | Fixes from the cycles, docs complete, MCP schema frozen | One full bench session done only in QuadCam: back up, apply the queue, verify, fly, analyse the flights |

## 12. Licensing

**Working assumption: QuadCam's code stays MIT** (licensing review, 2026-10-07). Upstream
licenses: Betaflight GPL-3.0-or-later, EdgeTX GPL-2.0-only (its sound packs GPL-2.0),
ExpressLRS GPL-3.0, esptool GPL-2.0-or-later, dfu-util GPL-2.0. MIT holds as long as QuadCam
talks to them from the outside and copies none of their code.

| Constraint | How this design meets it |
|---|---|
| Talk to GPL firmware and tools only through protocols, files and separate processes | Serial CLI and MSP, SD-card files, `esptool` and ffmpeg as subprocesses, DFU as a USB class |
| Never copy GPL source; reimplement tables and formats from documentation and observed files | `bf/msp.rs`, `bf/dump.rs`, `edgetx/yaml.rs`, the OSD width table, the rate formulas and the ELRS options block are written from published formats, recorded transcripts and fixtures. A reviewer checks each package for copied code |
| Download official firmware at run time; never ship a patched binary | `gear/firmware` downloads from upstream, checks the hash, patches the splash in the cache on the user's Mac |
| Do not bundle `esptool` (or any GPL tool) in the `.app` | The module manager (7.10) downloads it from upstream on the user's request, after showing its license. QuadCam does not redistribute it |
| Do not copy EdgeTX's voice-list file | `lines.csv` is QuadCam's own text (7.4) |
| Voice packs under a separate asset license | Each pack zip and its index entry carry the license and attribution. The chosen path is a re-render on a paid ElevenLabs plan, released under CC BY 4.0. Earlier free-tier renders are never published |
| Notices for what the `.app` bundles | WP14: a generated third-party notices file (crates, npm packages, the OFL fonts) in the `.app`, and a CI license check |
| Serial crate | serialport-rs (MPL-2.0): file-level copyleft only, fine in an MIT app as an unmodified dependency |

**Where the license decision could change the approach:**

| Area | MIT approach (this doc) | If QuadCam went GPL |
|---|---|---|
| ELRS options block | Own implementation from the format | Could port the ELRS configurator code |
| Betaflight rate formulas, OSD element widths, MSP tables | Written from published formulas, measured widths, the protocol description | Could port from Betaflight source |
| EdgeTX YAML encodings | From the file format and fixtures | Could port EdgeTX's YAML tables |
| Voice line text | Own text | Could ship EdgeTX's list |
| esptool, ffmpeg | Modules downloaded from upstream, run as separate processes | Could bundle them |
| DFU | Native, from the USB DFU specification | Could port dfu-util |
| Splash patch | Own (marker search, bit packing) | No change |

## 13. Open questions for the owner

Each has a default the build uses until you decide.

| # | Question | Default |
|---|---|---|
| 1 | Velocidrone's save format: can you share a sample save once it is installed? | Decided 2026-10-09: the adapter ships disabled until a sample save exists; the Sims page says so |
| 2 | ESP flashing: the `esptool` module (proven tool) or embed the `espflash` crate (no download, ESP8266 support uncertain)? | `esptool`, a downloaded module (decided) |
| 3 | Card prep for DJI goggles cards? | Decided 2026-10-07: a removable goggles card may be prepped as exFAT once every clip is in the library; a DJI device over USB is never formatted (`AGENTS.md`, Rules) |
| 4 | Auto backup of an FC reboots it (the CLI `exit`). Keep auto backup on for FCs, or MSP identity only and a manual full backup? | On; skipped while another app holds the port |
| 5 | Gear folder: the support folder (this Mac only) or inside the library folder (moves with it)? | Support folder; `gearDir` moves it |
| 6 | Voice packs: confirm CC BY 4.0 for the paid re-render, the attribution text, and the voices to render besides Callum | CC BY 4.0; no pack ships before the paid re-render |
| 7 | Write the pack label and flight analysis into clip files as QuickTime items? | No in 1.0; shown from the flight index |
| 8 | Betaflight firmware flashing: out of scope for 1.0 (version check only)? | Decided 2026-10-09: deferred to 1.1, and wanted. 1.0 checks the version only |
| 9 | ELRS version read over CRSF device info needs a hardware check. Until then, enter versions by hand? | Decided 2026-10-09: ELRS options and flashing are deferred to 1.1, and wanted. Until the hardware check, hand entry and a read-only check |
| 10 | Splash source: GitHub release binaries only, or also the EdgeTX cloud build? | Release binaries only |
| 11 | Firmware and voice indexes go online. Check only on request, or daily? | On request (`firmwareCheck` = `manual`); `README.md` Privacy updated |
| 12 | ffmpeg as a module: which static arm64 build (signed, LGPL preferred), and drop the cask's Homebrew `ffmpeg` dependency once the module works? | Module by default, Homebrew fallback. Decided 2026-10-09: the cask drops its `ffmpeg` dependency for 1.0, and the first run offers the module (a banner, with the license and the size first; `docs/modules.md`). WP14 pins Martin Riedl's signed, notarized 9.0.2 arm64 release build (GPL; no signed LGPL arm64 build found; `docs/modules.md`) |
