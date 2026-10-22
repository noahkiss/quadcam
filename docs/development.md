# Development

QuadCam is a Tauri 2 app. The core is Rust in `src-tauri/`. The frontend is React, TypeScript and Vite in `app/`.

## Requirements

- Rust, through [rustup](https://rustup.rs). `rust-toolchain.toml` in the repo root pins the version; rustup installs it the first time you run `cargo`. CI uses the same file.
- ffmpeg and ffprobe from Homebrew, for the tests and for `cargo tauri dev`. The app looks in its ffmpeg module first (see [Modules](modules.md)), then in `/opt/homebrew/bin` and `/usr/local/bin`, then `PATH`.
- Node.js 24 through [fnm](https://github.com/Schniz/fnm), which reads `.node-version`.
- pnpm, the version that `packageManager` in `app/package.json` names.
- exiftool is optional. The tests use it when it is installed.
- Xcode 26 or newer, for the Swift helper `quadcam-intel` (below), and Xcode 27 for its video
  analysis. Without it the build still works and those features are unavailable.

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

The Vite dev server uses port 4719 with `strictPort`. Set `QUADCAM_DEV_PORT` to run `pnpm dev` and `pnpm e2e` on another port, for a second git worktree beside the first (Vite and Playwright read the same variable). `cargo tauri dev` always loads 4719.

### The Swift helper

`intel/` is a Swift package: `quadcam-intel`, a small command-line helper for Apple's on-device
frameworks that Rust cannot reach (they are Swift-only). It prints one JSON object per line:

```bash
quadcam-intel caps [--locale en-US]          # which features this Mac has, and why not
quadcam-intel asr FILE... [--locale en-US]   # SpeechAnalyzer + SpeechTranscriber: text, words, confidence per file
quadcam-intel install [--locale en-US]       # asks macOS to install the speech assets (AssetInventory)
quadcam-intel video FILE [--highlights] [--keyframe]   # MediaIntelligence VideoAnalyzer (macOS 27+): highlights, levels, key frame
quadcam-intel suggest [REQUEST.json | -]     # Foundation Models: clip names, keywords, notes; a day's summary
```

`suggest` reads a JSON request from a file or stdin. `{"task": "names", "examples": [...], "clips": [{"id", "facts", "frames"}]}` answers one `suggestion` line per clip (`name`, `keywords`, `note`, `images`, `ms`, or `error`) and a `group` line with one name for the outing. `{"task": "summary", "facts": "..."}` answers a `summary` line. QuadCam owns the facts (`core/ondevice.rs`); the helper owns the instructions and the `@Generable` answer shapes (`Names.swift`). Text runs on macOS 26; the frames join the prompt on macOS 27 (`caps` reports `suggest` with `images`). The model's context is 4096 tokens, so QuadCam sends one frame per clip and short facts. FoundationModels is weak-linked, so the helper still launches on macOS 13. Private Cloud Compute is not used: it needs an App Store entitlement. To try a prompt by hand, `/usr/bin/fm` on macOS 27 runs the same model once its license is accepted.

Exit codes: 0 done, 1 failed, 2 usage, 3 unavailable here (an older macOS, or a locale without
its assets). It builds with the newest SDK and launches on macOS 13: each feature checks the
running OS (`#available`) and reports itself unavailable on an older one.

- **Build.** `src-tauri/build.rs` runs `xcrun swift build -c release --arch arm64` (scratch in
  `target/intel-swift/`) and copies the result to `src-tauri/binaries/quadcam-intel-<target>`.
  `tauri.conf.json` names it as an external binary, so tauri-build copies it beside the app's
  own binaries (`target/<profile>/quadcam-intel`, next to `quadcam-cli`) and the bundler puts it
  in `QuadCam.app/Contents/MacOS/`. Without Xcode 26 or newer, build.rs writes a stand-in that
  answers "unavailable" and prints a cargo warning; with `QUADCAM_INTEL_REQUIRED=1` (CI and
  releases) it fails instead.
- **Signing.** The bundler signs it with the app (ad-hoc locally). A release's
  `macos-sign-notarize` signs every Mach-O inside the bundle with the Developer ID, hardened
  runtime and timestamp, so it is notarized with the app.
- **From Rust.** `src-tauri/src/intel.rs` finds it beside the running binary and runs it
  through `modules::run::Runner` (absolute path, minimal environment, a timeout). A missing
  helper, an older macOS or missing assets make a feature unavailable, never an error. A
  process started by cargo gets no helper unless `QUADCAM_INTEL=real`; tests pass
  `intel::fake::FakeHelper`. One test runs the real helper on a line `say` spoke, and skips
  where the speech check is unavailable. Another runs its video analysis on a synthetic clip
  with `QUADCAM_INTEL=real` on macOS 27, and skips elsewhere.
  `intel::names::tests::the_real_helper_names_a_clip` runs the model only with
  `QUADCAM_INTEL=real`, and skips where it is unavailable.
- **Video analysis.** `video` runs both requests in one pass (asking for one costs as much as
  both). `src-tauri/src/highlights.rs` turns the answer into `highlight` moments and a key
  frame, and caches it by content fingerprint under `<cache>/video/`; `core/video.rs` holds the
  session's background job (from `Hooks::analysed`) and `library_analyze_video`.

`cargo tauri dev` does not touch your Photos library: Photos runs as a dry run. To try the real path, set `QUADCAM_PHOTOS=real`. See [Photos](photos.md#test-without-photos).

`cargo tauri dev` with your own `HOME` stops at QuadCam's support folder, so it never writes your settings or gear folder. Run it with `HOME=$(mktemp -d)`, or set `QUADCAM_PATHS=real` to use your real ones.

## Test

### Core

Run these in `src-tauri/`:

```bash
cargo nextest run -P quick                         # fast loop: skips the disk-image and long ffmpeg tests
cargo nextest run                                  # every unit and integration test
cargo nextest run --test it join::                 # one area's tests (tests/it/join.rs)
cargo test                                         # also works, more slowly
cargo test --test it import::size_and_speed -- --ignored --nocapture   # MP4 against MOV size and speed
INSTA_UPDATE=always cargo test --test it snapshots::   # accept a deliberate shape change, then review the diff
QUADCAM_UPDATE_BINDINGS=1 cargo test --test it bindings::   # write app/src/bindings.ts after an api change
```

- The tests run under [cargo-nextest](https://nexte.st) (`cargo install cargo-nextest --locked`). nextest runs each test in its own process, many at once. `src-tauri/.config/nextest.toml` holds three profiles:

  | Profile | Runs |
  |---|---|
  | `default` | Every test |
  | `quick` (`-P quick`) | Every test except the disk-image tests and `tests/it/join.rs` |
  | `ci` | Every test, with a JUnit report and one retry for `tests/it/join.rs` and `tests/it/mount_cycle.rs`, which can fail on a loaded machine |
  | `ci-quick` | The `quick` tests with the `ci` settings. CI runs it on a branch whose change does not touch the slow tests' code |

- The integration tests are one binary per crate, `it`: `tests/it/main.rs` declares a module per area, so a test's name starts with its module (`join::keep_separate_per_run_per_clip_and_by_setting`). Each test binary links the whole crate, so one binary builds much faster than one per file. A new area is a new file in `tests/it/` and a `mod` line in `main.rs`. `tests/output_dir.rs`, `tests/rates.rs`, `tests/real_paths.rs` and `tests/voice_core.rs` set an environment variable that code in other areas reads; under `cargo test` that would leak into tests running in the same process, so each stays a binary of its own. So does the sim's `tests/bench.rs`, which times a real-time loop. `Cargo.toml` lists every test binary (`autotests = false`).
- While you work, run `-P quick` and the tests of the area you changed. CI runs on every push, on every branch (see [CI](#ci) for when it skips the slow tests).
- On a busy machine, cap the load: `CARGO_BUILD_JOBS=4 cargo nextest run --test-threads 4`.
- The disk-image tests are in the `serial` test group: they run one at a time, because `hdiutil` and `diskutil` share one system daemon and a detached image's disk number goes to the next image. They also take a lock file in the temporary folder, so disk-image tests in two checkouts take turns. A new test that attaches a disk image belongs in the `serial` filter and the `quick` exclusion.
- Three tests time the sim's real-time thread against the wall clock: the stall in `sim/tests/it/runner.rs`, the step cost in `sim/tests/bench.rs`, and the frame count in `tests/it/sim_host.rs`. They run alone, with no other test beside them, because the thread oversleeps on a loaded machine.
- nextest does not run doc tests. The crates have none. If you add one, run `cargo test --doc` too.
- Each git worktree has its own `target/`. [sccache](https://github.com/mozilla/sccache) shares compiled dependencies between them: `brew install sccache`, then `export RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0` (sccache does not cache incremental builds). With many worktrees, `CARGO_INCREMENTAL=0` saves disk space even without sccache.
- The tests need ffmpeg. They make their own synthetic clips with `ffmpeg -f lavfi -i testsrc`.
- The format tests attach small FAT32 disk images with `hdiutil`. They erase only an image that they created, and they check that the target is a disk image first.
- No test can reach your settings file or gear folder. A process started by `cargo` whose `HOME` is your own home panics on QuadCam's support folder (`~/Library/Application Support/app.quadcam`: settings, the gear folder, the control socket) unless `QUADCAM_PATHS=real`. A test sets `HOME` to a temporary folder or gives the core a temporary settings file (`Core::with_settings`). `tests/real_paths.rs` checks that a core without one refuses its default gear folder.
- No test can reach your Photos library or your Trash. Under `cargo`, "Move to Trash" moves files into a temporary folder.
- `test-clips/README.md` describes an optional local corpus of real clips and logs. `scripts/make-corpus.sh` builds its synthetic part, a DJI-like card (`synthetic/dji-card/`) included.
- `tests/it/dji.rs` stages, dates, imports (MP4 copy and MOV remux) and verifies a synthetic DJI-like MP4 with a cover picture and an `.SRT` file.
- `tests/it/join.rs` splits one synthetic 630 s recording with ffmpeg into a 600 s and a 30 s file, as an Echo does, and checks the joined import: one output with every frame, every file's identity in the library and the card count, kept originals, joining per run and per clip, and "Delete clips after import" on the joined output. No real Echo files are in the tests.
- `tests/it/modules.rs` runs the module manager against a fixture HTTP server on 127.0.0.1: the license refusal, download, checksum match and mismatch, install, run, a changed file, check and update, removal, and ffmpeg found through the module, Homebrew or a set path. `import_mp4_with_the_ffmpeg_module` imports with ffmpeg from a fixture module. No test downloads from the internet: under `cargo`, the downloader reaches only 127.0.0.1 unless `QUADCAM_FETCH=real`.
- `tests/it/gear_firmware.rs` checks the firmware path with no network and no USB: the check reads three sources from memory and only when asked, the flash plan (every check, the digest), the apply (a backup of the old firmware, erase, write, read back, leave) against a fake DFU device, the refusals (a board or version not in `compat.rs`, missing or doubled splash markers, no backup, no or two DFU devices, a bad read back that stays in DFU), the MCP actions and the CLI. The DFU code (`gear/dfu.rs`) and the splash patch have unit tests on a fake device and a synthetic binary.
- `tests/it/gear_bf_flash.rs` checks the Betaflight flash preview on a fake FC (`FakeFc`), a fake DFU device (`FakeDfu`) and a fake build service (`FixtureCloud`), with nothing real: the plan's refusals (preview off, board without a target, unproven release, a DFU device already attached, a bad image, no build, a USB timer that would run out), the whole flash with the settings put back and verified, a setting the new version lacks or refuses, the digest, confirm and click, an FC that never shows as DFU, a dropped block that leaves the FC in its bootloader, a flash that fails before the erase and says nothing was erased, a chip of the wrong size, a failed backup that stops before the bootloader, a battery in with no USB timer, and the old active PID and rate profile coming back. `app/e2e/bfflash.spec.ts` covers the Settings switch, the Firmware page and the sheet.
- `scripts/notices.test.mjs` (`node --test scripts/notices.test.mjs`) checks the license allow list, and that a planted bad license fails it.
- `tests/it/delete_clips.rs` checks "Delete clips after import" on temp folders: only verified clip files go, every other file stays, the setting off deletes nothing, a run can turn it off, and nothing in the core, CLI or MCP turns it on.

### Frontend

Run these in `app/`:

```bash
pnpm typecheck && pnpm lint && pnpm test   # tsc, eslint, Vitest
pnpm e2e                                   # Playwright in headless WebKit, on a mocked core
pnpm build                                 # -> app/dist
```

- `pnpm dev`, then `http://localhost:4719/?mock=<scenario>`, runs the UI in a browser on the mocked core. The scenarios are in `app/src/ipc/mock/core.ts`.
- `app/scripts/make-fixtures.sh` records the mocked core's data from the real core. It needs the CLI and the corpus.
- The axe specs loop over both themes (`axeSchemes` in `e2e/fixtures.ts`). `QC_AXE_THEMES=dark` (or `light`) runs them in one theme only; CI sets `dark` on a branch or a pull request and both on `main`, a `rel-*` branch and a tag. Every other spec runs whatever the variable says. A new axe spec loops over `axeSchemes`, not its own list.
- `QC_SHOTS=<dir> pnpm exec playwright test e2e/shots.spec.ts` writes screenshots of each screen in both themes. The images in `docs/images/` come from this spec.

### How to build a dashboard page

Every Gear page and device page follows one pattern. The shared parts live in `app/src/components/dash/`; import them, never copy them.

- **Frame.** `views/Gear/GearView.tsx` puts each page in `Gear.module.css .page`: a `--s-6` gap, and side padding `--page-inline`, which fills the content column (the window less the sidebar) up to `--content-max` (1760 px, `theme/tokens.css`) and centres it past that. The frame scrolls at the full width. A page returns its content (or one wrapper with `.stack` when it needs its own region name) and adds no padding and no max-width of its own; prose may keep a 70ch measure. A segment adds none either, so every page has one left edge.
- **Responsive.** The app reflows like a web app down to about 400 px (the window's minimum is 420). `main` is the size container `content`: page rules use `@container content (...)`, not the window. At 720 px of window and under, the sidebar is an overlay behind the toolbar's sidebar button and `--page-pad` drops to 16 px. Card grids use `repeat(auto-fill, minmax(min(340px, 100%), 1fr))`, so they drop to one column. `e2e/layout.spec.ts` checks every page at 420, 900, 1400 and 2400 px.
- **Paths.** Show every file path through `components/PathText.tsx` (`<PathText path selectable? />`): mono, `~` for the home folder, cut in the middle when it does not fit with the last segment kept, the whole path in its title and on a copy. Its parent must be able to shrink: give a grid or flex child that holds one `min-width: 0`.
- **Order.** `PageHeader` with a `SummaryLine` subtitle, then the page's sections or one `DataTable`. Keep `StatRow` tiles only where the numbers are the page's main job (Flights' day, Storage's usage).
- **Header.** `<PageHeader title subtitle? icon? actions? fold? more? moreLabel? id? />`. `actions` always show (a pop-up, the main action, primary last). `fold` buttons show in a wide header and join the `…` menu in a narrow one; `more` (rare and destructive actions) is always in the `…` menu. In a narrow header, `actions` wrap under the title. The subtitle is one line (kind · state · a fact, or the page's counts as a `SummaryLine`), never a sentence. Device pages use `components/gear/DeviceHeader.tsx` (`kind name state facts? actions?`), which is a `PageHeader`.
- **Rows.** A table row keeps one visible action, the one the row exists for; the rest go in its `MoreMenu`. Rows that need action come first; rows with nothing to do fold under a `Disclosure` ("5 passed"). A selected row's detail opens in a `DetailDrawer` on the right (`WithDrawer` around the page), never in a card under the table.
- **Empty.** `<EmptyState icon? action?>One line saying what is missing.</EmptyState>`, left-aligned where the content would be. A page with tiles keeps them, showing 0 or "—".

| Component | Props | Use for |
|---|---|---|
| `PageHeader` | `title`, `subtitle?`, `icon?`, `actions?`, `fold?`, `more?`, `moreLabel?`, `id?` (the heading's id, for the page's `aria-labelledby`) | The top of every page |
| `SummaryLine` | `parts: (string \| { text, tone? } \| null \| false)[]` | A page's counts as one line in the header's subtitle, joined by "·" |
| `StatRow`, `Stat` | `StatRow label`; `Stat label value sub? tone? meter? badge? onClick?` | Summary tiles, only where the numbers are the page's main job. `onClick` makes a tile open what it sums up. `tone` colours the value only |
| `Disclosure` | `summary`, `count?`, `defaultOpen?`, `remember?` (a storage key) | A closed part of a page: rows with nothing to do, rare detail. Its content renders only while open |
| `WithDrawer`, `DetailDrawer` | `WithDrawer drawer?`; `DetailDrawer title subtitle? actions? onClose` | A selected row's detail on the right; a full-width sheet over the page in a content column of 840 px or less |
| `SectionCard` | `title`, `label?`, `status?`, `actions?` | A titled block of a page. No card holds a single line |
| `DataTable`, `Column` | `label columns rows rowKey selection? group? compact? empty?` | Rows that share columns. `numeric` columns are mono and right-aligned; `group` adds group header rows; `optional` columns hide under 720 px. At 440 px and under each row stacks: the row header and `stack: "top"` cells (and the `actions` column) on the first line, the rest as a muted second line; `stack: "hide"` drops a cell |
| `FactList`, `Fact` | `facts: { label, value, selectable? }[]`, `label?` | Key and value rows of one object; two columns of pairs from 720 px |
| `StatusDot`, `StatusTone` | `tone: "ok" \| "warn" \| "error" \| "idle"`, children (the label) | A state in a table cell, a card head or a header |
| `Badge`, `BadgeTone` | `tone? title?`, children | A short mark beside a value or a row |
| `EmptyState` | `icon?`, `action?`, children | An empty page or section |
| `MoreMenu` | `items: (MenuItem \| null)[]`, `label?` (default "More actions") | Rare and destructive actions |

`/?gallery` in dev shows each of them in both themes, and `e2e/theme.spec.ts` runs axe on it. `views/Gear/DeviceLists.tsx` (Connected, Devices) and `views/Gear/Overview.tsx` are worked examples.

### Window chrome and theme

The window looks like an AppKit sidebar window on macOS 26 and later, and keeps the standard title bar on older macOS. `src-tauri/src/glass.rs` decides at launch (`glass::supported`, a runtime `available!` check) and sets it up:

- **Title bar.** `glass::configure` gives the window an overlay title bar with a hidden title, the traffic lights centred in the toolbar's height. It also sets `window.__QUADCAM_GLASS__` before the page loads.
- **Glass.** `glass::install` stops the web view drawing its background and puts an `NSGlassEffectView` (regular style, square corners, full height, `glass::SIDEBAR_WIDTH` wide) behind it. This matches the view an `NSSplitViewController` sidebar uses on macOS 27. The glass is decoration: the web view still takes every click.
- **Page.** `native.ts` `markGlass` sets `html[data-glass]`. Under it the page and the sidebar are clear, every other region paints `--bg`, and `App.module.css` moves the sidebar to the full height with the toolbar, banners and status bar beside it. `--focus` becomes `AccentColor`.
- **Dragging.** The toolbar (`data-tauri-drag-region="deep"`: its controls still click) and the strip above the sidebar move the window. The capability grants `core:window:allow-start-dragging`.
- **Sizes.** `--sidebar-w` and `--bar-h` in `tokens.css` must equal `glass::SIDEBAR_WIDTH` and `glass::BAR_HEIGHT`; `glass::tests` checks it.
- **Pop-up menus.** The shared `Select` uses `appearance: base-select` under `@supports`, with a picker drawn like `Menu`. Older WebKit opens the system menu.
- **Try it in a browser.** `?glass` in dev, or `window.__QUADCAM_GLASS__ = true` in an init script as `e2e/glass.spec.ts` does, gives the glass layout. The glass itself is native, so the sidebar shows the page's clear background there.

Keep a new frame region inside the grid areas in `App.module.css`, and give it its own background, or it shows the glass.

### The native map

The Map page (`views/Map/`) shows an Apple MapKit `MKMapView`, not MapKit JS (which needs a token). `src-tauri/src/mapview.rs` hosts it:

- **Place.** The map is a subview of the web view's parent, above the web view (the glass sits below it). The page keeps a placeholder (`MapHost`, `[data-map-host]`) and sends its rectangle with `map_frame` (CSS pixels and the page width, which gives the zoom); `null` hides the map. Between two reports the autoresizing mask keeps the map's margins, so a window resize stays aligned.
- **When it reports.** On every DOM change (a `MutationObserver`), and then every animation frame until the rectangle has kept still for 30 frames; a resize, a scroll or a CSS transition or animation starts the frames too, and a check every 500 ms catches a move nothing announced. It sends only a changed rectangle. At rest the page asks for no frames. Leaving the page sends `null`. With the Map page on, the app makes the native map at startup, hidden (`map_frame` with `null`), so the page opens without that wait.
- **Covered means hidden.** A native view draws above the whole page, so the map hides while a dialog, a menu, a listbox, a tooltip or a toast overlaps it, or while another element is on top at its centre or corners (`covered`). Keep the page's own controls outside the map's rectangle.
- **Pins.** `map_pins` sends the pins, only when they or the look differ from the last sent (a library or settings reload with nothing new on the map sends nothing) (`lib/map.ts` `mapData`: one per place with its clip count, one per saved place without clips, one per clip with a location and no place), the look (the `mapStyle` setting) and whether to zoom to them. Each pin carries what its callout shows: a place's `count`, `first` and `last` day; a clip's day, `time`, `sky`, `is_day` and poster frame (`thumb`). MapKit's own clusters zoom in. A new set of pins changes only what changed (`diff_pins`): a pin equal in every field keeps its annotation, its view and a shown callout, and the look is set again only when it changes (a configuration makes MapKit draw the whole map again).
- **Callouts.** A click on a pin shows MapKit's callout. `mapview::callout` turns a pin into the callout's lines, button, SF Symbol and VoiceOver label; it is pure, and its unit tests cover it. A pin's callout views are made the first time it is selected (`did_select`), not with its view: MapKit makes a pin view each time a pin scrolls into view, and its views cost about 1 ms a pin. The native view adds the lines and the sky as the detail view (`Detail`, a stack that pins its edges to MapKit's wrapper, which sizes itself to the stack and clips it but never places it), the poster frame on the left and the button (**Show clips**, **Open**) on the right. The button, or Return while the callout shows, sends the `map-pick` event with the pin's id (`place:<name>` or `clip:<id>`) and closes the callout; Escape closes it. The map view (`QuadCamMapView`, an `MKMapView` subclass) takes the keys while a callout shows, and gives them back to the web view when the map hides.
- **Scale.** The scale shows only while the zoom changes (`mapViewDidChangeVisibleRegion`) and hides 1.5 s after, as in Maps, so at rest it covers no label. AppKit has no `MKScaleView` to place it elsewhere.
- **Test it.** `tests/map_callout.rs` shows real callouts on a map in a window that never shows, and checks that every line sits inside its wrapper and that no two lines overlap. The mock core records `map_frame` and `map_pins` (`core.mapFrame`, `core.map`) and `core.mapPick(id)` plays a click; the placeholder shows in the browser (`e2e/map.spec.ts`). The real map needs the built app. With `QUADCAM_NO_FOCUS=1` the window sits behind others, so WebKit and MapKit stop drawing: a `screencapture` can show stale content. A debug build logs `map: loaded` and `map: rendered` to stderr. Run it outside a sandboxed shell: MapKit reaches its tiles through a system service.
- **Hand test** (the callout and the saved style need the built app and a person at the Mac):
  1. Build the app, start it with a library that has a saved place with clips on two days, a saved place without clips, and a clip with a location, no place, a time, a poster and weather. Turn on **Show the Map page** and open **Map**.
  2. Click the red pin. The callout shows the place, the clip count, and the first and last day. Click **Show clips**: the library shows that place's clips.
  3. Open **Map** again and click the gray pin. The callout shows **No clips** and no button.
  4. Click the blue pin. The callout shows the clip's name, its day and time, its poster frame, and the sky icon and word. Press Return: the clip opens. Open **Map** again, click the pin, and press Escape: the callout closes and the map stays.
  5. With a callout open, click **Satellite**: the look changes and the callout stays. Quit, start again, and open **Map**: it opens in Satellite. `quadcam-cli settings` shows `map_style` `satellite`.
  6. Turn on VoiceOver (Command-F5). Move to a pin: VoiceOver reads its name, kind and callout lines. Open its callout: VoiceOver reads the lines, the sky and the button (**Show clips: <place>**, **Open: <clip>**).
  7. After the map hides (open a clip, or another sidebar item), type in the library's search field: the keys reach the page.

### CI

`.github/workflows/ci.yml` runs on every push to a branch or a `v*` tag, and on pull requests from forks:

- `cargo fmt --check`, clippy, the tests (`cargo nextest run --profile ci`, with the JUnit report as the `junit` artifact), and a release build.
- On a branch or a pull request, the disk-image tests and `tests/it/join.rs` run only when the change since the merge base with `main` touches the code they cover: import, disks, cards, card prep, joins, media, scan, the session, the sources, the CLI's `main.rs`, the MCP server outside Gear, `tests/it/common` and those test files, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `nextest.toml` or `ci.yml`. Otherwise CI runs the `ci-quick` profile. The filter is in `ci.yml`; a new slow test, or new code it covers, joins it. On `main`, a `rel-*` branch and a tag, CI runs every test.
- A `ui` job for `app/`: typecheck, lint, Vitest, and a build.
- An `e2e` job: the Playwright specs in headless WebKit, split across four runners. It runs in Playwright's image (`mcr.microsoft.com/playwright`), whose tag matches `@playwright/test`; Renovate bumps both in one pull request. On a branch or a pull request, the axe specs run in the dark theme only.
- The Rust job and the release run on GitHub's `xcode-27` image (macOS 27, Xcode 27), a public preview. It builds the Swift helper; move to a GA image once one ships Xcode 27. The `ui` and `e2e` jobs need no macOS and run on `ubuntu-latest`.
- A push of `main` or a tag skips the tests when a `rel-*` branch's run already passed on the same commit (job `proven`), as it does in the release steps. Any other push of `main` runs everything.

A newer push to the same branch or pull request cancels the older run. A run on `main` or on a tag is never cancelled, because the release waits for a green run on the tagged commit.

### Try the app without it taking focus

- Start a debug build with `QUADCAM_NO_FOCUS=1`. The window opens unfocused, behind other windows.
- In a debug build, `QUADCAM_DEV_EVAL=<file>` runs the script in that file in the window, then deletes the file. A script reports back with `window.__TAURI__.event.emit("dev-log", text)`, which the app prints to stderr.
- Set `HOME` to a temporary folder. The settings, cache and session then stay out of your real ones. With your own `HOME`, a build run by `cargo` (`cargo tauri dev`, `cargo run`) stops at the support folder unless `QUADCAM_PATHS=real`.
- Take screenshots with `screencapture -l <windowid>`.

### Seed a dev HOME with made-up data

A temporary `HOME` starts empty, so every page shows its first-run screen. `scripts/dev-seed.py` fills one with a made-up library and gear:

```bash
(cd src-tauri && cargo build --bin quadcam-cli)
H=$(mktemp -d)
scripts/dev-seed.py "$H" --own-gear      # about 5 minutes the first time
HOME="$H" QUADCAM_NO_FOCUS=1 cargo tauri dev
```

- **What it makes:** 4 places, 3 aircraft profiles, 40 clips over 12 flying days (analog AVI and DJI MP4 clips from ffmpeg, with names, notes, places or a location, stars, picks and rejects, and radio-log moments and flight stats), and one EdgeTX log per aircraft and day. With gear: 3 pack types, 10 packs (one retired, three charged), every flight on a pack, and 2 repairs. It turns on the Map and Sim pages, ELRS tools and Betaflight flashing.
- **How it writes:** through `quadcam-cli` only, the app's own write paths. The clips are staged from generated "cards", dated against the generated logs and imported. The sources stay in `$H/.dev-seed/`.
- **Safety:** it refuses your real home folder, a folder that holds it, and a `HOME` whose library folder is outside it. It writes nothing outside `HOME`.
- **Gear:** packs, flights and repairs live in the gear folder. When `gearDir` points outside `HOME`, the seeder leaves it alone and skips them. `--own-gear` points `gearDir` at `HOME`'s own gear folder first.
- **Again:** a second run changes nothing; a day already in the library is skipped. `--force` deletes the seeded library and sources and imports them again. `--cli PATH` picks another `quadcam-cli` build.
- **Not seeded:** devices, backups and staged changes (the Devices, Bench and Storage pages need a device plugged in once), sim calibrations, weather and video analysis.

## Project layout

| Path | Holds |
|---|---|
| `app/` | The frontend: React, TypeScript, Vite, with bundled fonts and icons |
| `app/src/components/trim/` | The trim editor that the open clip and the Import sheet share |
| `app/src/bindings.ts` | The core's commands and types for the frontend, generated from `src-tauri/src/api/`. Never edit it by hand |
| `app/e2e/` | Playwright specs on the mocked core, and its fixtures |
| `src-tauri/src/api/` | The one table of core methods, with their params and results, and the app's events. It makes the socket's methods and the typed GUI commands |
| `src-tauri/src/core/` | The core that the app, the CLI and the MCP server drive: import, library, cuts, files (Photos, previews, Trash), and setup (settings, places, profiles) |
| `src-tauri/src/sources/` | Footage sources: how each video system's clips are found, checked, repaired and converted, whether its card can be formatted, and whether its imported clips can be deleted. `analog.rs` (DVR AVI) and `dji.rs` (DJI MP4) |
| `src-tauri/src/identity.rs` | Clip ids: the XXH64 source fingerprint and head id, and the 0.4 legacy ids |
| `src-tauri/src/paths.rs` | Where QuadCam keeps its files under your home folder |
| `src-tauri/src/settings.rs` | The settings file: the one reader and writer, the setting names and their checks |
| `src-tauri/src/geocode.rs` | Place search: Apple MapKit, OpenStreetMap Nominatim, US Census, Google Places; naming a point with Apple's geocoder |
| `src-tauri/src/locate.rs` | This Mac's location from Location Services, one fix per request. A process started by cargo never asks unless `QUADCAM_LOCATION=real` |
| `src-tauri/src/sun.rs` | Sun tags: the sun's position, sunrise and sunset from NOAA's formulas, offline |
| `src-tauri/src/weather.rs` | Weather from Open-Meteo through the `Fetch` trait (`core/weather.rs` writes it into clips); tests pass a fake |
| `src-tauri/src/library.rs` | The library: layout, the index, and its rebuild from the files |
| `src-tauri/src/trim.rs` | Cut ranges and the rule for exported cuts, shared by the session and the library |
| `src-tauri/src/cuts.rs` | The one cut writer for the session and the library: cut, write metadata, verify, rename |
| `src-tauri/src/trash.rs` | Moving files to the Trash |
| `src-tauri/src/lib.rs` | The app's own Tauri commands |
| `src-tauri/src/control.rs` | The control socket |
| `src-tauri/src/mcp/` | The MCP server: protocol and handlers, each tool's argument type, the tool list with schemas derived from those types, the text answers |
| `src-tauri/src/bin/quadcam-cli/` | The command-line tool |
| `src-tauri/src/{scan,disk,media,logs,naming,session,photos}.rs`, `pipeline/` | Scanning, disks, ffmpeg, radio logs, file names, the session, Photos, the import steps |
| `src-tauri/src/moments.rs` | Moments from radio-log sticks and dead air from video frames. Every threshold is in `moments::tune` |
| `src-tauri/src/highlights.rs` | Highlights and the key frame from macOS video analysis: moments, the file item, the cache |
| `src-tauri/src/metadata.rs`, `qtmeta.rs` | Places, profiles and per-clip metadata. Writing QuickTime metadata into the files |
| `intel/` | The Swift helper `quadcam-intel` (see [The Swift helper](#the-swift-helper)) |
| `src-tauri/src/intel.rs` | Runs the Swift helper: its features, the speech check of voice files, video analysis |
| `src-tauri/tests/` | Integration tests on synthetic clips and FAT32 disk images. `it/` is one test binary, one module per area; the three files beside it change an environment variable and stay binaries of their own |
| `scripts/make-corpus.sh` | Builds `test-clips/synthetic/` |
| `scripts/dev-seed.py` | Fills a temporary `HOME` with a made-up library and gear for a dev build ("Seed a dev HOME with made-up data" above) |
| `assets/icon.svg` | The icon source. Regenerate the icons with `cargo tauri icon` |

[`AGENTS.md`](../AGENTS.md) holds the full module map and the rules for contributors and coding agents.

## Release

Users install the Homebrew cask `noahkiss/tap/quadcam`. A release is a pushed tag.

1. On the release branch, set the new version in `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. They must match. Commit, push the branch, and wait for its CI run to pass.
2. Fast-forward `main` to that commit, then tag the same commit and push both. The main checkout must be clean, and the tag must name the release branch's head before you push it:

   ```bash
   set -e
   test -z "$(git status --porcelain)"
   git switch main && git merge --ff-only rel-X.Y.Z
   git tag vX.Y.Z
   test "$(git rev-parse 'vX.Y.Z^{commit}')" = "$(git rev-parse rel-X.Y.Z)"
   git push origin main vX.Y.Z
   ```

3. `.github/workflows/release.yml` runs the shared composite actions `tauri-macos-build`, `macos-sign-notarize` and `release-attach` from [`noahkiss/workflows`](https://github.com/noahkiss/workflows). They run in this repo's own job, because that job names the `release` environment; a reusable workflow from another repo cannot read this repo's environment secrets.
   - It checks that both manifests state the tag's version.
   - It builds `QuadCam.app` on an Apple Silicon runner (`xcode-27`, a public-preview image with Xcode 27) with the prebuilt Tauri CLI from `app/`. It restores the Rust dependency cache that `release-cache.yml` saves on `main` after each release; a cache saved under one tag is invisible to the next.
   - It signs, notarizes and staples the app (see [Signing](#signing)).
   - It waits for a green CI run on the tagged commit. Any green run counts, so the release branch's run passes it at once. It does not run the tests itself. If every run on the commit fails, or none passes within 40 minutes, there is no release.
   - It zips the app as `quadcam-X.Y.Z-arm64.zip` and attaches the zip to the GitHub Release, with the in-app updater's files: `quadcam-X.Y.Z-arm64.app.tar.gz`, its `.sig` and `latest.json`.
   - It starts the tap's `bump.yml`. That workflow rewrites `Casks/quadcam.rb`, installs the cask on macOS, and commits.
4. Watch it with `gh run watch <id> -R noahkiss/quadcam --interval 60 --exit-status`. The 3 s default hits GitHub's API rate limit when several watchers run.
5. Once the release is out, delete every branch it merged, local and remote (`git branch -d`, `git push origin --delete`), with their worktrees. Keep `main` and branches with unmerged work.

To run the pipeline again for an existing tag:

```bash
gh workflow run release.yml -R noahkiss/quadcam --ref vX.Y.Z -f tag=vX.Y.Z
```

Run it on the tag (`--ref`): the `release` environment that holds the signing secrets admits only `v*` tags. The run uses `release.yml` as it stands at that tag.

A re-run never replaces a zip that is already attached. The cask's sha256 pins what was published, and a rebuilt zip is not byte-identical.

### Signing

The release signs the app with `Developer ID Application: NKMK Digital Co. (2Z88BYP37C)`, with hardened runtime and a secure timestamp. Apple's notary service checks it, and the ticket is stapled to the app before it is zipped. The shared action `macos-sign-notarize` does this, then verifies the result with `codesign --verify --deep --strict`, the team ID, `spctl --assess`, `xcrun stapler validate` and an online notarization check.

- The certificate and the App Store Connect API key are secrets of the `release` environment (`MAC_CERT_P12`, `MAC_CERT_PASSWORD`, `ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`). Only `v*` tags can use it.
- `src-tauri/Entitlements.plist` holds two entitlements: `com.apple.security.personal-information.photos-library` and `com.apple.security.personal-information.location`. Hardened runtime blocks PhotoKit and Location Services (**Use current location**) without them. QuadCam needs nothing else: ffmpeg, ffprobe, curl, diskutil and `quadcam-intel` run as separate processes, the app is not sandboxed, and it sends no Apple Events.
- Local builds keep the ad-hoc signature (`signingIdentity "-"`), with the same hardened runtime and entitlements (`tauri.conf.json`).

