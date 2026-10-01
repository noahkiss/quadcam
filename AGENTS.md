# quadcam

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
| `ui/` | Frontend: plain HTML, CSS and JS, no build step. `app.js` (library, import sheet, settings), `trim.js` (the one trim editor, used by clip detail and the import review). Icons and fonts are inlined or bundled so the app works offline |
| `src-tauri/src/` | Rust core. `core.rs` (`Core`) owns the session and the library index and is the one surface every front end drives; `core_library.rs` holds its library methods. `lib.rs` holds the Tauri commands (`core_call` runs any `Core::dispatch` method), `control.rs` the app's socket, `mcp.rs` the MCP server, `bin/quadcam-cli.rs` the CLI. The logic modules (`scan`, `disk`, `media`, `logs`, `moments`, `metadata`, `qtmeta`, `naming`, `pipeline`, `session`, `photos`, `library`, `trim`, `trash`) run without Tauri |
| `src-tauri/Info.plist` | Photos usage strings, merged into the bundle's Info.plist |
| `src-tauri/tests/` | Integration tests on synthetic clips and FAT32 disk images |
| `test-clips/` | Local test corpus. Git tracks only its README |
| `scripts/make-corpus.sh` | Builds `test-clips/synthetic/` |
| `assets/icon.svg` | Icon source. Regenerate with `cargo tauri icon` |

## Requirements

- Rust via rustup (stable), plus `cargo install tauri-cli --version "^2" --locked`.
- ffmpeg and ffprobe from Homebrew (`brew install ffmpeg`). The app looks in
  `/opt/homebrew/bin` and `/usr/local/bin`, then `PATH`. exiftool is optional; tests use it
  when present.
- No Node. There is no dev server, so there is no port.

## Build, test, run

Run these in `src-tauri/`:

```bash
cargo test -- --test-threads=1  # unit + integration tests (needs ffmpeg; attaches small disk images)
cargo test --test import size_and_speed -- --ignored --nocapture   # MP4 vs MOV size/speed
cargo tauri dev                 # run from source (Photos is dry-run; QUADCAM_PHOTOS=real to test it)
cargo tauri build               # -> target/release/bundle/macos/quadcam.app
cargo build --release --bin quadcam-cli   # -> target/release/quadcam-cli
open target/release/bundle/macos/quadcam.app
```

## Release

GitHub (`noahkiss/quadcam`, public) is the only remote. Users install the cask
`noahkiss/tap/quadcam`.

1. Set the new version in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. They must match.
2. Commit, then push a tag: `git tag vX.Y.Z && git push origin main vX.Y.Z`.
3. `.github/workflows/release.yml` tests, builds `quadcam.app` on a `macos-26` (arm64) runner,
   zips it as `quadcam-X.Y.Z-arm64.zip`, and attaches it to the GitHub Release. It then fires
   the tap's `bump.yml`, which rewrites `Casks/quadcam.rb`, installs it on macOS, and commits.
4. Watch it: `gh run watch -R noahkiss/quadcam --exit-status`. Re-run for an existing tag with
   `gh workflow run release.yml -R noahkiss/quadcam -f tag=vX.Y.Z`.

- **Signing:** ad-hoc only (`signingIdentity "-"`, `hardenedRuntime false` in
  `tauri.conf.json`). There is no Developer ID and no notarization. The cask removes the
  quarantine attribute in `postflight_steps`. Hardened runtime stays off: without the Photos
  entitlement it blocks PhotoKit, and it has no use without notarization.
- **Secret:** `HOMEBREW_TAP_TOKEN` (the tap PAT) lets the release dispatch the tap.
- `.github/workflows/ci.yml` runs fmt, clippy, tests and a release build on every push and PR.

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
  (`app.quadcam.source`), never a file name. A file quadcam did not write is known by a hash of
  its first MB before `moov` (`library::head_id`), which metadata rewrites never touch.
- **Cuts:** `trim.rs` is the one cut model for the session and the library. Dropping a cut that
  was already exported needs a decision (`RemovedCuts::Keep`: the file stays as its own clip,
  marked `app.quadcam.detached`; `Trash`: it goes to the Trash). Without one, `Core` answers
  `CutChange::Confirm` (GUI) or refuses the patch (CLI `--removed`, MCP `removed_cuts`).
- **Previews:** after `Core::analyse`, the GUI's `Hooks::analysed` runs `Core::make_previews` on a
  thread. `Core::preview` makes one proxy at a time (a static lock), so a Play click on a clip
  being made waits for it and reuses the file. Library MP4s play directly.
- **Cards** show in the sidebar with an "N new" count (content fingerprints not in the index);
  inserting a card never starts an import on its own.

- **Output folder:** defaults to `~/Movies/quadcam`, resolved from `$HOME` at runtime
  (`pipeline::default_output_dir`). The app creates that default, with its parents, on the
  first import. It never creates a folder the user picked. Picking a folder saves it as the
  setting.
- **Settings:** the GUI saves them in the Tauri store file `settings.json` (keys
  `outputDir`, `format`, `formatLabel`, `photosAlbum`, and others) and pushes them to the core.
  The headless CLI and MCP server read the same file (`Defaults::with_app_settings`).
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
quadcam-cli --json import --plan plan.json     # {"clips":[{"id":0,"name":"..","date":"..","note":"..","skip":false}],"format":"mov","output_dir":".."}
quadcam-cli --json verify                      # re-check the session's outputs
quadcam-cli --json clear                       # forget the session, delete the session file
quadcam-cli --json library list --group picks  # also rate, rebuild, rename, edit, cut, trash, photos
quadcam-cli --json photos out.mp4 --album Drone
quadcam-cli --json eject
quadcam-cli --json format --plan               # runs every guard, prints device + volume UUID
quadcam-cli --json format --device /dev/diskN --volume-uuid <uuid> --yes
```

`format` refuses (exit 3) unless every GUI guard passes and `--device` (the card's whole
disk), `--volume-uuid` and `--yes` all match the staged card.

### MCP server

`quadcam-cli mcp` is an MCP server on stdio, a thin layer over the same core. `cargo tauri
build` also puts the CLI inside the bundle (`quadcam.app/Contents/MacOS/quadcam-cli`), and the
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
- Tools: `quadcam_status`, `quadcam_library` (read-only library search), `quadcam_load_clips`, `quadcam_read_clips` (thumbnails as image
  content), `quadcam_match_logs`, `quadcam_suggest`, `quadcam_export`, `quadcam_verify`,
  `quadcam_add_to_photos`, `quadcam_eject`, `quadcam_format_card`.
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
  Pass `HOME=<temp>` so the store, cache and session stay out of the real ones.
- **Never import into a real Photos library while testing.** In-process tests pass
  `photos::Recorder`. Tests that spawn the CLI set `QUADCAM_PHOTOS=dry-run`. As a fail-safe,
  `Core::real_photos` returns the recorder for any process started by cargo (it carries
  `CARGO_MANIFEST_DIR`) unless `QUADCAM_PHOTOS=real`. Never construct `photos::PhotoKit` in
  a test. Try the real path by hand in the built app only.
- The control socket and MCP server expose the same `Core::dispatch` methods. Add a feature
  to `Core` first, then wire it into the GUI, the CLI and the MCP tools.
