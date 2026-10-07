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

An FC on USB gets a session timer in its row ("USB 12 min"). At the limit set in Settings
(default 20 min) QuadCam shows a notification: small quads overheat when powered on USB.

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
| Steps on connect | `gearOnConnect` | per device kind: `backup` only (`import`, `apply_ready` off) (7.11) |
| Cues | `gearCues` | speech and notifications on, sound off; each cue on; reminder every 60 s (7.11) |

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
| `gear/bf/dump.rs` | Parses `dump all` and `diff all` into a typed config with section context (`profile N`, `rateprofile N`); renders CLI lines back | none |
| `gear/bf/fake.rs` | `FakeFc`: a CLI and MSP simulator seeded from a synthetic dump (tests and the mock core) | `bf/dump` |
| `gear/edgetx/yaml.rs` | Line-level YAML reader and editor for EdgeTX files (section 6.3). Never a generic YAML round-trip | none |
| `gear/edgetx/model.rs` | Typed views of a model file: header, timers, mixes, logical switches, special functions, switch warnings, telemetry sensors and screens, module settings | `edgetx/yaml` |
| `gear/edgetx/card.rs` | The SD card: `RADIO/radio.yml`, `MODELS/`, `SOUNDS/`, `SCRIPTS/`, `LOGS/`; identity (board, `semver`) | `edgetx/model` |
| `gear/compat.rs` | The table of proven versions: EdgeTX boards and versions, Betaflight versions, ELRS targets, splash layouts, sim file versions. Data, reviewed per release | none |
| `gear/blobs.rs` | The content-addressed blob store: put, get, verify, garbage-collect (section 7.1) | `store`, `xxhash-rust` |
| `gear/backup.rs` | Takes, lists, reads, diffs, retains and prunes snapshots; imports old backup folders | `blobs`, `bf`, `edgetx` |
| `gear/radiologs.rs` | Stores each radio log once, outside snapshots; feeds `flights` | `blobs`, `logs` |
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
| Radio logs | `<gear>/logs/<radio-id>/<file name>.csv` → blob | Each log stored once, not part of any snapshot (section 7.1) |
| Staged changes | `<gear>/changes/<YYYY-MM-DD>-<device-slug>-<n>/` | `change.json`, `before/`, `after/`, `apply.cli` or file diffs, `report.json`. The same shape as a hand-kept staging folder |
| Bench history | `<gear>/changes/` (applied ones) | The log of what was applied, verified or reverted |
| Downloads | `~/Library/Caches/app.quadcam/firmware/<product>/<version>/` | With a SHA-256 per file |
| Voice renders | `~/Library/Caches/app.quadcam/voice/raw/<provider>/<key>.pcm` | Raw takes; a re-render costs nothing |
| Installed voice packs | `<gear>/voices/<pack-id>/` | WAVs and `pack.json` |
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
| `gear_backup` | `BackupParams { device }` → `Backup` | device read, gear folder |
| `gear_backups` | `BackupFilter` → `Vec<Backup>` | no |
| `gear_backup_read` | `BackupReadParams { id, path }` → `BackupContent` | no |
| `gear_backup_diff` | `BackupDiffParams { a, b, path }` → `Vec<DiffItem>` | no |
| `gear_storage` | – → `StorageView` (totals, per device, snapshot counts) | no |
| `gear_prune` | `PruneParams { dry_run }` → `PruneReport` | gear folder |
| `gear_export` | `ExportParams { device or snapshot, to }` → `ExportReport` | a folder the user picked |
| `gear_import_backups` | `ImportBackupsParams { folder, device, dry_run }` → `ImportBackupsReport` | gear folder |
| `gear_switch_map` | `AircraftParams { aircraft, live }` → `SwitchMap` | no |
| `gear_osd` | `OsdParams { device or backup or change, grid }` → `OsdView` | no |
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
| `gear_flights` | `FlightFilter { day, aircraft, pack, place, logs }` → `Vec<FlightReport>` | flights cache |
| `gear_flight_set` | `FlightSetParams { flight, pack, place }` → `FlightReport` | gear.json |
| `gear_packs` | – → `PacksView` (packs, types, charging sheet, history) | no |
| `modules` | – → `Vec<ModuleStatus>` (installed, pinned, newest, license) | no |
| `module_install` | `ModuleParams { name, confirm }` → `ModuleStatus` | modules folder; network |
| `module_remove` | `NameParams` → `ModuleStatus` | modules folder |
| `modules_check` | – → `Vec<ModuleStatus>` | network |
| `gear_pack_save` / `gear_pack_delete` | `PackSaveParams` / `NameParams` → `Pack` | gear.json |

