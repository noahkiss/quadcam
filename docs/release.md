# Release

How a QuadCam release is cut, built, signed and published.

GitHub (`noahkiss/quadcam`, public) is the only remote. Users install the cask
`noahkiss/tap/quadcam`.

1. On the release branch (`rel-X.Y.Z`, every change merged in), set the new version in
   `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (they must match), commit
   `release: X.Y.Z`, and push the branch. CI runs on that commit; a `rel-*` branch runs
   everything, as main does (every Rust test, axe in both themes).
2. Wait for that run to go green (`gh run watch <id> -R noahkiss/quadcam --interval 60 --exit-status`).
3. Fast-forward main to that same commit and tag it. No merge commit, no rebase: the tag
   names the commit that went green. Check that the main checkout is clean first, and check
   the tag against the release branch before you push it.

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
   (`quadcam-cli modules manifest`, the pins Settings > Modules checks) with its `modules.json.sig`
   (`tauri signer sign` with the updater key; the app ignores pins without it), and the in-app updater's
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
  retired without notice, and its queue can run long. The UI
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
  `release` environment sign the updater archive and `modules.json`; its public key is `plugins.updater.pubkey` in
  `tauri.conf.json`. `release-attach` tars the app after notarization and signs that tar with
  `tauri signer sign --app-version` (the config sets `requireSignedVersion`). The config leaves
  `createUpdaterArtifacts` off: Tauri's own archive would hold the app before notarization, and a
  local `cargo tauri build` would need the key. A lost key means no in-app update reaches installed
  copies, and no newer module pin either: they must update through Homebrew once.
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
