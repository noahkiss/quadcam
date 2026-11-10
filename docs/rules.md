# Rules

The project's hard rules in full, with the code that enforces each one. `AGENTS.md` lists them in short.

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
  disk images only; anything else refuses), refuses a volume that is not FAT or exFAT and a
  disk with more than one volume (`check_format_guards`, `check_one_volume`), refuses a radio's
  card, a DJI device over USB and a card holding a video file no source reads
  (`disk::check_volume_contents`, `sources::unread_videos`), and runs the caller's `recheck` last: the session's
  `format_ready` after an import, "every clip in the library" for card prep.
- **Never refuse a card for its size.** The other disk guards stay: not internal storage, not
  the boot disk or `disk0`, removable or ejectable (a built-in SD slot card counts), the same
  device and volume UUID. Size is advice only: `disk::format_advice` puts the policy's
  `size_warning` into `FormatPlan.warnings`, which the confirm shows.
- **DJI, two rules.** (a) Never format a DJI device over USB (an O4 air unit, goggles in
  storage mode): absolute. `check_volume_contents` refuses any volume, whatever it holds, whose
  media name names DJI, or that is not a built-in-slot card while a DJI USB device is attached
  (`gear::events::dji_usb_attached` through `Env.dji_usb`, fails closed, and reads `ioreg`
  under cargo too). (b) A removable DJI goggles card may be
  erased by card prep only, as exFAT, behind the every-clip-in-the-library guard. A card that
  once held DJI clips (empty `DCIM/DJI_*` folders) is a goggles card under (b).
- A source's `CardPolicy` says whether "Format card" (after an import, `format_offered`) and
  card prep (`prep_offered`) are offered for its cards. `Core::format_plan` and card prep refuse
  a source that does not offer them, before every other guard; the guards themselves stay in
  `disk::format_card`.
- "Format card" is never saved as a setting.
- Outputs are written under a hidden `.part` name and renamed only after verify passes.
  Never overwrite an existing file. The one in-place write is a metadata edit
  (`qtmeta::update`): it rewrites only the last box, `moov`, under a file lock, with the old
  `moov` in a `.<name>.moov` journal that the next open puts back after a crash
  (docs/metadata.md).
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
  rewrite `app/src/bindings.ts`; the `bindings` test fails when the committed file is stale.
