# Build and test

The build and test loop in detail. `AGENTS.md` has the short form.

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
  branch push whose change touches the slow tests' code (the `ci.yml` path filter in `docs/release.md`).
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
