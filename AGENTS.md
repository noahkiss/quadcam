# QuadCam

A macOS desktop app (Tauri 2) that imports analog FPV DVR clips (MJPEG AVI) into a library.
It stages the clips off the card, dates them (EdgeTX radio logs or the import date), names them
`YYYY-MM-DD_<name>.mp4`, converts them with ffmpeg, verifies them with ffprobe, and files them
by flying day. It can then add them to Photos and format the card to FAT32. The library is the
home screen; import is a sheet over it. `README.md` is the public user guide; keep it
in step with every change a user can see.

**Publishable repo:** no personal information in anything committed. That means no names,
hostnames, user paths, or personal gear setups. Name specific devices only as examples in the
README. Personal preferences go in the app's settings file on the machine
(`~/Library/Application Support/app.quadcam/settings.json`), never in code defaults.

## Layout

| Path | Holds |
|---|---|
| `app/` | New frontend: React + TypeScript + Vite (pnpm, Node pinned in `.node-version`). Builds to `app/dist`, which ships in the app beside a copy of `ui/` in `app/dist/legacy/`. Not yet the default; `docs/architecture-plan.md` section 4.2 has the steps |
| `ui/` | Legacy frontend, still the default: plain HTML, CSS and JS, no build step. `app.js` (library, import sheet, settings), `trim.js` (the one trim editor, used by clip detail and the import review). Icons and fonts are inlined or bundled so the app works offline |
| `src-tauri/src/` | Rust core. `core/` (`Core`) owns the session and the library index and is the one surface every front end drives: `mod.rs` (state, locking, `Hooks`), `import.rs` (stage, analyse, dates, import, verify, format), `library.rs` (the index, list, rate, edit, rename, redate), `cuts.rs` (session and library cut lists), `files.rs` (Photos, previews, strips, Trash) and `setup.rs` (settings, places, profiles). `api/` is the one method table: each row names a method, its params and result types and the `Core` call, and `api!` makes `Core::dispatch` (socket, MCP) and one typed Tauri command per method from it; `api/events.rs` holds the typed events. `lib.rs` holds the GUI's own Tauri commands, the legacy UI's commands (`core_call` runs any `dispatch` method), and `specta_builder`, which tauri-specta exports to `app/src/bindings.ts`; `control.rs` the app's socket, `mcp/` the MCP server (`server.rs` the protocol and handlers, `params.rs` each tool's argument type, `tools.rs` the tool list with schemas derived from those types, `render.rs` the text answers), `bin/quadcam-cli.rs` the CLI (clap flags build the `api` params, and it calls the table's `api::call` functions). The logic modules (`scan`, `disk`, `media`, `logs`, `moments`, `metadata`, `qtmeta`, `naming`, `pipeline` (with `pipeline/import.rs`, the import run), `session`, `photos`, `library`, `trim`, `cuts` (the one cut writer), `sources` (the `Source` trait per video system; `sources/analog.rs` is the DVR: clip layout, half-written check and repair, encode plan, dead air, card policy), `trash`, `settings` (with `Defaults`, the effective settings), `paths` (every path under `$HOME`), `geocode`) run without Tauri |
| `src-tauri/Info.plist` | Photos usage strings, merged into the bundle's Info.plist |
| `src-tauri/tests/` | Integration tests on synthetic clips and FAT32 disk images |
| `test-clips/` | Local test corpus. Git tracks only its README |
| `scripts/make-corpus.sh` | Builds `test-clips/synthetic/` |
| `assets/icon.svg` | Icon source. Regenerate with `cargo tauri icon` |

## Requirements

- Rust via rustup (stable), plus `cargo install tauri-cli --version "^2" --locked`.
- specta `=2.0.0-rc.25`, tauri-specta `=2.0.0-rc.25` and specta-typescript `=0.0.12` are release
  candidates: they stay pinned exactly, and an upgrade regenerates and reviews `app/src/bindings.ts`.
- ffmpeg and ffprobe from Homebrew (`brew install ffmpeg`). The app looks in
  `/opt/homebrew/bin` and `/usr/local/bin`, then `PATH`. exiftool is optional; tests use it
  when present.
- Node 24 through fnm (`.node-version`) and pnpm (`packageManager` in `app/package.json`), for
  `app/`; `cargo tauri dev` and `cargo tauri build` run it.
- **Which UI the window loads:** `QUADCAM_UI=next|legacy`, else the `ui` setting (`next` or
  `legacy`; no control in the Settings window, `quadcam-cli settings set ui=next`), else
  legacy. Both ship in every build: `app/dist`, and `ui/` copied to `app/dist/legacy/` by
  `app/vite.config.ts`. The dev server serves the same two paths. The default flips to `next`
  (plan step U8) after the owner's check. The Vite dev server is pinned to port 4719 with `strictPort`;
  `cargo tauri dev` starts it.

## Build, test, run

Run these in `src-tauri/`:

```bash
cargo test -- --test-threads=1  # unit + integration tests (needs ffmpeg; attaches small disk images)
cargo test --test import size_and_speed -- --ignored --nocapture   # MP4 vs MOV size/speed
INSTA_UPDATE=always cargo test --test snapshots   # accept a deliberate shape change, then review the diff
QUADCAM_UPDATE_BINDINGS=1 cargo test --test bindings   # write app/src/bindings.ts after an api change
cargo tauri dev                 # run from source (Photos is dry-run; QUADCAM_PHOTOS=real to test it)
QUADCAM_UI=next cargo tauri dev   # the same with the new UI in app/
cargo tauri build               # -> target/release/bundle/macos/QuadCam.app (runs pnpm --dir app build)
cargo build --release --bin quadcam-cli   # -> target/release/quadcam-cli
open target/release/bundle/macos/QuadCam.app
```

In `app/` (`pnpm install` first):

```bash
pnpm typecheck && pnpm lint && pnpm test   # tsc, eslint, Vitest
pnpm e2e                                   # Playwright, headless, both UIs on the mocked core
pnpm build                                 # -> app/dist
scripts/make-fixtures.sh                   # re-record the mock core's data (needs the CLI and the corpus)
```

- **Core calls in `app/`** go through `app/src/ipc/api.ts`, on the generated
  `app/src/bindings.ts` (never edit it). `ipc/types.ts` narrows the generated types for the UI
  (specta types every f64 as `number | null`; fields serde may omit are optional there), and
  `ipc/normalize.ts` makes each answer fit: a range or point without a time is dropped, a
  length reads as 0. The new UI uses only the typed commands, never the `@deprecated` legacy
  ones. The mock core answers both, and the parity specs compare calls by core method
  (`app.method(...)` in `app/e2e/fixtures.ts`), whichever command the UI used.
- **Theme** (`app/src/theme/`): `palette.css` is generated from `@catppuccin/palette`
  (`node scripts/gen-palette.mjs`; a test fails when it is stale). `tokens.css` maps it to
  semantic names (Mocha for dark, Latte for light) plus the 8-pt spacing, radii and type
  scale. Components use CSS Modules and tokens only, never a hex value. `e2e/theme.spec.ts`
  runs axe, contrast included, on the component gallery (`/?gallery` in dev) in both themes.
- **Dev in a browser:** `pnpm dev`, then `http://localhost:4719/?mock=<scenario>` runs the
  new UI on the mock core (scenarios in `app/src/ipc/mock/core.ts`).
- **Parity specs** (`app/e2e/parity/`) run every spec on both UIs (Playwright projects `legacy`
  and `next`) against `app/src/ipc/mock/`, a fake core behind a fake Tauri runtime. Specs find
  controls by role and label, never by class, so one spec fits both UIs. An area runs on
  `next` once it is listed in `PORTED` (`app/e2e/fixtures.ts`). The mock's data is the real
  core's JSON, recorded by `app/scripts/make-fixtures.sh`; regenerate it when a shape changes.

## Release

GitHub (`noahkiss/quadcam`, public) is the only remote. Users install the cask
`noahkiss/tap/quadcam`.

1. Set the new version in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. They must match.
2. Commit, then push a tag: `git tag vX.Y.Z && git push origin main vX.Y.Z`.
3. `.github/workflows/release.yml` tests, builds `QuadCam.app` on a `macos-26` (arm64) runner,
   zips it as `quadcam-X.Y.Z-arm64.zip`, and attaches it to the GitHub Release. It then fires
   the tap's `bump.yml`, which rewrites `Casks/quadcam.rb`, installs it on macOS, and commits.
4. Watch it: `gh run watch -R noahkiss/quadcam --exit-status`. Re-run for an existing tag with
   `gh workflow run release.yml -R noahkiss/quadcam -f tag=vX.Y.Z`.

- **Signing:** ad-hoc only (`signingIdentity "-"`, `hardenedRuntime false` in
  `tauri.conf.json`). There is no Developer ID and no notarization. The cask removes the
  quarantine attribute in `postflight_steps`. Hardened runtime stays off: without the Photos
  entitlement it blocks PhotoKit, and it has no use without notarization.
- **Secret:** `HOMEBREW_TAP_TOKEN` (the tap PAT) lets the release dispatch the tap.
- `.github/workflows/ci.yml` runs fmt, clippy, tests and a release build on every push and PR,
  and a `ui` job for `app/` (typecheck, lint, Vitest, Playwright, build).

## Behaviour notes

- **Library model:** the output folder is the library. `library::Layout` files clips as
  `YYYY/YYYY-MM-DD/` (default), `YYYY-MM-DD/` or flat, optionally with the place name on the
  day folder; originals go in `originals/` inside the day folder. The files are the source of
  truth: every detail (rating, flag, moments, keep ranges, flight stats, place, source
  fingerprint, Photos state) is an `app.quadcam.*` QuickTime item in the file.
  `<library>/.quadcam/index.json` is a rebuildable cache (`library::rebuild`); only unsaved cut
  ranges live in it alone. A rebuild never moves or writes a file, which is also how an
  existing export folder is adopted.
- **Identity:** a library clip's id is its DVR source's content fingerprint
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
- **Previews:** after `Core::analyse`, the GUI's `Hooks::analysed` runs `Core::make_previews` on a
  thread. Session and library previews both go through `Core::proxy_once`, which makes one
  proxy at a time (a static lock), so a Play click on a clip being made waits for it and reuses
  the file. Library MP4s play directly. Session and library Photos adds both go through
  `Core::share_files`, which also marks the library's clips as in Photos.
- **Native feel (UI):** page text is not selectable and the cursor is the arrow; WebKit needs
  `-webkit-user-select`. Text a person may copy (paths, details values) gets the
  `selectable` class. The web view's context menu shows only over text fields and selected
  `selectable` text; the app's own menus call `preventDefault` first.
- **Cards** show in the sidebar with an "N new" count (content fingerprints not in the index);
  inserting a card never starts an import on its own.

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
  `library_apply_name_format` renames existing clips (with cuts and originals) in place.
  Folder names keep `YYYY-MM-DD`.
- **Time of day:** a clip's plan time comes from its radio log or by hand (`PlanPatch.time`,
  `HH:MM`, empty for none). Without one the creation date is local noon. A library date or
  time edit (`library_edit`) rewrites the QuickTime creation date, the `mvhd` time and the
  mtimes of the clip, its cuts and its original; a new day moves them all (`relocate`, which
  rename uses too). A library profile edit rewrites make, model, aircraft, video system,
  profile name and the profile's keywords.
