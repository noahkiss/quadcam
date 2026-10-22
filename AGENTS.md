# QuadCam

A macOS desktop app (Tauri 2) that imports FPV clips into a library from two sources: analog
DVRs (MJPEG AVI) and DJI units (MP4 named by the unit's clock). It stages the clips off the card,
dates them (EdgeTX radio logs, the DJI clip clock, or the import date), names them
`YYYY-MM-DD_<name>.mp4`, converts analog clips with ffmpeg (a full DJI clip is copied, never
re-encoded), verifies them with ffprobe, and files them by flying day. It can then add them to
Photos and format a card with its source's file system (analog FAT32; card prep also a
removable DJI goggles card as exFAT). The library is the
home screen; import is a sheet over it. `README.md` is the public overview and `docs/` the
public user guide, one topic per file; keep them in step with every change a user can see.

**Publishable repo:** no personal information in anything committed. That means no names,
hostnames, user paths, or personal gear setups. Name specific devices only as examples in the
README and `docs/`. Personal preferences go in the app's settings file on the machine
(`~/Library/Application Support/app.quadcam/settings.json`), never in code defaults.

## Layout

| Path | Holds |
|---|---|
| `app/` | The frontend: React + TypeScript + Vite (pnpm, Node pinned in `.node-version`). Builds to `app/dist`, which the app ships. `views/` (library, clip detail, import sheet, settings, `Gear/` with its slots for later Gear packages: `segments.tsx`, `pages.tsx`, `slots.ts`, `DeviceLists.tsx` the Connected and Devices pages, `Overview.tsx` a device's Overview, `SaveDevice.tsx` the name and aircraft sheet, `Gear/Osd/` the FC page's OSD segment, `Gear/Rates/` the Rates segment (curves, compare, the sims' rates, `RateEditor` and the sims' Sync), `Gear/Switches/` the Switches segment and the Controls page, `Gear/Sim/` the Sim page (`SimHub` its sections; Fly: `SimPage`, `Stage`, `scene.ts` the three.js picture, `Osd`, `SimSettings`, hidden unless the `simPreview` setting is on; Radio: `CalibrationPage`, the sim's calibration; Rates: `Gear/Sims/`); `lib/simhost.ts` the host loop (clock sync, interpolation, frame stats) and `lib/simview.ts` its pure helpers, `Gear/Backups/` the Backups segment, `Gear/Models/` the radio's Models and Checklists segments (`useModel` stages ops through `gear_model_edit`; `lib/models.ts` its pure helpers), `Gear/RadioSettings/` the radio's Settings segment (`radio.yml`; `useRadioSettings` stages ops through `gear_radio_settings_edit`; `lib/radioSettings.ts` its pure helpers), `Gear/Voice/` the Voices page (`VoicesPage` the pack library, `ApplyToRadios`, `DeletePack`, `SpeechCheck` a pack's speech check sheet (lines to review first, play, Accept, Re-take selected priced by a dry run), `Studio` the ElevenLabs voice studio, `useStudio` its data, `BrowseVoices` the Voice Library sheet, `Gallery` the Samples gallery (cards by voice, `FlagControl`), `useGallery` its data, `usePlayer` its player (Play all, Stop), `Publish` the publish review with Approve) and the radio's Voice segment (`VoiceSegment`), `Gear/Apply/` the apply sheet, `Gear/Changes/` the Changes segment (`ChangeTable` the change table the Bench shares, history, Edit setting, Copy settings), `Gear/Bench/` the Bench page and the copy dialog (`lib/bench.ts` its pure logic), `Gear/Storage/` the Storage page and `Gear/Flights/` the Pack up, Flights, Packs and Repairs pages; `Shell/StatusBar.tsx`, the Gear status bar), `components/` (`PathText` every displayed file path, with `dash/`, the dashboard pieces every page builds on: `PageHeader` (title, subtitle, actions, `fold` buttons that join the `…` menu when narrow, `more`), `SummaryLine` (a page's counts under the title), `StatRow`/`Stat` summary tiles, `DataTable` (columns, checkbox rows, group rows, numeric columns, stacked rows when narrow), `SectionCard`, `Disclosure` (a closed part of a page), `WithDrawer`/`DetailDrawer` (a row's detail on the right; a sheet when narrow), `Badge`, `EmptyState`, `FactList`, `StatusDot`, `MoreMenu` (the `…` menu, or a pop-up button with `text`); `trim/`, the one trim editor, used by clip detail and the import review, and `gear/`: `DeviceHeader`, `PlugInBar`, `ChecksList`, `DiffView`, `Sticks` (the two-stick widget, Mode 1-4)), `store/`, `actions/`, `ipc/`. Icons and fonts are inlined or bundled so the app works offline |
| `src-tauri/src/` | Rust core. `core/` (`Core`) owns the session and the library index and is the one surface every front end drives: `mod.rs` (state, locking, `Hooks`), `import.rs` (stage, analyse, dates, import, verify, delete clips after import, format), `library.rs` (the index, list, rate, edit, rename, redate), `cuts.rs` (session and library cut lists), `files.rs` (Photos, previews, strips, Trash), `rematch.rs` (radio logs matched again to library clips), `prep.rs` (card prep: format a card with no session) and `setup.rs` (settings, places, profiles) and `gear.rs` (Gear: devices, detection poll, on-connect hooks, cues). `api/` is the one method table: each row names a method, its params and result types and the `Core` call, and `api!` makes `Core::dispatch` (socket, MCP) and one typed Tauri command per method from it; `api/events.rs` holds the typed events; `api/gear.rs` holds Gear's rows, which join the table through `with_gear_rows!`. `lib.rs` holds the GUI's own Tauri commands and `specta_builder`, which tauri-specta exports to `app/src/bindings.ts`; `control.rs` the app's socket, `updater.rs` the app's own updates (tauri-plugin-updater: the launch and hourly check, download, install and restart, as GUI commands only; the restart's new process comes to the front (`QUADCAM_UPDATE_RELAUNCH`, never with `QUADCAM_NO_FOCUS`); `core/update.rs` holds the state for `status.update` and `restart_blocker`, which refuses the restart while a job runs), `mcp/` the MCP server (`server.rs` the protocol, the stdio loop and handlers, `reload.rs` the switch to a newer build on disk (`exec` in the same process, the handoff, the probe), `params.rs` each tool's argument type, `tools.rs` the tool list with schemas derived from those types, `render.rs` the text answers, `gear.rs` the three Gear tools with their argument types and handlers), `bin/quadcam-cli/main.rs` the CLI (clap flags build the `api` params, and it calls the table's `api::call` functions; `bin/quadcam-cli/gear/` holds the `gear` subcommands, one file per area). `gear/` is Gear (`docs/gear-design.md`): `model` (shared types), `store` (the gear folder and `gear.json`, its only writer), `compat` (proven versions), `serial` (USB serial ports, the per-port lock, the check for another process holding the port, the fail-safe), `detect` (what is plugged in), `events` (connected, identified, unmounted but present, removed), `cues` (speech, sound, notification; a voice pack's clip per cue line, `Core::cue_pack` picks the pack), `bf/` (the Betaflight link: `cli` the CLI session and the write engine `run_lines`, `msp` v1/v2 framing and identity, `blackbox` the blackbox flash over MSP (summary, chunked read, erase and poll, log header reader, estimates), `dump` the `dump all`/`diff all` parser with section context, `verify_lines` and the transcript scrubber, `fake` the `FakeFc` simulator, `boards` known issues, USB limits and the makers' build sources per board; `core/fc.rs` holds its `Core` jobs, each opening and releasing the port with one cue), `flights/` (flight analysis from radio logs: armed runs, hover, sag, resting voltage, mAh, dropouts, the `flights.json` cache; `synth` the synthetic log with known values), `packs` (packs, pack types, flight sets, pack history, suggestions), `blackbox` (pulled flash images: one record per pull under `<gear>/blackbox/`, the raw image in the blob store, the by-order pairing of logs with flights; `core/blackbox.rs` holds the pull job, the flash summary (one MSP read, no cue), the USB disk path, the manual erase and the `blackbox` on-connect step, `bin/quadcam-cli/gear/blackbox.rs` its CLI, `app/src/views/Gear/Blackbox/` its FC segment), `crashes` (the crash and repair log), `report` (the session report), `preflight` (the Pack up check; `core/flights.rs` holds their `Core` methods, `mcp/gear_flights.rs` their MCP actions, `bin/quadcam-cli/gear/flights.rs` and `packs.rs` their CLI), `osd` (Betaflight OSD positions: decode, element table, grids, per-profile render, overlap and off-screen check, the editor's `OsdMove`/`OsdCopy` resolve and `apply_edits`; `core/osd.rs` holds `Core::gear_osd` and `Core::gear_osd_edit`), `rates` (rate profiles of a dump, curve samples, the fit of one rate model onto another; the math is `quadcam_sim::rates`), `sims/` (one adapter per sim: Liftoff, Micro Drones, Uncrashed, The Zone, Velocidrone off; byte-span `Doc`, `Slot`/`encode`/`write_profile` for writes; `core/rates.rs` holds `gear_rates`, `gear_sims` and `gear_rates_preview`; `apply/sim.rs` the sim sync plan and `core/sim_sync.rs` its plan, apply and rollback, `core/sim_restore.rs` the restore of a sim's backup), `blobs` (the content-addressed blob store), `changes` (staged changes under `<gear>/changes/`, `render_fc`: FC edits to CLI lines, diff and before state), `apply` (`apply.rs` the report types, `apply/fc.rs` the FC plan, its checks and digest, `apply/card.rs` the card plan, its checks and digest, `roll_back` and `verify`; `core/apply.rs` stage, plan and apply as one FC job; `core/apply_card.rs` the card apply and the mount cycle (`Env.mount` and `Env.unmount`: a card is mounted for a job and unmounted after, the person's Mount lasts 10 minutes); the sheet's click is `gear_apply_click`, an agent's request waits on `Hooks::confirm_apply`), `copy` (copy settings between quads: pure, over two `dump all` texts; `core/bench.rs` holds Keep, Revert, the copy rows and the `apply_ready` on-connect step), `backup` (snapshots: take, read, diff, retain, prune, export, import old backup folders; `core/backup.rs` holds its `Core` jobs and the on-connect `backup_hooks`), `radiologs` (each radio log once per radio), `health` (the card check: `diskutil verifyVolume`/`repairVolume` through `DiskRunner`, its log), `splash` (an image as the EdgeTX splash: 1-bit 128x64, preview, patch and decode a firmware image by its markers), `radios/` (one profile per EdgeTX radio board, `boards.rs` the rows, about 50, all but the proven ones read-only: chip, firmware format and size guard, DFU and bootloader entry with the DFU steps the UI shows, storage and write route, measured USB speed, SD-card pack, YAML schema, splash kind, internal ELRS targets and upload methods, USB product name and ports, last EdgeTX series, look-alike boards, target history, the maker's fork, switch and pot slots, quirks; `check_flash` its guards; the flash, read, restore, DFU link, ELRS flash plan and detection (`Connected.radio`) read it), `elrs/` (ExpressLRS, a preview behind `elrsPreview`, `docs/gear.md`: `crsf` the CRSF codec and parameter messages, `link` the passthrough through a radio's CLI or an FC's CLI and the CRSF conversation, `fake` `FakeElrs`/`FakeHost`, `image` the release bundle, targets and the options block of an image, `flash` the target guard and the `esptool` arguments, `uid` the binding phrase's UID; `core/elrs.rs` holds its `Core` jobs: read, the staged `Edit::ElrsOptions` plan and apply, and the flash plan and apply), `dfu` (USB DFU 1.1 with DfuSe over a `Usb` transport: `NusbUsb` real, `FakeDfu` in tests; erase, write, read back, leave), `firmware/` (`check` newest releases against the saved devices, `vendor` an FC's maker's builds (the Zendesk feed, each file's version line, build identity, the vendor or official channel), `edgetx` the release zip, the board binary and the image checks, `betaflight` the FC flash's board table, Intel HEX, image checks, build service (`Cloud`) and cache, `betaflight_config` the settings carried over a flash, `mod.rs` the `Flasher` trait with its recorder, `FwEnv` and `FixtureFetch`; `core/firmware.rs` holds the check, the splash preview, the flash plan and the flash apply (an FC's go to `core/bf_flash.rs`, a preview behind `bfFlashPreview`: backup, `bl`, DFU flash, then the old settings through the FC apply), each flash behind a digest and `confirm`, and a process started by cargo flashes only a fake unless `QUADCAM_FLASH=real`; `core/fw_restore.rs` lists a radio's firmware copies (`fwcopy`) and restores one through the same DFU write, `dfu_write`, behind a plan, a digest and `confirm`), `edgetx/` (the EdgeTX card engine: `yaml` the line-level editor, `model` typed views and `ModelOp`, `radio` the typed view of `radio.yml` for the Settings editor and the range check every `set_scalar` passes, `editors` the model editors' ops (value screens, logging, RSSI alarms, callouts owned by track name, timer and checklist checks) with their reads and the staged-op merge, `card` plans, the per-file writer and the unmount, `synth` the synthetic card, `cli` the radio's USB serial CLI (`RadioCli`, `FakeRadioCli`), `setup/` a radio setup as data (`docs/radio-design.md` 4.2 and 5: jobs, assignments, the template with `@job` tokens, aircraft overrides; `generate` its generator, `defaults`, `lua` QuadCam's own Lua screens, `postflash` the storage pack, ELRS Lua and volume label planners; `core/radio_setup.rs` holds its `Core` rows); `core/gear_card.rs` holds its `Core` reads and `gear_card_clean`, `core/radio_cli.rs` the serial jobs, `core/dfu_link.rs` the link of a DFU device to a saved radio, `core/model_edit.rs` `gear_model` and `gear_model_edit`, `core/radio_settings.rs` `gear_radio_settings` and `gear_radio_settings_edit`), `voice/` (radio voice: `lines` QuadCam's lines and the spelling rules from `resources/voice/`, `asr` the speech check's comparison (spelling, numbers as words, case and punctuation aside), `tts` the `say` and OpenAI-compatible providers behind `Tts`/`Run`/`Post`, `eleven` the ElevenLabs provider (models, voices, credits, the Voice Library and its add, speech with character timestamps, through `Http`), `keychain` the API key in the macOS Keychain behind `KeyStore`, `rates` the one rate table (USD per 1K characters, promo, request limit) and the recorded character counts, `batch` carrier-sentence batches cut by those timestamps with their cache, checks and estimate, `reuse` the line index (a line a cached take holds is cut from it, never sent again; per-voice counts; the import of earlier single-line takes), `sets` the line sets in `resources/voice/sets/`, `render` the raw cache and normalisation, `packs` build, index and install, `publish` the voices repo (release names, previews, spot check, the license by provider, `gh` through `Gh`, the printed commands), `wav`; `core/voice.rs` holds its `Core` methods and `core/voice_publish.rs` a pack's publish (prepare and review as an `api` row; Approve, `voice_publish_approve`, is a GUI command and the CLI's `--approve` at a terminal, never MCP) and an index pack's previews, `core/voice_studio.rs` the studio's (key, catalogue, the Voice Library and its add, estimate, sample, set render of one or several packs with progress and Stop; never a paid call without `confirm` and a credit check); `core/voice_samples.rs` the sample gallery (samples in `<gear>/voice-samples/` with `samples.json`, moved once from the cache; the packs as cards; flags in `flags.json` beside the takes, marked in the line index as `rejected`; `gear_voice_retake`, the flagged takes again with a seed from the flags, priced and confirmed with its digest); `core/voice_check.rs` the speech check (`gear_voice_check`, and after every render and sample: a line heard wrong in a studio pack leaves it as a re-take, recorded per take in the line index as `heard`; `accept` puts it back; a named re-take line is heard again from its held take); `core/voice_groups.rs` the grouped re-take (`gear_voice_retake` with `grouped`: re-take lines grouped by the word they got wrong, one line per group tried other ways, the winner kept as a pack `Structure` for its lines); `Edit::CardFiles` applies through `core/apply_card.rs`), `switchmap` (the switch map: an EdgeTX model's mixes, logical switches and special functions joined with `aux`/`adjrange` per control and position, conflicts, live matching), `radio_hid` (the radio in USB Joystick mode over hidapi, `FakeHid`, the `radio-input` stream; `Hub`, the one reader for that stream and the sim's unthrottled sinks; `core/switchmap.rs` holds both `Core` reads and the watch), `sim_cal` (the sim's radio calibrations keyed by Gear radio id, the resolve step, the defaults from a switch map; `core/sim.rs` holds its `Core` rows and the calibration session; `core/sim_host.rs` the in-process sim host: the physics thread, the input ring and the page's frame reads, as GUI commands only). The logic modules (`scan`, `disk`, `media`, `logs`, `logmatch` (shape-first log matching: flight lengths, order and gaps, clocks as tie-breaks, EdgeTX models as a filter; an analog clip with dead air matches by picture: flights inside keep ranges, swaps at dead air, short armed dead air as breakup, volt steps per cell, an offset per clip, split DVR files as one timeline), `moments`, `highlights` (macOS video analysis: `highlight` moments for clips without radio-log moments, the key frame, the `app.quadcam.video` item, the cache by content fingerprint; `core/video.rs` holds the session's background job and `library_analyze_video`), `join` (recordings an analog DVR split into files: detection, the `ffconcat` source, the swap of joined and own values), `metadata`, `qtmeta`, `naming`, `pipeline` (with `pipeline/import.rs`, the import run), `session`, `photos`, `library`, `trim`, `cuts` (the one cut writer), `sources` (the `Source` trait per video system, tried in `sources::all()` order; `sources/dji.rs` is DJI (an O4 air unit over USB, a goggles card, older `DJIG####` names): `DCIM/DJI_*/` names, the clock in the name, `.SRT` sidecars, byte-copy MP4 export, no "Format card" after an import, card prep (exFAT) for a removable goggles card only; `sources/analog.rs` is the DVR: clip layout, half-written check and repair, encode plan, dead air, card policy), `trash`, `settings` (with `Defaults`, the effective settings), `paths` (every path under `$HOME`), `geocode` (place search, and `reverse`: a point named by Apple's geocoder), `locate` (this Mac's location from Location Services, one fix), `sun` (sun tags from a clip's place and time, offline), `weather` (Open-Meteo's archive and historical forecast through `Fetch`; `core/weather.rs` holds `library_weather`, which writes `app.quadcam.weather`)) run without Tauri |
| `src-tauri/sim/` | `quadcam-sim`, the sim core (`docs/sim-design.md`), a workspace member with no Tauri dependency: `input.rs` (samples, the input ring, calibration, auto-calibration, the link model); the physics and controller (`sim.rs` the step, `world.rs` Rapier, `motor`, `aero`, `battery`, `mixer`, `fc`, `rates`, `diff` its own `diff all` reader, `profile` with the presets in `presets/`, `ring` the frame each step reads, `runner` the real-time thread, `snapshot`, `record`, `bench`); `log.rs` and `validate.rs` the validation harness on decoded blackbox logs, with fixtures in `tests/fixtures/`; `LICENSES/` the notices of ported MIT code. The dev profile builds it and its physics crates optimised. Test it with `cargo test --workspace`; `cargo run -p quadcam-sim --example bench` is the 10 min real-time bench |
| `intel/` | `quadcam-intel`, the Swift helper (SwiftPM, `docs/development.md` "The Swift helper"): `caps`, `asr` (SpeechAnalyzer + SpeechTranscriber, macOS 26+), `install` (AssetInventory), `video` (`Video.swift`: MediaIntelligence VideoAnalyzer highlights, levels and key frame, macOS 27+), `suggest` (`Names.swift`: Foundation Models clip names, keywords, notes and a day's summary; text on macOS 26, a frame too on 27; FoundationModels weak-linked); JSON lines on stdout, exit 3 for unavailable. Built for arm64 with the newest SDK, deployment target macOS 13, each feature behind `#available`. `src-tauri/build.rs` builds it into `src-tauri/binaries/` (git-ignored); `tauri.conf.json` bundles it as the external binary `binaries/quadcam-intel` (`Contents/MacOS/`, beside `quadcam-cli`; `target/<profile>/` in a cargo build). Without Xcode 26 a stand-in answers unavailable; `QUADCAM_INTEL_REQUIRED=1` (CI, release) fails the build instead. `src-tauri/src/intel.rs` runs it through `modules::run::Runner` with a timeout (`intel/names.rs` its `suggest`, the request in a temp file); `gear::Env.intel` holds it. `core/ondevice.rs` is on-device suggestions: outings (day, place, aircraft) share the model's name with pack numbers, a field is filled only while empty or still the model's last value (`ClipPlan.ondevice`), the setting `on_device_suggestions`, and the session report's `summarize` |
| `src-tauri/src/schema.rs` | The MCP and CLI schema version (`SCHEMA_VERSION`), the deprecation table (`DEPRECATIONS`) and the surface guard's logic. `tests/it/surface.rs` runs the guard against `tests/surface/mcp.txt` and `cli.txt` |
| `src-tauri/src/modules/` | The module manager (`docs/modules.md`): `manifest.rs` (the pins in `src-tauri/resources/modules.toml`, compiled in; newer pins from a release's `modules.json`), `fetch.rs` (`Fetch`, `/usr/bin/curl`; a cargo process reaches only 127.0.0.1 unless `QUADCAM_FETCH=real`), `install.rs` (checksum, unpack, quarantine, ad-hoc signature, `installed.json`), `run.rs` (hash check before a run, the child-process `Runner`). `core/modules.rs` holds the `Core` methods |
| `scripts/notices.mjs` | Third-party notices (crates, npm packages, fonts, icons) and the license allow list; `pnpm build` writes `app/dist/THIRD_PARTY_NOTICES.txt`, which `tauri.conf.json` copies to `Contents/Resources`. `scripts/licenses/` holds SPDX standard texts |
| `src-tauri/Info.plist` | Photos, speech recognition and location usage strings, merged into the bundle's Info.plist |
| `src-tauri/Entitlements.plist` | Hardened-runtime entitlements (Photos library, location) for every signature |
| `src-tauri/tests/` | Integration tests on synthetic clips and FAT32 disk images. `it/` is one test binary, one module per area; the four files beside it change an environment variable and stay binaries of their own |
| `test-clips/` | Local test corpus. Git tracks only its README |
| `scripts/make-corpus.sh` | Builds `test-clips/synthetic/` |
| `scripts/dev-seed.py` | Fills a temporary `HOME` with a made-up library and gear for a dev build (`docs/development.md`, "Seed a dev HOME with made-up data") |
| `assets/icon.svg` | Icon source. Regenerate with `cargo tauri icon` |
| `docs/` | The user guide (linked from `README.md`), `docs/images/` (screenshots from `app/e2e/shots.spec.ts`), and `architecture-plan.md` |

## Requirements

- Rust via rustup, at the version `rust-toolchain.toml` (repo root) pins; rustup installs it on the
  first `cargo` run. The Tauri CLI is a dev dependency of `app/` (`@tauri-apps/cli`): run it as
  `app/node_modules/.bin/tauri` after `pnpm install`, or install it globally with
  `cargo install tauri-cli --version "^2" --locked` to type `cargo tauri`. Releases use the prebuilt one.
- specta `=2.0.0-rc.25`, tauri-specta `=2.0.0-rc.25` and specta-typescript `=0.0.12` are release
  candidates: they stay pinned exactly, and an upgrade regenerates and reviews `app/src/bindings.ts`.
- ffmpeg and ffprobe from Homebrew (`brew install ffmpeg`), or QuadCam's ffmpeg module (users get the module through the first-run banner; the cask does not install ffmpeg). `tests/it/external_tools.rs` lists every program the code may run. The app
  looks in a path the `modules` setting names, then the module (unless `ffmpegSource` is
  `homebrew`), then `/opt/homebrew/bin` and `/usr/local/bin`, then `PATH` (`media::find_tools`). exiftool is optional; tests use it
  when present.
- Node 24 through fnm (`.node-version`) and pnpm (`packageManager` in `app/package.json`), for
  `app/`; `cargo tauri dev` and `cargo tauri build` run it.
- The window loads `app/` (built to `app/dist`). The Vite dev server is pinned to port 4719
  with `strictPort`; `cargo tauri dev` starts it and always loads 4719. `QUADCAM_DEV_PORT`
  moves the server for `pnpm dev` and `pnpm e2e`: Vite and Playwright both read it. A second
  worktree sets its own value (for example `QUADCAM_DEV_PORT=5871 pnpm e2e`), so its
  Playwright run never reuses the first worktree's server.

## Build, test, run

Run these in `src-tauri/`:

```bash
cargo nextest run -P quick      # the fast loop: all but the disk-image and long ffmpeg tests
cargo nextest run               # every test (needs ffmpeg; attaches small disk images one at a time)
cargo nextest run --test it join::   # one area (tests/it/join.rs); -E 'test(name)' picks tests by name
cargo test                      # still works; nextest is faster (each test its own process)
cargo test --test it import::size_and_speed -- --ignored --nocapture   # MP4 vs MOV size/speed
INSTA_UPDATE=always cargo test --test it snapshots::   # accept a deliberate shape change, then review the diff
QUADCAM_UPDATE_SURFACE=1 cargo test --test it surface::   # record new tools, actions, flags in tests/surface/ (additions only)
QUADCAM_UPDATE_BINDINGS=1 cargo test --test it bindings::   # write app/src/bindings.ts after an api change
HOME=$(mktemp -d) cargo tauri dev   # run from source (Photos is dry-run; QUADCAM_PHOTOS=real to test it; the real HOME needs QUADCAM_PATHS=real)
cargo tauri build               # -> target/release/bundle/macos/QuadCam.app (runs pnpm --dir app build)
cargo build --release --bin quadcam-cli   # -> target/release/quadcam-cli
open target/release/bundle/macos/QuadCam.app
```

- **Test loop:** run `cargo nextest run -P quick` plus the tests of the area you touched
  (`--test it <module>::`). The full suite is CI's job: it runs on every push to main or a tag, and on a
  branch push whose change touches the slow tests' code (see the path filter below).
  Run it locally only before a release, or when CI is down. On a busy machine (other
  worktrees building) cap it: `--test-threads 4`, and `CARGO_BUILD_JOBS=4`.
- **nextest** (`cargo install cargo-nextest --locked`, or `brew install cargo-nextest`):
  `src-tauri/.config/nextest.toml` holds the profiles. `default` runs everything; `quick`
  skips the disk-image tests and `tests/it/join.rs`; `ci` adds JUnit and one retry for the two
  known load flakes (`tests/it/join.rs`, `tests/it/mount_cycle.rs`); `ci-quick` is `quick`'s filter
  with `ci`'s settings, for a branch whose change misses the slow tests' code. The `serial` group runs the
  disk-image tests one at a time (hdiutil and diskutil share one daemon, and a detached
  image's disk number is reused). The integration tests are one binary per crate, `it`
  (`tests/it/main.rs`, a module per area), so a test's name starts with its module
  (`join::...`) and the crate links once, not once per file. `tests/output_dir.rs`, `rates.rs`,
  `real_paths.rs` and `voice_core.rs` change an environment variable that other areas read, so each stays a
  binary of its own (`Cargo.toml` lists every `[[test]]`; `autotests = false`), as do the
  sim's `tests/bench.rs` and `tests/map_callout.rs` (AppKit on the main thread: no test harness,
  it answers nextest's `--list` itself). `tests/it/common` `Image` also takes a machine-wide lock, so
  disk-image tests in two worktrees take turns. A new test that attaches an `Image` goes into
  the `serial` filter and the `quick` exclusion. Three tests that time the sim's real-time
  thread against the wall clock (`sim/tests/it/runner.rs` stall, `sim/tests/bench.rs`,
  `tests/it/sim_host.rs` frames advance) run alone (`threads-required`); a new one joins them. Doc tests are not nextest's: the crates have
  none; if one is added, CI needs `cargo test --doc`.
- **Rust version:** `rust-toolchain.toml` pins one Rust release (channel, clippy, rustfmt) for
  every folder and for CI, so a new stable lint never fails CI first. To bump it: set the new
  `channel`, run `cargo clippy --workspace --all-targets -- -D warnings` and `cargo nextest run`,
  and fix every new lint or warning in that same commit (`rust: 1.NN.0`). `rust-version` in
  `Cargo.toml` is the oldest supported release and moves only when the code needs a newer one.
- **Build cache:** sccache (`brew install sccache`, then `RUSTC_WRAPPER=sccache`) shares
  compiled dependencies across worktrees, which each have their own `target/`. sccache cannot
  cache incremental builds, so set `CARGO_INCREMENTAL=0` with it. `CARGO_INCREMENTAL=0` also
  suits many worktrees without sccache: incremental data is large on disk and helps only the
  crate you edit.

In `app/` (`pnpm install` first):

```bash
pnpm typecheck && pnpm lint && pnpm test   # tsc, eslint, Vitest
pnpm e2e                                   # Playwright, headless, on the mocked core
QUADCAM_DEV_PORT=5871 pnpm e2e             # same, on another dev-server port (a second worktree)
pnpm build                                 # -> app/dist
scripts/make-fixtures.sh                   # re-record the mock core's data (needs the CLI and the corpus)
```

- **Core calls in `app/`** go through `app/src/ipc/api.ts`, on the generated
  `app/src/bindings.ts` (never edit it). `ipc/types.ts` narrows the generated types for the UI
  (specta types every f64 as `number | null`; fields serde may omit are optional there), and
  `ipc/normalize.ts` makes each answer fit: a range or point without a time is dropped, a
  length reads as 0. The UI calls one typed command per core method; the specs read those
  calls by method (`app.method(...)` in `app/e2e/fixtures.ts`).
- **Theme** (`app/src/theme/`): `palette.css` is generated from `@catppuccin/palette`
  (`node scripts/gen-palette.mjs`; a test fails when it is stale). `tokens.css` maps it to
  semantic names (Mocha for dark, Latte for light) plus the 8-pt spacing, radii and type
  scale. Components use CSS Modules and tokens only, never a hex value. `e2e/theme.spec.ts`
  runs axe, contrast included, on the component gallery (`/?gallery` in dev) in both themes.
  Gear pages follow one dashboard pattern (`components/dash/`: `PageHeader`, `SummaryLine`,
  `StatRow`, `SectionCard`, `DataTable`, `Disclosure`, `DetailDrawer`, `FactList`, `StatusDot`,
  `Badge`, `EmptyState`, `MoreMenu`); `docs/development.md` "How to build a dashboard page"
  describes it. The app is responsive: content fills the window less the sidebar up to
  `--content-max` (1760 px, `tokens.css`) and centres; no page sets its own max-width. `main`
  is the size container `content`, so page rules use `@container content`; at 720 px of window
  the sidebar becomes an overlay. `e2e/layout.spec.ts` checks every page at 420, 900, 1400
  and 2400 px.
  The Map page (`views/Map/`, behind the `mapView` setting) lays a native `MKMapView` over the
  web view at the placeholder's rectangle (`src-tauri/src/mapview.rs`, `docs/development.md`
  "The native map"); the page's pins come from `lib/map.ts`.
  On macOS 26 and later `src-tauri/src/glass.rs` gives the window an overlay title bar and a
  glass sidebar, and `html[data-glass]` lays the page out for it (`docs/development.md`
  "Window chrome and theme").
  Every axe spec loops over `axeSchemes` (`e2e/fixtures.ts`); `QC_AXE_THEMES=dark` limits them
  to one theme, as CI does on a branch (main, `rel-*` and tags run both).
- **Dev in a browser:** `pnpm dev`, then `http://localhost:4719/?mock=<scenario>` (your `QUADCAM_DEV_PORT` instead of 4719) runs the
  UI on the mock core (scenarios in `app/src/ipc/mock/core.ts`; `gear` has devices plugged
  in). The mock's Gear (`ipc/mock/gear.ts`) is written by hand, not recorded: a recorded
  `gear status` would hold the recording machine's devices. Specs plug devices in with
  `core.plug(connected, unmounted)`.
- **Parity specs** (`app/e2e/parity/`) hold the behaviour the legacy UI had, and run in
  headless WebKit against `app/src/ipc/mock/`, a fake core behind a fake Tauri runtime. Specs
  find controls by role and label, never by class. The mock's data is the real core's JSON,
  recorded by `app/scripts/make-fixtures.sh`; regenerate it when a shape changes.

## Release

GitHub (`noahkiss/quadcam`, public) is the only remote. Users install the cask
`noahkiss/tap/quadcam`.

1. On the release branch (`rel-X.Y.Z`, every change merged in), set the new version in
   `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (they must match), commit
   `release: X.Y.Z`, and push the branch. CI runs on that commit; a `rel-*` branch runs
   everything, as main does (every Rust test, axe in both themes).
2. Wait for that run to go green (`gh run watch <id> -R noahkiss/quadcam --interval 60 --exit-status`).
3. Fast-forward main to that same commit and tag it. No merge commit, no rebase: the tag
   names the commit that went green. Check that the main checkout is clean first, and check
   the tag against the release branch before you push it. A failed fast-forward once let a
   script tag the old commit.

   ```bash
   set -e
   test -z "$(git status --porcelain)"
   git switch main && git merge --ff-only rel-X.Y.Z
   git tag vX.Y.Z
   test "$(git rev-parse 'vX.Y.Z^{commit}')" = "$(git rev-parse rel-X.Y.Z)"
   git push origin main vX.Y.Z
   ```

4. `.github/workflows/release.yml` (through the shared composite actions) builds, signs
   and notarizes `QuadCam.app` on an `xcode-27` (arm64) runner with the
   prebuilt Tauri CLI from `app/`. It does not run the tests: before it attaches the zip
   (`quadcam-X.Y.Z-arm64.zip`) to the GitHub Release, it waits up to 40 minutes for a green
   `ci.yml` run on the tagged commit, and fails without one. It also attaches `modules.json`
   (`quadcam-cli modules manifest`, the pins Settings > Modules checks), and the in-app updater's
   files: `quadcam-X.Y.Z-arm64.app.tar.gz` (the notarized, stapled app), its minisign `.sig`, and
   `latest.json`, which the app reads from `releases/latest/download/`. It then fires the tap's
   `bump.yml`, which rewrites `Casks/quadcam.rb`, installs it on macOS, and commits.
5. Watch it: `gh run watch <id> -R noahkiss/quadcam --interval 60 --exit-status`. Re-run for an
   existing tag with `gh workflow run release.yml -R noahkiss/quadcam --ref vX.Y.Z -f tag=vX.Y.Z`.
6. Once the release is out, prune. Delete every branch merged into it, local and remote:
   `rel-X.Y.Z` and each feature branch it merged (`git branch -d <branch>`,
   `git push origin --delete <branch>`). Remove their worktrees (`git worktree remove`). Keep
   only main and branches with unmerged work. GitHub deletes a pull request's head branch on
   merge ("Automatically delete head branches"); the rest is this step.

- **Runner image:** `ci.yml`'s `check` job, `release.yml` and `release-cache.yml` run on GitHub's
  `xcode-27` image (macOS 27, arm64, Xcode 27), which is a public preview: it may change or be
  retired without notice, and its queue can run long (47 min for the 0.20.0 release). The UI
  jobs (`ui`, `e2e`) run on `ubuntu-latest`.
  The Swift helper (`intel/`) builds against its SDK: its speech check needs the macOS 26 SDK,
  and the macOS 27 APIs it is meant to add need Xcode 27. Move both files to a GA label once
  one ships Xcode 27. A red run that fails before any
  step of ours (image missing, queue never starts) is the preview, not the change.
- **The CI wait:** `release-attach` (`noahkiss/workflows`) accepts any green `ci.yml` run on
  the tagged commit, so the release branch's green run counts. It fails only when every run on
  the commit ended without success. The runs that the push of main and the tag start find
  that green `rel-*` run (`ci.yml` job `proven`) and skip the tests, so they leave the macOS
  runners to the release. A push of main with no green `rel-*` run on its commit (a merge, a
  direct push) runs everything.
- **Renovate:** a push to a `renovate/*` branch whose change stays inside `app/` (and
  `.node-version`) skips the macOS `check` job (`ci.yml` job `renovate`) and runs only the UI
  jobs. A change anywhere else, such as `Cargo.lock` or a workflow, runs everything. The
  push of main after the merge runs everything too.
- **Release cache:** a cache saved under a tag is invisible to the next tag, so
  `release-cache.yml` builds the app unsigned on main after each Release run and saves the
  Rust dependency cache under the key `quadcam-release`; `release.yml` restores it
  (`cache-shared-key`, `cache-save: false` on `tauri-macos-build`). rust-cache removes the
  workspace's own crates before it saves, so the release still builds the app from the tag.
  Keep the two jobs' toolchain and `CARGO*`/`RUST*` environment the same: rust-cache hashes
  them into the key. After a change to `rust-toolchain.toml`, seed it with
  `gh workflow run release-cache.yml -R noahkiss/quadcam --ref main`.

- **Signing:** `release.yml`'s own job names the `release` environment and runs the composite
  actions `tauri-macos-build`, `macos-sign-notarize` and `release-attach` from `noahkiss/workflows`.
  A reusable workflow from another repo cannot read this repo's environment secrets. It signs with `Developer ID Application: NKMK Digital Co. (2Z88BYP37C)` (hardened runtime,
  timestamp, `src-tauri/Entitlements.plist`), notarizes and staples. Local builds stay ad-hoc
  (`signingIdentity "-"`) with the same hardened runtime and entitlements. The entitlements are
  `com.apple.security.personal-information.photos-library`, which PhotoKit needs under hardened
  runtime, and `com.apple.security.personal-information.location`, which Location Services
  needs; add one only for a new protected resource. The secrets live in the `release`
  environment (`v*` tags only), so a re-run dispatches on the tag, with that tag's `release.yml`:
  `gh workflow run release.yml -R noahkiss/quadcam --ref vX.Y.Z -f tag=vX.Y.Z`.
- **Secret:** `HOMEBREW_TAP_TOKEN` (the tap PAT) lets the release dispatch the tap.
- **Updater key:** `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (empty) in the
  `release` environment sign the updater archive; its public key is `plugins.updater.pubkey` in
  `tauri.conf.json`. `release-attach` tars the app after notarization and signs that tar with
  `tauri signer sign --app-version` (the config sets `requireSignedVersion`). The config leaves
  `createUpdaterArtifacts` off: Tauri's own archive would hold the app before notarization, and a
  local `cargo tauri build` would need the key. A lost key means no in-app update reaches installed
  copies: they must update through Homebrew once.
- `.github/workflows/ci.yml` runs fmt, clippy, tests (`cargo nextest run --profile ci`) and a
  release build on every push to a branch or `v*` tag and on pull requests from forks, a
  `ui` job for `app/` (typecheck, lint, Vitest, build), and an `e2e` job that splits the
  Playwright specs across four Linux runners (`--shard`). On a branch or pull request,
  a path filter (`dorny/paths-filter`, against the merge base with main) picks `ci` only when
  the change touches what the slow tests cover (import, disk, card, prep, join, media, scan,
  session, sources, the CLI's `main.rs`, the MCP server's non-Gear files, `tests/it/common`, those
  test files, `Cargo.toml`/`Cargo.lock`, the toolchain file, `nextest.toml`, `ci.yml`), else
  `ci-quick`; main, `rel-*` and tags always run `ci`. A new slow test, or new code under one, joins that
  filter in `ci.yml`. A newer push cancels an
  older run on the same branch or PR. A run on `main` or a tag is never cancelled: the release
  waits for it.
- **Watching runs:** use `gh run watch <id> --interval 60`. Several watchers at the 3 s default
  hit GitHub's API rate limit (HTTP 403); after one, wait 2 minutes and retry.

## Behaviour notes

- **Library model:** the output folder is the library. `library::Layout` files clips as
  `YYYY/YYYY-MM-DD/` (default), `YYYY-MM-DD/` or flat, optionally with the place name on the
  day folder; originals go in `originals/` inside the day folder. The files are the source of
  truth: every detail (rating, flag, moments, keep ranges, flight stats, place, source
  fingerprint, Photos state) is an `app.quadcam.*` QuickTime item in the file.
  `<library>/.quadcam/index.json` is a rebuildable cache (`library::rebuild`); only unsaved cut
  ranges live in it alone. A rebuild never moves or writes a file, which is also how an
  existing export folder is adopted.
- **Identity:** a library clip's id is its source file's content fingerprint
  (`app.quadcam.source`), never a file name. A file QuadCam did not write is known by a hash of
  its first MB before `moov` (`identity::head_id`), which metadata rewrites never touch. Both
  are specified hashes (XXH64 over defined bytes; `identity.rs` states them), with test
  vectors. Ids from 0.4 and earlier (`DefaultHasher`) stay recognizable through
  `identity::legacy`, an explicit SipHash-1-3. An index without `id_scheme: 2` is moved when it
  loads: a clip whose kept original proves its 0.4 id, and every adopted file, get the current
  id, and the 0.4 id stays in `aliases`, which every lookup accepts. Other clips keep their 0.4
  id. No media file is written. `tests/fixtures/legacy-library` is a library 0.4.1 built.
- **Cuts:** `trim.rs` is the one cut model for the session and the library. Dropping a cut that
  was already exported needs a decision (`RemovedCuts::Keep`: the file stays as its own clip,
  marked `app.quadcam.detached`; `Trash`: it goes to the Trash). Without one, `Core` answers
  `CutChange::Confirm` (GUI) or refuses the patch (CLI `--removed`, MCP `removed_cuts`).
  `cuts::write_cut` writes every cut file, session and library alike: it verifies frames and
  reads the QuickTime items back before the rename.
- **Flight, not pack:** a radio-log armed segment is a "flight" in UI text, docs, API, MCP and
  CLI names; "pack" means a battery (Gear) only. `FlightStats` reads the 0.6.4 keys `packs`
  and `pack_spans` through serde aliases.
- **Split by flight:** `FlightStats.flight_spans` holds each radio-log flight's armed range in
  clip seconds (moved by the log offset). `trim::flight_cuts` makes one cut per flight, 2 s
  each side at most half the gap, clamped to the clip, and refuses when there is nothing to
  split; the cuts are added to the clip's list (`PlanPatch.split_by_flight`, `session_split`,
  `library_split`), so no exported cut is dropped.
- **Joined recordings:** at analysis `join::groups` finds analog files the DVR split (next
  number, same folder and format, the first full length by `join::full_length`: a
  `KNOWN_SPLITS` length (Echo 600 s) +/- 1 s, the same length as the file numbered next to it
  (a whole minute, >= 60 s), or a size just under 4 GiB; file times that agree); unsure
  stays apart. Each file stays a `Clip`; the first carries `Join` (its `source()` is an
  `ffconcat` list in staging, read by ffmpeg as one input) and the later ones `part_of`.
  Parts are left out of jobs and dating; `can_format` and `delete_clip_file` judge a part by
  its first file's result and the joined probe. The output names the later files in
  `app.quadcam.parts` (`LibClip.parts`, counted by `known_sources`); kept originals add
  `<stem>.part2.avi`. Setting `join_split_recordings` (on), per run `SourceParams.join`, per
  clip `PlanPatch.joined`.
- **Previews:** after `Core::analyse`, the GUI's `Hooks::analysed` runs `Core::make_previews` on a
  thread. Session and library previews both go through `Core::proxy_once`, which makes one
  proxy at a time (a static lock), so a Play click on a clip being made waits for it and reuses
  the file. Library MP4s play directly. Session and library Photos adds both go through
  `Core::share_files`, which also marks the library's clips as in Photos.
- **Native feel (UI):** page text is not selectable and the cursor is the arrow; WebKit needs
  `-webkit-user-select`. Text a person may copy (paths, details values) gets the
  `selectable` class. The web view's context menu shows only over text fields and selected
  `selectable` text; the app's own menus call `preventDefault` first.
- **Removable cards:** `disk::is_removable` takes an external removable or ejectable disk, or
  a card in the built-in SD slot (`DiskInfo::is_slot_card`: removable media on the `Secure
  Digital` bus; macOS reports the slot as `Internal`). Internal storage (`is_internal_storage`)
  never counts. "Safe to remove" (`disk::safe_remove`, method `eject`, after a format or card
  prep) runs `diskutil unmountDisk` for a card, so the disk stays listed until it is pulled,
  and `diskutil eject` for anything else. Disk images report removable media, so the disk-image
  tests see an unmount: the image stays attached and `Image::is_mounted` is false.
- **Mount cycle:** every operation on a card mounts it if needed, works, and unmounts it when
  done; "safe to unplug" plays only after the unmount worked. One helper does it:
  `core/apply_card.rs` (`locate_volume`, `unmounted_cards`, `mount_picked`, `card_for_job`) for
  gear jobs, `core/session_card.rs` for the import session's card (`stage_device`, `import`
  unmounts it after "Delete clips after import", `format_plan`/`format`/`eject`/card prep mount
  it again). Do not add a second mount path. A DJI device over USB (an O4 air unit) follows the
  same cycle: it is unmounted (never ejected, never formatted).
  Lists and checks that only read (`gear_connected`, Pack up) never mount. The Mount button
  (10 minutes, `MOUNT_MINUTES`) stays for browsing. Tests: `tests/it/gear_apply_card.rs` (fake
  `diskutil`) and `tests/it/mount_cycle.rs` (disk images).
- **Cards** show in the sidebar with their source (Analog, DJI) and an "N new" count (content
  fingerprints not in the index); inserting a card never starts an import on its own.

- **Output folder:** defaults to `~/Movies/quadcam`, resolved from `$HOME` at runtime
  (`paths::default_output_dir`). The app creates that default, with its parents, on the
  first import. It never creates a folder the user picked. Picking a folder saves it as the
  setting.
- **Settings:** one file, `settings.json` in the support folder (camelCase keys `outputDir`,
  `format`, `formatLabel`, `photosAlbum`, `places`, `profiles`, `defaultProfile`, `geocoder`,
  and others). `settings.rs` is its only reader and writer: a write reads the file fresh,
  changes only the given keys, keeps unknown keys, and renames a temp file into place under
  an flock on `settings.json.lock`. No surface keeps a full copy and writes it back. The GUI
  (no Tauri store plugin) reads and writes through `Core` (`settings`, `settings_set`,
  `place_*`, `profile_*`); its Settings window saves only keys changed while it was open. The
  app watches the file and re-reads it (event `settings-changed`) when the CLI writes it;
  an agent's MCP writes go through the control socket when the app runs. `KEYS` names every
  setting (file key, CLI/MCP name, check). Add a new setting there with a default in
  `Defaults`; never drop a key a 0.3.0 file has (`settings::tests::settings_from_0_3_0_carry_over`).
- **Place search:** `geocode.rs`. `apple` (default): MapKit `MKLocalSearch` through
  objc2-map-kit. Its answer arrives on the main queue, so on the main thread (CLI, MCP) it
  spins the run loop; in the app it waits on a worker thread. `nominatim`: `/usr/bin/curl`
  with a quadcam User-Agent and at most one request a second across processes. `census`:
  US Census one-line address geocoder (keyless, US street addresses), also the automatic
  fallback when `apple` or `nominatim` finds nothing. `google`: Places API (New) text search,
  off unless chosen; the key comes from `QUADCAM_GOOGLE_PLACES_KEY`, else the
  `googlePlacesKey` setting, and goes to curl on stdin (never argv). Settings reads redact it
  (`settings::SECRET_KEYS`); `Defaults` never serializes it. Never put a key in code or
  tests. Search only on an explicit request, never per keystroke.
- **File-name date:** `naming::DateFormat` (`nameDateFormat`: `YYYY-MM-DD` default, or
  `YY.MM.DD`) starts new file names, renames and redates. `naming::split_date` reads either
  format; a clip's date comes from its creation date first, then the name.
  `library_apply_name_format` renames existing clips (with cuts, originals and the originals' sidecars) in place.
  Folder names keep `YYYY-MM-DD`.
- **Time of day:** a clip's plan time comes from its radio log, its clip clock (DJI), or by hand (`PlanPatch.time`,
  `HH:MM`, empty for none). Without one the creation date is local noon. A library date or
  time edit (`library_edit`) rewrites the QuickTime creation date, the `mvhd` time and the
  mtimes of the clip, its cuts, its original and the original's sidecars (a DJI `.srt`,
  found on disk by stem, `LibClip::sidecars`); a new day moves them all (`relocate`, which
  rename uses too). A library profile edit rewrites make, model, aircraft, video system,
  profile name and the profile's keywords.
- **Session restore:** the GUI's core uses the same session file as the CLI. At launch,
  `Core::forget_unrestorable` drops it unless it was analysed and every staged clip still
  exists. "Start over" (`Core::clear`, method `clear`, `quadcam-cli clear`) deletes the file;
  the GUI also clears it after it erases a card.
- **Delete clips after import:** the setting `deleteClipsAfterImport` (off by default) is the
  only consent to delete clip files from a card or folder. `Core::import` runs it after the
  export; a run can turn it off (`ImportOptions.keep_clips`, CLI `--keep-clips`, MCP
  `keep_clips`), never on. It never formats. Its guards all run in
  `core::import::delete_clip_file`, immediately before the unlink: the source's
  `CardPolicy.delete_clips_offered`, a verified result, the output verified again, the path a
  plain file inside `Session.source` that the source's `pick` takes as a clip, and the same size
  and fingerprint as the staged copy. Sidecars and every other file stay. The result lists each
  clip (`ImportOutcome.clip_deletion`).
- **Add to Photos:** one "Add N files" button on the import's Finish step, and per clip in the library. `photos::PhotoKit` uses
  PhotoKit through `objc2-photos`: add-only access for the library, read-write when an
  album is set (default album `Drone`, created if missing; an empty setting means library
  only). Denied access returns a message that points to System Settings > Privacy &
  Security > Photos. Only outputs this session verified can be shared.

- **ExpressLRS (preview):** every job refuses unless the `elrsPreview` setting is on. `gear elrs read`
  puts a radio or an FC in passthrough until it is restarted or unplugged (a second job needs that
  restart). The binding phrase is the write-only setting `elrsBindingPhrase` (in `SECRET_KEYS`);
  it is never an option, never in a plan, report, log or test, and plans show only
  `uid::fingerprint`. A flash is the usual plan, digest and `confirm`; it keeps the chip's flash as
  a backup first, runs only the `esptool` module, and counts as verified only when esptool says it
  verified the data. Tests use `gear::elrs::fake` and a recording `Runner`; ExpressLRS and esptool
  never download or run for real. Nothing has touched a real device: say so in anything a user
  reads.

## Agent/CLI usage

`quadcam-cli` does everything the GUI does, on the same core. Every command takes `--json`
and prints one object: `{"ok":true,"result":...}` or
`{"ok":false,"error":{"code","exit","message"}}`. Exit codes: 0 ok, 1 failed, 2 usage,
3 refused by a safety guard, 4 no session, no card or no device. Runs share a session file
(`~/Library/Caches/app.quadcam/session.json`; override with `--session FILE`).

```bash
quadcam-cli --json cards                       # cards and radio log sources
quadcam-cli --json scan /Volumes/CARD          # list clips, copy nothing
quadcam-cli --json stage /Volumes/CARD         # copy to staging; new session
quadcam-cli --json analyze                     # probe, recover, thumbnails
quadcam-cli --json dates --logs /path/to/LOGS --day 2026-10-04 --set 2=2026-10-03
quadcam-cli --json import --name 0=backyard-loops --skip 3 --format mp4 --add-to-photos
quadcam-cli --json import --plan plan.json     # {"clips":[{"id":0,"name":"..","date":"..","time":"HH:MM","note":"..","skip":false}],"format":"mov","output_dir":".."}
quadcam-cli --json import --time 0=18:30       # manual time of day (default noon)
quadcam-cli --json import --keep-clips         # this run keeps the clips (delete_clips_after_import)
quadcam-cli --json stage /Volumes/CARD --no-join   # split DVR recordings stay separate this run
quadcam-cli --json import --separate 0         # clip 0's files (a split recording) import one by one
quadcam-cli --json cut 0 --by-flight           # one cut per radio-log flight
quadcam-cli --json verify                      # re-check the session's outputs
quadcam-cli --json clear                       # forget the session, delete the session file
quadcam-cli --json library list --group picks  # also rate, rebuild, rename, edit, cut, trash, photos
quadcam-cli --json library edit <id> --date 2026-09-28 --time 18:30 --profile Whoop
quadcam-cli --json places search "Golden Gate Park" [--provider nominatim]
quadcam-cli --json places save NAME --location LAT,LON | --search QUERY [--pick N]
quadcam-cli --json profiles save NAME --aircraft .. --camera-make .. --models A,B --default
quadcam-cli --json settings set layout=day place_folders=true   # `settings` shows them
quadcam-cli --json modules [check | install NAME --yes | remove NAME | manifest]
quadcam-cli --json library apply-name-format   # after settings set name_date_format=YY.MM.DD
quadcam-cli --json photos out.mp4 --album Drone
quadcam-cli --json eject                       # safe to remove: unmountDisk a card
quadcam-cli --json gear status                 # gear folder, Gear settings, what is plugged in
quadcam-cli --json gear devices save <id> --name N --aircraft PROFILE
quadcam-cli --json gear fc identify|read|check|notes|usb   # a Betaflight FC (read reboots it)
quadcam-cli gear osd quad.dump_all.txt [apply.cli] [--grid PAL] --text   # OSD per profile, checked
quadcam-cli --json gear osd-edit <fc> --move vbat=12,3 --copy 1:2         # stage OSD edits (one "OSD layout" change)
quadcam-cli --json format --plan               # runs every guard, prints device + volume UUID
quadcam-cli --json format --device /dev/diskN --volume-uuid <uuid> --yes
quadcam-cli --json format --prep --plan --mount /Volumes/CARD   # card prep: no session needed
quadcam-cli --json format --prep --device /dev/diskN --volume-uuid <uuid> --yes
```

`format` refuses (exit 3) unless every GUI guard passes and `--device` (the card's whole
disk), `--volume-uuid` and `--yes` all match the staged card. `format --prep` is card prep
(`docs/gear-design.md` 7.9): a card with no session, refused while any clip on it (by content
fingerprint) is not in the library. It moves to `gear card-prep` with the app's card-prep
button (WP13).

### MCP server

`quadcam-cli mcp` is an MCP server on stdio, a thin layer over the same core. `cargo tauri
build` also puts the CLI inside the bundle (`QuadCam.app/Contents/MacOS/quadcam-cli`), and the
cask links it into Homebrew's `bin`. Register it:

```bash
claude mcp add quadcam -- "$(brew --prefix)/bin/quadcam-cli" mcp
```

- When the app is running, the server drives the app's session through the control socket
  (`~/Library/Application Support/app.quadcam/control.sock`: JSON-RPC 2.0, one object per
  line, folder 0700, socket 0600). The person sees every change live. When the app is not
  running, it falls back to a headless core on the shared session file. Each call opens a
  fresh connection (with a ping), so an app restart never leaves a dead pipe. A call that
  started against the app is never retried headless.
- Tools: the import flow `quadcam_status`, `quadcam_load_clips`, `quadcam_read_clips`
  (thumbnails as image content), `quadcam_match_logs`, `quadcam_suggest`, `quadcam_export`,
  `quadcam_verify`, `quadcam_add_to_photos`, `quadcam_eject`, `quadcam_format_card`; the
  library `quadcam_library` (read-only search), `quadcam_library_edit` (rating, flag, name,
  details, aircraft, date, time), `quadcam_library_files` (cuts, Trash, Photos, rebuild);
  setup `quadcam_places` (list, search, save, delete), `quadcam_profiles` (list, save,
  delete, set_default), `quadcam_settings` (read, write, modules, module_install, module_remove). Keep the surface this small:
  add an action or a field to a tool before adding a tool. Each tool's arguments are a type in
  `mcp/params.rs`; `mcp/tools.rs` derives the input schema from it (schemars) and keeps the
  descriptions as written. The handler deserializes the arguments into that type once. A
  schema change shows in the `snapshots` test `mcp_tools`. Gear's tools are `quadcam_gear` (reads),
  `quadcam_gear_edit` (QuadCam's own data) and `quadcam_gear_apply` (devices; digest and
  `confirm=true`); a Gear feature adds an action to one of them, in `mcp/gear.rs`.
  `quadcam_library_edit` is one `library_update`
  call, which checks every id and value before any file changes.
- **The MCP and CLI surface is frozen at 1.0** (`SCHEMA_VERSION` in `schema.rs`, now 1;
  `docs/mcp.md` and `docs/cli.md` hold the Stability rules). To add to it: add the tool,
  `action` value, optional parameter, command or flag; give the MCP action a CLI command (or
  the reverse); name a concept as the surface already does (`log_dir`, `rate_profile`,
  `device` for a saved device id, `confirm` in MCP and `--yes` in the CLI, a plan before a
  write); run `QUADCAM_UPDATE_SURFACE=1 cargo test --test it surface::` and commit the changed
  baseline lines with `INSTA_UPDATE=always cargo test --test it snapshots::`. Never remove or
  rename an item, change a parameter's type, or make a parameter required: the surface test
  fails. To retire one, add a `Deprecation` to `schema::DEPRECATIONS` (the item keeps
  working and warns, in MCP and in the CLI), and remove it only in a later release that
  bumps `SCHEMA_VERSION`. A rename before the deprecation period uses a serde `alias`
  (MCP) or a clap `alias` (CLI) so the old name keeps working.
- **Updates:** while idle, `serve_stdio` stats the file it started from (`mcp/reload.rs`). A
  newer build settled there (probe: `QUADCAM_MCP_PROBE=1 quadcam-cli mcp`) runs in the same
  process through `exec`, with the session's state in `QUADCAM_MCP_HANDOFF`, and sends
  `notifications/tools/list_changed` when its tools differ. Never while a line is half read or
  `Core::restart_blocker` names a job; never to an older version. `HANDOFF_FORMAT` versions the
  handoff; a probe lists the formats a build reads. Tests: `tests/it/mcp_reload.rs` (`QUADCAM_MCP_TEST_BUILD`, cargo only).
- Agent suggestions show in the GUI with a dashed accent outline and an "agent" badge until
  the person edits the field. Read back with `quadcam_read_clips` before export.
- `quadcam_format_card` needs `device`, `volume_uuid` and `confirm=true` (read them with
  `dry_run=true`). `prep=true` (with `mount` for the dry run) is card prep, the `card_prep_plan`
  and `card_prep` methods in `core/prep.rs`. The plan names the file system and any size
  advice (`warnings`). With the app running, the person must also click Erase in the app;
  Cancel, closing the dialog or 3 minutes without a click refuses.
- Photos from the CLI or a headless MCP server runs PhotoKit in that process, so macOS
  attributes the permission prompt to the terminal. Prefer the app for Photos.

## Rules

- **Look at GPL code; never copy it.** QuadCam is MIT. Betaflight, EdgeTX, ExpressLRS and esptool
  are GPL. Read their source and docs to learn a protocol or file format, then write QuadCam's own
  code and tables. Talk to them only through protocols, files and separate processes. Never bundle
  a GPL binary in the app; download it from upstream at run time (`docs/gear-design.md`, Licensing).
- **Never erase a real disk while testing.** The format tests only erase a disk image they
  created, and they assert `BusProtocol == "Disk Image"` first. Keep it that way.
- Every format guard runs again inside `disk::format_card`, immediately before
  `diskutil eraseDisk`. Do not move a guard out of that path. It takes the source's
  `CardPolicy` and the path (`EraseBy`: refuses a policy that does not offer that path), writes
  the policy's file system (`erase_personality`: FAT32, proven on real cards; exFAT, tested on
  disk images only; anything else refuses), refuses a radio's card and a DJI device over USB
  (`disk::check_volume_contents`), and runs the caller's `recheck` last: the session's
  `format_ready` after an import, "every clip in the library" for card prep.
- **Never refuse a card for its size.** The other disk guards stay: not internal storage, not
  the boot disk or `disk0`, removable or ejectable (a built-in SD slot card counts), the same
  device and volume UUID. Size is advice only: `disk::format_advice` puts the policy's
  `size_warning` into `FormatPlan.warnings`, which the confirm shows.
- **DJI, two rules.** (a) Never format a DJI device over USB (an O4 air unit, goggles in
  storage mode): absolute. `check_volume_contents` refuses a DJI volume whose media name names
  DJI, or that is not a built-in-slot card while a DJI USB device is attached
  (`gear::events::dji_usb_attached` through `Env.dji_usb`, fails closed). (b) A removable DJI goggles card may be
  erased by card prep only, as exFAT, behind the every-clip-in-the-library guard. A card that
  once held DJI clips (empty `DCIM/DJI_*` folders) is a goggles card under (b).
- A source's `CardPolicy` says whether "Format card" (after an import, `format_offered`) and
  card prep (`prep_offered`) are offered for its cards. `Core::format_plan` and card prep refuse
  a source that does not offer them, before every other guard; the guards themselves stay in
  `disk::format_card`.
- "Format card" is never saved as a setting.
- Outputs are written under a hidden `.part` name and renamed only after verify passes.
  Never overwrite an existing file.
- Record corpus values in `test-clips/README.md` when the test clips change.
- **Never download from the internet while testing.** Module tests serve fixture archives
  from a local server; `modules::fetch::real_fetch` reaches only 127.0.0.1 under cargo. A
  module pin is an official upstream release asset with its SHA-256 checked against the
  publisher's (`docs/modules.md`). A new dependency must pass `node scripts/notices.mjs --check`.
- **Never ask Location Services, the geocoder or a weather service while testing.** In a process
  started by cargo, `locate::current` refuses unless `QUADCAM_LOCATION=real`, and
  `geocode::reverse` refuses unless `QUADCAM_FETCH=real`. Weather goes only through
  `Core.weather_fetch`; tests pass a fixture with `Core::with_weather_fetch`.
- **Never fill the real Trash while testing.** `trash::real_trash` returns a temporary-folder
  stand-in for any process started by cargo; tests pass `DirTrash`.
- **Never move the person's mouse or keyboard, or take focus, while testing.** Start the app
  with `QUADCAM_NO_FOCUS=1` (unfocused, behind other windows, never activated) and drive it
  with `QUADCAM_DEV_EVAL=<file>` (debug builds: runs the file's script in the window; replies
  come back as `dev-log` events on stderr). Screenshot with `screencapture -l <windowid>`.
  Pass `HOME=<temp>` so the settings, cache and session stay out of the real ones.
- **Never import into a real Photos library while testing.** In-process tests pass
  `photos::Recorder`. Tests that spawn the CLI set `QUADCAM_PHOTOS=dry-run`. As a fail-safe,
  `Core::real_photos` returns the recorder for any process started by cargo (it carries
  `CARGO_MANIFEST_DIR`) unless `QUADCAM_PHOTOS=real`. Never construct `photos::PhotoKit` in
  a test. Try the real path by hand in the built app only.
- **Betaflight fixtures are scrubbed.** `tests/fixtures/bf/` holds real `dump all`/`diff all`
  transcripts passed through `gear::bf::dump::scrub` (MCU id zeroed, every `*_name` value and the
  craft name replaced); `fixtures_are_scrubbed` fails on a UID-shaped value. Scrub any new one.
- **An FC is written only through `bf::run` (`Core::gear_fc_run`),** which the apply engine
  calls. It re-reads `version` and `mcu_id` in the CLI and re-runs the `compat` guard (board and
  build) and the device check right before the first write. It is not an `api` row.
- **An FC's firmware is flashed only by `Core::bf_flash_at`** (the `bfFlashPreview` setting, the plan's digest, `confirm`, and with the app running the person's click). `bf::to_bootloader` is the one place that sends `bl`, after the backup; `bl` stays in `cli::FORBIDDEN` for the write engine. The flash goes through `Flasher` (a recorder under cargo unless `QUADCAM_FLASH=real`) and the image through `Cloud` (reaches only 127.0.0.1 under cargo unless `QUADCAM_FETCH=real`). Never flash a real FC while testing.
- **Never open a real serial port, probe a real disk, or play a cue while testing.** A process
  started by cargo gets no serial ports (`gear::serial::system` is `NoPorts`), no presence or
  card-reader reads, a refused `events::probe_media`, and silent cues (`RecordedCues`), unless
  `QUADCAM_SERIAL=real` or `QUADCAM_CUES=real`, and no radio joystick (`radio_hid::system` is `NoHid`)
  unless `QUADCAM_HID=real`. Sim rate files come from `$HOME`; a cargo process reads running sims from `QUADCAM_SIMS_RUNNING` (comma separated process names), never from `ps`. **Never write a real sim file while testing:** `apply::sim::writes_allowed` refuses any sim write outside the temporary folder in a process started by cargo. Tests build cores with `Core::with_gear_env`
  and `gear::Env::fake` (synthetic volumes, `FakePorts`, `ScriptLink`).
- **Never write a real EdgeTX card while testing.** `edgetx::card::write` refuses a path under
  `/Volumes` in a process started by cargo unless `QUADCAM_CARD_WRITE=real`, and
  `edgetx::card::release` unmounts nothing unless `QUADCAM_SERIAL=real`. Tests write the
  synthetic card (`edgetx::synth`) into a temporary folder; a test that sets `Env.real_cards`
  to its own folder sees that refusal. EdgeTX files are edited line by
  line (`edgetx::yaml`), never through a YAML library.
- **Never `diskutil eject` a card; `safe_remove` (method `eject`) unmounts cards and ejects
  only non-card disks.** Only an unmounted card still inserted stays visible to Gear
  (`docs/gear-design.md`, 7.11).
- **Never depend on the Mac's own speech recognizer in a test.** A process started by cargo
  gets no `quadcam-intel` helper (`intel::Intel::system` is unavailable) unless
  `QUADCAM_INTEL=real`; tests pass `intel::fake::FakeHelper` through `Core::with_intel`. The one
  real-helper tests (`intel::tests::the_real_helper_hears_a_spoken_line`, and
  `the_real_helper_analyses_a_clip` with `QUADCAM_INTEL=real`) skip where their feature is
  unavailable; `intel::names::tests::the_real_helper_names_a_clip` runs the on-device model
  only with `QUADCAM_INTEL=real`.
- **Never check for an app update while testing.** A process started by cargo never runs the
  automatic update check unless `QUADCAM_UPDATER=real` (`updater::automatic_allowed`); the
  menu's Check for Updates still reaches the network, so do not click it in a test run. The
  mock core fakes the feed (`app/src/ipc/mock/update.ts`).
- **Never touch the real settings file or library while testing.** Tests and manual runs
  set `HOME` to a temp folder (the CLI and MCP derive every path from it) or pass
  `Core::with_settings` a temp file (`AutoBackend::with_settings` for a headless MCP core).
  As a fail-safe, a process started by cargo whose `HOME` is the account's own home panics on
  the support folder and every path under it (settings, gear folder, control socket) unless
  `QUADCAM_PATHS=real` (`paths::support_dir`; `tests/real_paths.rs` proves it). A core built
  without `with_settings` therefore refuses its default gear folder. Code that only finds
  things there (the installed modules, the tool settings) gets a temporary stand-in
  (`paths::support_dir_or_stand_in`). `cargo tauri dev` needs a temp `HOME` or `QUADCAM_PATHS=real`.
- The control socket and MCP server expose the same `Core::dispatch` methods. Add a feature
  to `Core` first, then add its row to the `api!` table (that makes the dispatch method and the
  typed GUI command), then wire it into the CLI and the MCP tools. Debug builds of the app
  rewrite `app/src/bindings.ts`; the `bindings` test fails when the committed file is stale. ¶
