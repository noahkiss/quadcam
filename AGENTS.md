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

## Where things are

| Topic | File |
|---|---|
| Module map, folder by folder | `docs/layout.md` |
| Build and test loop in detail (nextest profiles, Rust bumps, build cache, the UI) | `docs/build-and-test.md` |
| Release, CI, signing, the updater key | `docs/release.md` |
| How the core behaves (library, cards, settings, staging, Photos) | `docs/behaviour.md` |
| `quadcam-cli` and the MCP server, for scripts and agents | `docs/agent-usage.md` |
| Every rule below, with the code that enforces it | `docs/rules.md` |

Read the matching file before you change that area. Keep it in step with the code.

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

In `src-tauri/`:

```bash
cargo nextest run -P quick           # the fast loop: all but the disk-image and long ffmpeg tests
cargo nextest run --test it join::   # one area (tests/it/join.rs)
cargo nextest run                    # every test (needs ffmpeg)
cargo clippy --workspace --all-targets -- -D warnings
HOME=$(mktemp -d) cargo tauri dev    # run from source
QUADCAM_UPDATE_BINDINGS=1 cargo test --test it bindings::   # write app/src/bindings.ts after an api change
```

In `app/` (`pnpm install` first):

```bash
pnpm typecheck && pnpm lint && pnpm test   # tsc, eslint, Vitest
pnpm e2e                                   # Playwright, headless WebKit, on the mocked core
```

- **Test loop:** run `cargo nextest run -P quick` plus the tests of the area you touched. CI runs
  the full suite. On a busy machine cap it: `--test-threads 4`, `CARGO_BUILD_JOBS=4`.
- A second worktree sets its own `QUADCAM_DEV_PORT` (for example `5871`) for `pnpm dev` and
  `pnpm e2e`.
- `app/src/bindings.ts` is generated; never edit it. The `bindings` test fails when it is stale.

## Rules

Each rule's mechanism is in `docs/rules.md`. Read it before you touch disks, devices, settings
or tests.

- **Look at GPL code; never copy it.** QuadCam is MIT. Learn protocols from GPL projects, then
  write QuadCam's own code. Never bundle a GPL binary; download it from upstream at run time.
- **Never erase a real disk while testing.** Format tests erase only a disk image they created.
- **Every format guard runs again inside `disk::format_card`,** right before `diskutil eraseDisk`.
  Do not move a guard out of that path.
- **Never refuse a card for its size.** Size is advice only.
- **Never format a DJI device over USB.** A removable DJI goggles card may be erased by card prep
  only, as exFAT, once every clip is in the library.
- "Format card" is never saved as a setting.
- **Outputs are written under a hidden `.part` name** and renamed after verify. Never overwrite
  an existing file.
- **In a test, touch nothing real.** No internet, Location Services, geocoder, weather, Trash,
  Photos library, serial port, disk probe, cue, FC flash, EdgeTX card, sim file, speech
  recognizer, update check, settings file or library. A process started by cargo gets fakes;
  each has a `QUADCAM_*=real` switch for manual runs only.
- **Never move the person's mouse or keyboard, or take focus, while testing.** Start the app
  with `QUADCAM_NO_FOCUS=1` and a temporary `HOME`.
- **Betaflight fixtures are scrubbed** (`gear::bf::dump::scrub`). Scrub any new one.
- **An FC is written only through `bf::run`, and flashed only by `Core::bf_flash_at`.**
- **Never `diskutil eject` a card.** `safe_remove` unmounts cards.
- **Add a feature to `Core` first,** then its row in the `api!` table, then the CLI and MCP tools.
- Record corpus values in `test-clips/README.md` when the test clips change.
- A new dependency must pass `node scripts/notices.mjs --check`.