- **Session restore:** the GUI's core uses the same session file as the CLI. At launch,
  `Core::forget_unrestorable` drops it unless it was analysed and every staged clip still
  exists. "Start over" (`Core::clear`, method `clear`, `quadcam-cli clear`) deletes the file;
  the GUI also clears it after it erases a card.
- **Add to Photos:** per clip and "Add all" on the summary. `photos::PhotoKit` uses
  PhotoKit through `objc2-photos`: add-only access for the library, read-write when an
  album is set (default album `Drone`, created if missing; an empty setting means library
  only). Denied access returns a message that points to System Settings > Privacy &
  Security > Photos. Only outputs this session verified can be shared.

## Agent/CLI usage

`quadcam-cli` does everything the GUI does, on the same core. Every command takes `--json`
and prints one object: `{"ok":true,"result":...}` or
`{"ok":false,"error":{"code","exit","message"}}`. Exit codes: 0 ok, 1 failed, 2 usage,
3 refused by a safety guard, 4 no session or no card. Runs share a session file
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
quadcam-cli --json verify                      # re-check the session's outputs
quadcam-cli --json clear                       # forget the session, delete the session file
quadcam-cli --json library list --group picks  # also rate, rebuild, rename, edit, cut, trash, photos
quadcam-cli --json library edit <id> --date 2026-09-28 --time 18:30 --profile Whoop
quadcam-cli --json places search "Golden Gate Park" [--provider nominatim]
quadcam-cli --json places save NAME --location LAT,LON | --search QUERY [--pick N]
quadcam-cli --json profiles save NAME --aircraft .. --camera-make .. --models A,B --default
quadcam-cli --json settings set layout=day place_folders=true   # `settings` shows them
quadcam-cli --json library apply-name-format   # after settings set name_date_format=YY.MM.DD
quadcam-cli --json photos out.mp4 --album Drone
quadcam-cli --json eject
quadcam-cli --json format --plan               # runs every guard, prints device + volume UUID
quadcam-cli --json format --device /dev/diskN --volume-uuid <uuid> --yes
```

`format` refuses (exit 3) unless every GUI guard passes and `--device` (the card's whole
disk), `--volume-uuid` and `--yes` all match the staged card.

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
  delete, set_default), `quadcam_settings` (read, write). Keep the surface this small:
  add an action or a field to a tool before adding a tool. Each tool's arguments are a type in
  `mcp/params.rs`; `mcp/tools.rs` derives the input schema from it (schemars) and keeps the
  descriptions as written. The handler deserializes the arguments into that type once. A
  schema change shows in the `snapshots` tests: `mcp_tools` and the frozen
  `tests/fixtures/mcp_tools_before.json`. `quadcam_library_edit` is one `library_update`
  call, which checks every id and value before any file changes.
- Agent suggestions show in the GUI with a dashed accent outline and an "agent" badge until
  the person edits the field. Read back with `quadcam_read_clips` before export.
- `quadcam_format_card` needs `device`, `volume_uuid` and `confirm=true` (read them with
  `dry_run=true`). With the app running, the person must also click Erase in the app;
  Cancel, closing the dialog or 3 minutes without a click refuses.
- Photos from the CLI or a headless MCP server runs PhotoKit in that process, so macOS
  attributes the permission prompt to the terminal. Prefer the app for Photos.

## Rules

- **Never erase a real disk while testing.** The format tests only erase a disk image they
  created, and they assert `BusProtocol == "Disk Image"` first. Keep it that way.
- Every format guard runs again inside `disk::format_card`, immediately before
  `diskutil eraseDisk`. Do not move a guard out of that path.
- A source's `CardPolicy` says whether "Format card" is offered for its cards. `Core::format_plan`
  refuses a source that does not offer it, before every other guard; the guards themselves stay
  in `disk::format_card`.
- "Format card" is never saved as a setting.
- Outputs are written under a hidden `.part` name and renamed only after verify passes.
  Never overwrite an existing file.
- Record corpus values in `test-clips/README.md` when the test clips change.
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
- **Never touch the real settings file or library while testing.** Tests and manual runs
  set `HOME` to a temp folder (the CLI and MCP derive every path from it) or pass
  `Core::with_settings` a temp file.
- The control socket and MCP server expose the same `Core::dispatch` methods. Add a feature
  to `Core` first, then add its row to the `api!` table (that makes the dispatch method and the
  typed GUI command), then wire it into the CLI and the MCP tools. Debug builds of the app
  rewrite `app/src/bindings.ts`; the `bindings` test fails when the committed file is stale.