Events (`api/events.rs`): `DeviceChanged` (connect, disconnect, backup state), `ApplyProgress`,
`RenderProgress`, `FlashProgress`.

### 5.2 CLI

`quadcam-cli gear <command>`, with `--json` and the existing exit codes (3 = refused by a
guard, 4 = no device). The subcommands live in `bin/cli/gear/*.rs`, one file per area, so
work packages do not edit one shared file.

```bash
quadcam-cli --json gear devices
quadcam-cli --json gear backup --device <id>|--port /dev/cu.usbmodemX|--mount /Volumes/RADIO
quadcam-cli --json gear backups [--device <id>]
quadcam-cli --json gear backup show <backup> [PATH]
quadcam-cli --json gear backup diff <a> <b> [PATH]
quadcam-cli --json gear storage [--prune [--dry-run]] [--export <device|snapshot> DIR]
quadcam-cli --json gear import-backups FOLDER [--device <id>] [--dry-run]
quadcam-cli --json gear map --aircraft NAME [--live]
quadcam-cli --json gear osd --device <id> [--grid PAL|NTSC|HD] [--text]
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
quadcam-cli --json gear packs [save LABEL --type T | delete LABEL]
quadcam-cli --json modules [check | install NAME --yes | remove NAME]
```

### 5.3 MCP: three new tools

The server has 16 tools. Gear adds three, split by what they can change. An agent harness
can then allow the read tool freely and gate the other two.

| Tool | Changes | Actions |
|---|---|---|
| `quadcam_gear` | Nothing | `status`, `devices`, `storage`, `backups`, `backup_read`, `backup_diff`, `switch_map`, `osd`, `rates`, `sims`, `changes`, `apply_plan`, `voice`, `firmware_check`, `flights`, `packs` |
| `quadcam_gear_edit` | QuadCam's own data only: never a device, a sim or a card | `device_save`, `device_forget`, `stage`, `update`, `discard`, `restore_stage`, `voice_edit`, `voice_render`, `voice_choose`, `pack_save`, `pack_delete`, `flight_set`, `backup` (a read of the device; writes only to the gear folder), `import_backups`, `prune`, `export` |
| `quadcam_gear_apply` | A device, a sim or the radio firmware | `apply`, `sim_sync`, `flash`. Each needs the `digest` from a plan and `confirm=true` |

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

MSP serves identity and the live view. It never writes in 1.0.

| Need | Messages |
|---|---|
| Identity without entering the CLI (no reboot) | `MSP_API_VERSION`, `MSP_FC_VARIANT`, `MSP_FC_VERSION`, `MSP_BOARD_INFO`, `MSP_BUILD_INFO`, `MSP_UID` |
| Live switch map | `MSP_RC` (channel values), `MSP_BOXIDS` + `MSP_MODE_RANGES` + `MSP_ADJUSTMENT_RANGES`, the active-modes flags in `MSP_STATUS_EX` |
| Live overview | `MSP_ANALOG`, `MSP_BATTERY_STATE` |

- v1 frames (`$M<`, XOR checksum) for the classic messages, v2 (`$X<`, CRC8 DVB-S2) where
  needed. Message ids come from the public MSP protocol description and are checked against
  a recorded transcript per supported version.
- CLI and MSP do not run at once. Entering the CLI ends the MSP session; leaving it reboots.
  `gear/bf` holds one state machine per port: Idle, Msp, Cli, Rebooting.
- The device id is a hash of `MSP_UID`. The raw UID never leaves the machine and never goes
  into a fixture.

**What each feature needs:**

