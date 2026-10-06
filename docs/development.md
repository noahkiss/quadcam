# Development

QuadCam is a Tauri 2 app. The core is Rust in `src-tauri/`. The frontend is React, TypeScript and Vite in `app/`.

## Requirements

- Rust, through [rustup](https://rustup.rs) (stable).
- ffmpeg and ffprobe from Homebrew. The app looks in `/opt/homebrew/bin` and `/usr/local/bin`, then `PATH`.
- Node.js 24 through [fnm](https://github.com/Schniz/fnm), which reads `.node-version`.
- pnpm, the version that `packageManager` in `app/package.json` names.
- exiftool is optional. The tests use it when it is installed.

```bash
brew install ffmpeg fnm pnpm
fnm install
(cd app && pnpm install)   # also installs the Tauri CLI
```

The Tauri CLI is a dev dependency of `app/` (`@tauri-apps/cli`). After `pnpm install`, run it as `app/node_modules/.bin/tauri`. To type `cargo tauri` instead, install it globally:

```bash
cargo install tauri-cli --version "^2" --locked
```

The commands below write `cargo tauri`. Either form works.

## Build

Run these in `src-tauri/`:

```bash
cargo tauri build                         # -> target/release/bundle/macos/QuadCam.app
cargo tauri dev                           # run from source (starts the dev server on port 4719)
cargo build --release --bin quadcam-cli   # -> target/release/quadcam-cli
```

`cargo tauri build` builds the frontend first (`pnpm --dir app build`). The bundle also holds the command-line tool, `QuadCam.app/Contents/MacOS/quadcam-cli`.

The Vite dev server uses port 4719 with `strictPort`.

`cargo tauri dev` does not touch your Photos library: Photos runs as a dry run. To try the real path, set `QUADCAM_PHOTOS=real`. See [Photos](photos.md#test-without-photos).

## Test

### Core

Run these in `src-tauri/`:

```bash
cargo test -- --test-threads=1                     # unit and integration tests
cargo test --test import size_and_speed -- --ignored --nocapture   # MP4 against MOV size and speed
INSTA_UPDATE=always cargo test --test snapshots    # accept a deliberate shape change, then review the diff
QUADCAM_UPDATE_BINDINGS=1 cargo test --test bindings   # write app/src/bindings.ts after an api change
```

- The tests need ffmpeg. They make their own synthetic clips with `ffmpeg -f lavfi -i testsrc`.
- The format tests attach small FAT32 disk images with `hdiutil`. They erase only an image that they created, and they check that the target is a disk image first.
- No test can reach your Photos library or your Trash. Under `cargo`, "Move to Trash" moves files into a temporary folder.
- `test-clips/README.md` describes an optional local corpus of real clips and logs. `scripts/make-corpus.sh` builds its synthetic part, a DJI-like card (`synthetic/dji-card/`) included.
- `tests/dji.rs` stages, dates, imports (MP4 copy and MOV remux) and verifies a synthetic DJI-like MP4 with a cover picture and an `.SRT` file.

### Frontend

Run these in `app/`:

```bash
pnpm typecheck && pnpm lint && pnpm test   # tsc, eslint, Vitest
pnpm e2e                                   # Playwright in headless WebKit, on a mocked core
pnpm build                                 # -> app/dist
```

- `pnpm dev`, then `http://localhost:4719/?mock=<scenario>`, runs the UI in a browser on the mocked core. The scenarios are in `app/src/ipc/mock/core.ts`.
- `app/scripts/make-fixtures.sh` records the mocked core's data from the real core. It needs the CLI and the corpus.
- `QC_SHOTS=<dir> pnpm exec playwright test e2e/shots.spec.ts` writes screenshots of each screen in both themes. The images in `docs/images/` come from this spec.

### CI

`.github/workflows/ci.yml` runs on every push and pull request:

- `cargo fmt --check`, clippy, the tests, and a release build.
- A `ui` job for `app/`: typecheck, lint, Vitest, Playwright, and a build.

### Try the app without it taking focus

- Start a debug build with `QUADCAM_NO_FOCUS=1`. The window opens unfocused, behind other windows.
- In a debug build, `QUADCAM_DEV_EVAL=<file>` runs the script in that file in the window, then deletes the file. A script reports back with `window.__TAURI__.event.emit("dev-log", text)`, which the app prints to stderr.
- Set `HOME` to a temporary folder. The settings, cache and session then stay out of your real ones.
- Take screenshots with `screencapture -l <windowid>`.

## Project layout

| Path | Holds |
|---|---|
| `app/` | The frontend: React, TypeScript, Vite, with bundled fonts and icons |
| `app/src/components/trim/` | The trim editor that the open clip and the Import sheet share |
| `app/src/bindings.ts` | The core's commands and types for the frontend, generated from `src-tauri/src/api/`. Never edit it by hand |
| `app/e2e/` | Playwright specs on the mocked core, and its fixtures |
| `src-tauri/src/api/` | The one table of core methods, with their params and results, and the app's events. It makes the socket's methods and the typed GUI commands |
| `src-tauri/src/core/` | The core that the app, the CLI and the MCP server drive: import, library, cuts, files (Photos, previews, Trash), and setup (settings, places, profiles) |
| `src-tauri/src/sources/` | Footage sources: how each video system's clips are found, checked, repaired and converted, and whether its card can be formatted. `analog.rs` (DVR AVI) and `dji.rs` (DJI MP4) |
| `src-tauri/src/identity.rs` | Clip ids: the XXH64 source fingerprint and head id, and the 0.4 legacy ids |
| `src-tauri/src/paths.rs` | Where QuadCam keeps its files under your home folder |
| `src-tauri/src/settings.rs` | The settings file: the one reader and writer, the setting names and their checks |
| `src-tauri/src/geocode.rs` | Place search: Apple MapKit, OpenStreetMap Nominatim, US Census, Google Places |
| `src-tauri/src/library.rs` | The library: layout, the index, and its rebuild from the files |
| `src-tauri/src/trim.rs` | Cut ranges and the rule for exported cuts, shared by the session and the library |
| `src-tauri/src/cuts.rs` | The one cut writer for the session and the library: cut, write metadata, verify, rename |
| `src-tauri/src/trash.rs` | Moving files to the Trash |
| `src-tauri/src/lib.rs` | The app's own Tauri commands |
| `src-tauri/src/control.rs` | The control socket |
| `src-tauri/src/mcp/` | The MCP server: protocol and handlers, each tool's argument type, the tool list with schemas derived from those types, the text answers |
| `src-tauri/src/bin/quadcam-cli.rs` | The command-line tool |
| `src-tauri/src/{scan,disk,media,logs,naming,session,photos}.rs`, `pipeline/` | Scanning, disks, ffmpeg, radio logs, file names, the session, Photos, the import steps |
| `src-tauri/src/moments.rs` | Moments from radio-log sticks and dead air from video frames. Every threshold is in `moments::tune` |
| `src-tauri/src/metadata.rs`, `qtmeta.rs` | Places, profiles and per-clip metadata. Writing QuickTime metadata into the files |
| `src-tauri/tests/` | Integration tests on synthetic clips and FAT32 disk images |
| `scripts/make-corpus.sh` | Builds `test-clips/synthetic/` |
| `assets/icon.svg` | The icon source. Regenerate the icons with `cargo tauri icon` |

[`AGENTS.md`](../AGENTS.md) holds the full module map and the rules for contributors and coding agents.

## Release

Users install the Homebrew cask `noahkiss/tap/quadcam`. A release is a pushed tag.

1. Set the new version in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. They must match.
2. Commit, then push the commit and a tag:

   ```bash
   git tag vX.Y.Z && git push origin main vX.Y.Z
   ```

3. `.github/workflows/release.yml` runs the shared composite actions `tauri-macos-build`, `macos-sign-notarize` and `release-attach` from [`noahkiss/workflows`](https://github.com/noahkiss/workflows). They run in this repo's own job, because that job names the `release` environment; a reusable workflow from another repo cannot read this repo's environment secrets.
   - It checks that both manifests state the tag's version.
   - It builds `QuadCam.app` on an Apple Silicon runner (`macos-26`) with the prebuilt Tauri CLI from `app/`.
   - It signs, notarizes and staples the app (see [Signing](#signing)).
   - It waits for a green CI run on the tagged commit. It does not run the tests again. If CI fails, or does not finish within 40 minutes, there is no release.
   - It zips the app as `quadcam-X.Y.Z-arm64.zip` and attaches the zip to the GitHub Release.
   - It starts the tap's `bump.yml`. That workflow rewrites `Casks/quadcam.rb`, installs the cask on macOS, and commits.
4. Watch it with `gh run watch -R noahkiss/quadcam --exit-status`.

To run the pipeline again for an existing tag:

```bash
gh workflow run release.yml -R noahkiss/quadcam --ref vX.Y.Z -f tag=vX.Y.Z
```

Run it on the tag (`--ref`): the `release` environment that holds the signing secrets admits only `v*` tags. The run uses `release.yml` as it stands at that tag.

A re-run never replaces a zip that is already attached. The cask's sha256 pins what was published, and a rebuilt zip is not byte-identical.

### Signing

The release signs the app with `Developer ID Application: NKMK Digital Co. (2Z88BYP37C)`, with hardened runtime and a secure timestamp. Apple's notary service checks it, and the ticket is stapled to the app before it is zipped. The shared action `macos-sign-notarize` does this, then verifies the result with `codesign --verify --deep --strict`, the team ID, `spctl --assess`, `xcrun stapler validate` and an online notarization check.

- The certificate and the App Store Connect API key are secrets of the `release` environment (`MAC_CERT_P12`, `MAC_CERT_PASSWORD`, `ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`). Only `v*` tags can use it.
- `src-tauri/Entitlements.plist` holds one entitlement: `com.apple.security.personal-information.photos-library`. Hardened runtime blocks PhotoKit without it. QuadCam needs nothing else: ffmpeg, ffprobe, curl and diskutil run as separate processes, the app is not sandboxed, and it sends no Apple Events.
- Local builds keep the ad-hoc signature (`signingIdentity "-"`), with the same hardened runtime and entitlements (`tauri.conf.json`).