| Feature | CLI | MSP |
|---|---|---|
| Backup, restore | yes | – |
| Staged FC changes (rates, OSD, battery, modes, adjustments, launch control, any `set`) | yes | – |
| Verify | yes (`dump all`) | – |
| Identity, version guard | `version` as a fallback | yes |
| Switch map, live | – | yes |
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
| Liftoff | `~/Library/Application Support/LuGus Studios/Liftoff/Saves/Player/UserData.xml` | XML; each rate profile its own `<rateProfiles>` sibling | Betaflight style, values × 100, no throttle curve |
| Liftoff: Micro Drones | Inside the game's app bundle: `Contents/Saves/Player/UserData.xml` | XML; profiles wrapped in `<FlightRatesProfile>` | As Liftoff |
| Uncrashed | `~/Library/Application Support/Uncrashed/<id>/rates/<NAME>.sav` | Unreal GVAS. After `FloatProperty\0` and one byte, an i32 count (12) and 12 little-endian f32 | Per axis (roll, pitch, yaw): super, RC rate, expo; then rates type (0 = Betaflight), throttle mid, throttle expo. A new profile is a copy of an existing file with the floats replaced; the name is the file name. An empty profile has no floats |
| The Zone | `~/Library/Application Support/Godot/app_userdata/The Zone/settings.cfg` | Godot config; `[rate_profile_0]`, `type "betaflight"` | Each axis `Vector3(RC rate, super rate, expo)` |
| Velocidrone | Unknown | – | Adapter ships disabled until a sample save exists (open question 1) |

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
the backup the apply took. Applied items move to History. **Copy as Markdown** exports the
queue.

### 7.8 Packs and charging

Packs are user data: label, pack type, received date, retired. Pack types hold chemistry
(LiPo, LiHV), cells, capacity, connector, and charge settings (full, storage, rate). The
Charging view is a table of pack types with their settings, plus free notes. Nothing in the
repo names a real pack or a shop.

### 7.9 Card prep

Formats a DVR or goggles card that has no session: a new card, or a card whose clips are all
in the library. Guards: every existing `disk::format_card` guard, plus "no clip on the card is
missing from the library" (the sidebar's "N new" is 0). The file system and the label come
from the source's `CardPolicy`. DJI cards stay refused, as today (open question 3).

Built in WP11 before WP1 landed: the `Core` methods live in `core/prep.rs`, the two rows in
`api/mod.rs`, and the CLI is `format --prep`. They move to `core/gear.rs`, `api/gear.rs` and
`gear card-prep` once those exist. The app has no card-prep button yet (WP13).

### 7.10 Modules

Tools and firmware that QuadCam does not ship become **modules**: QuadCam downloads them from
their upstream, on the user's request, into its own folder. A user needs nothing installed
elsewhere, and the `.app` bundles nothing GPL. QuadCam fetches; it does not redistribute.

**Manifest.** `resources/modules.toml` pins each tool module per QuadCam release:

```toml
[esptool]
version = "5.1.0"
url = "https://github.com/espressif/esptool/releases/download/v5.1.0/esptool-v5.1.0-macos-arm64.tar.gz"
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

- QuadCam downloads with its own HTTP client, so the files carry no quarantine attribute.
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
(`detect::parse_card_reader`) and uses it for a card in the built-in slot; other cards use
the volume UUID.

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
`Automation` (`backup`, `import`, `apply_ready`), the device kinds, and the function. The app
runs `Core::gear_on_connect` on its own thread for each `connected` and `identified` event.
A hook runs only when `gearOnConnect` lists its automation for the device's kind; `backup`
also needs `gearAutoBackup`. Defaults: `backup` for every kind, the others off. WP4 registers
backup, a later package import, WP5 `apply_ready` (which still goes through the plan, the
checks and the confirm in section 8; an automatic apply never skips them). A failed step plays
the `step_failed` cue.

**Cues** (`gear/cues.rs`). Short lines for a person with their hands full:

| Cue | When |
|---|---|
| `safe_to_unplug` | A device was released: a card unmounted and still in, or (later packages) an FC port closed after a step |
| `still_inserted` | Every `still_inserted_every_s` (default 60, 0 for none) after the unmount, until the card is pulled |
| `step_failed` | An on-connect step or a bench step failed |

Channels: speech (`/usr/bin/say`, an optional voice), a system sound (`/usr/bin/afplay`), a
notification (`/usr/bin/osascript`, the text passed as arguments, never in the script). Each
is a child process with an argv list. `gearCues` turns each cue and each channel on or off.
A process started by cargo gets the silent `RecordedCues` unless `QUADCAM_CUES=real`. The
native notification API can replace `osascript` with the Gear UI (WP13).

## 8. Safety model

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
| WP1 | Gear foundation | `gear/mod.rs`, `store.rs`, `model.rs`, `compat.rs`, `serial.rs`, `detect.rs`, `events.rs`, `cues.rs`; `api/gear.rs`, `core/gear.rs`, `mcp/gear.rs` (three tools, empty action sets), `bin/cli/gear/mod.rs`; settings keys; paths | – | 0 |
| WP2 | Betaflight link | `gear/bf/` (`cli.rs`, `msp.rs`, `dump.rs`, `fake.rs`) | WP1 | 1 |
| WP3 | EdgeTX card engine | `gear/edgetx/` (`yaml.rs`, `model.rs`, `card.rs`), the synthetic card generator | WP1 | 1 |
| WP4 | Backups and the store | `gear/blobs.rs`, `backup.rs`, `radiologs.rs`; retention, import of old backup folders, auto backup on connect; Backups segment and Storage page | WP1, WP2, WP3 | 2 |
| WP5 | Staged changes and apply | `gear/changes.rs`, `apply.rs`, `apply/fc.rs`, `apply/card.rs`; the apply sheet; Bench page | WP1, WP2, WP3, WP4 | 3 |
| WP6 | Switch map | `gear/switchmap.rs`, Switches segment | WP2, WP3 | 2 |
| WP7 | OSD | `gear/osd.rs`, OSD segment (view and editor) | WP2 (parse); WP5 to stage | 1 (pure part), 3 (editor) |
| WP8 | Rates and sims | `gear/rates.rs`, `gear/sims/`, `apply/sim.rs`, Rates segment, Sims page | WP2, WP5 (plan/confirm pattern) | 2 (read), 3 (sync) |
| WP9 | Radio extras: voice and model editors | `gear/voice/`, `resources/voice/`, Voice segment, `build-pack` and the pack index; `ModelOp` editors for checklists, telemetry screens, logging, timers, alarms and callouts; Checklists segment | WP3, WP5, WP14 | 4 |
| WP10 | Firmware and splash | `gear/firmware/`, `gear/splash.rs`, `gear/dfu.rs`, Firmware page, Splash segment | WP2, WP3, WP4, WP5, WP14 | 4 |
| WP11 | Card prep | `disk.rs` changes, `card_prep*` rows, `quadcam_format_card` `prep` | – (existing code) | 1 |
| WP12 | Flights and packs | `logs.rs` columns, `gear/flights.rs`, `gear/packs.rs`, Flights and Packs pages | WP1 (reads log folders; the log store once WP4 lands) | 1 |
| WP13 | Gear shell UI | Sidebar Gear section, page frame and segments, Connected rows, plug-in bar, shared components (`DiffView`, `ChecksList`, `DeviceHeader`), mock-core scenarios | WP1 (types) | 1 |
| WP14 | Modules and third-party notices | `modules/` (the module manager, 7.10), the Modules section in Settings, `media.rs` finding ffmpeg through it; `THIRD_PARTY_NOTICES` generated at build (`cargo about` or equivalent for crates, the pnpm license list for `app/`, the OFL text for the bundled fonts) and shipped in the `.app`; an About window entry; a CI check that fails on a dependency with no license or a license outside the allow list (MIT, Apache-2.0, BSD, ISC, MPL-2.0, OFL-1.1, Unicode, Zlib) | – | 1 |

**Parallel groups:** 0 → 1 → 2 → 3 → 4. WP14 (modules) runs in group 1 because firmware
(WP10) and the voice render (WP9, ffmpeg) need it. Inside a group, packages run at once. WP7 and WP8
split: their read-only halves run early; their write halves wait for WP5.

### Acceptance criteria

| Id | Accepted when |
|---|---|
| WP1 | `gear.json` passes the settings-file tests' equivalents (unknown keys kept, locked writes, CLI change seen by the app). `detect` lists a synthetic radio volume and a fake serial port. The three MCP tools exist and the schema snapshot is updated. `real_ports()` is empty under cargo. Events: an unmounted card reads `unmounted_present` until its disk node goes; hooks run only when their automation is on; cues follow `gearCues`, tested on a silent sink |
| WP2 | Against `FakeFc`: backup, a run that stops at an error and discards, a run with `save` that waits through the reboot, a `dump all` parse with section context. MSP identity read. Transcript scrubber test |
| WP3 | Parse-render is byte-identical on every fixture (both layouts, CRLF). Each encoding in 6.3 has a test. Unknown lines refuse with file and line |
| WP4 | A second snapshot of an unchanged card writes nothing. A changed model file adds one blob. FC backup through `FakeFc`. Retention keeps apply and pinned snapshots and thins the rest by the settings; collection removes only unreferenced blobs. A grown log replaces the stored one. Import of a synthetic folder of card copies and FC pairs dedupes. A crash between blob and manifest (simulated) leaves a consistent store |
| WP5 | Every check in 8.2 has a refusing test. FC apply verifies against `dump all`; a default-valued `set` verifies. Card apply rolls back on a read-back mismatch. MCP apply needs digest and confirm; with the app running, the click. Bench statuses and Keep/Revert |
| WP6 | Golden switch maps for two synthetic aircraft (3-position select, `REPL` lines, `adjrange`). Live highlight from `FakeFc` `MSP_RC` |
| WP7 | Round-trip property test; NTSC, PAL and HD golden renders; overlap and off-screen checks; an editor move stages the right CLI line |
| WP8 | Curves match reference values for Betaflight, Actual and Quick; Actual-to-Betaflight fit within a stated error; each sim adapter reads and writes a synthetic file byte-exact; refuses while "running" (faked) |
| WP9 | Spelling rules golden test; cache hit renders with no provider call; normalisation golden WAV; `build-pack` writes a zip and index entry; Choose voice stages one change that keeps overrides when asked. Each `ModelOp` golden on both layouts; checklist length and name rules; ownership replaces a previous change's items |
| WP10 | Splash patch and decode on a synthetic binary with markers; refusals for missing or doubled markers and for boards and versions not in `compat.rs`; EdgeTX flash plan picks the board binary; ELRS options block written and read back; flashes go to the recorder in tests; DFU against a fake `nusb` device: erase, write, read back, compare |
| WP11 | Disk-image test: prep refuses with a clip not in the library, passes otherwise; DJI refused |
| WP12 | Each measure in 7.6 matches the synthetic log's known values; pack history; old `LogRow` tests still pass |
| WP13 | Mock scenarios render; axe passes in both themes; Gear section collapses; plug-in bar appears for a device with staged changes |
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
| 1 | Velocidrone's save format: can you share a sample save once it is installed? | Adapter ships disabled |
| 2 | ESP flashing: the `esptool` module (proven tool) or embed the `espflash` crate (no download, ESP8266 support uncertain)? | `esptool`, a cask dependency |
| 3 | Card prep for DJI goggles cards: today QuadCam never formats a DJI card (`AGENTS.md`, Rules). Keep that? | Keep refused |
| 4 | Auto backup of an FC reboots it (the CLI `exit`). Keep auto backup on for FCs, or MSP identity only and a manual full backup? | On; skipped while another app holds the port |
| 5 | Gear folder: the support folder (this Mac only) or inside the library folder (moves with it)? | Support folder; `gearDir` moves it |
| 6 | Voice packs: confirm CC BY 4.0 for the paid re-render, the attribution text, and the voices to render besides Callum | CC BY 4.0; no pack ships before the paid re-render |
| 7 | Write the pack label and flight analysis into clip files as QuickTime items? | No in 1.0; shown from the flight index |
| 8 | Betaflight firmware flashing: out of scope for 1.0 (version check only)? | Out of scope |
| 9 | ELRS version read over CRSF device info needs a hardware check. Until then, enter versions by hand? | Hand entry, read-only check |
| 10 | Splash source: GitHub release binaries only, or also the EdgeTX cloud build? | Release binaries only |
| 11 | Firmware and voice indexes go online. Check only on request, or daily? | On request (`firmwareCheck` = `manual`); `README.md` Privacy updated |
| 12 | ffmpeg as a module: which static arm64 build (signed, LGPL preferred), and drop the cask's Homebrew `ffmpeg` dependency once the module works? | Module by default, Homebrew fallback; the cask keeps the dependency until 1.0. WP14 pins Martin Riedl's signed, notarized 9.0.2 arm64 release build (GPL; no signed LGPL arm64 build found; `docs/modules.md`) |
