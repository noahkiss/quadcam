# quadcam

quadcam turns the SD card from analog FPV goggles into dated, named, playable video files. It copies the clips off the card, gives each clip a date and a name, and converts it to MP4. It verifies every file, and then it can clear the card for the next flight.

quadcam is a macOS desktop app. A command-line tool and an MCP server come with it, so scripts and coding agents can do everything the app does.

## What it does

Analog DVRs record MJPEG video in AVI files (`PICT0001.AVI` and similar). They have no clock, so the files carry no useful date. quadcam fixes that in one pass:

1. **Detect the card.** The app watches for removable volumes that hold AVI files, at the root or in folders such as `DCIM/`.
2. **Stage.** The app copies every clip to a local folder first, so you can pull the card early.
3. **Check.** The app probes each clip. It finds half-written files (for example, after a power-off during recording), recovers them, and marks empty files.
4. **Date.** Each clip gets a date from the best source available:
   - **Radio log:** if you point quadcam at EdgeTX logs, it matches armed flight segments to clips.
   - **Import date:** today, when no log matches.
   - **Edited:** any date you type.
5. **Name.** You give each clip a short name and an optional note. The file name is `YYYY-MM-DD_<name>.mp4`. An empty name becomes `flight`, `flight-2`, and so on.
6. **Convert.** The default is MP4 (H.264 with the VideoToolbox hardware encoder). The files play everywhere and are much smaller: 14 times smaller on the synthetic test clips. Real, noisy footage compresses less. A lossless MOV remux keeps the original MJPEG frames.
7. **Verify.** quadcam compares every output with its source: frame count, duration, streams, and metadata. Only verified files count as imported.
8. **Share.** You can add the files to the Photos app, into an album.
9. **Format (optional).** When every clip verified, you can erase the card as FAT32 and eject it. See [Format safety](#format-safety).

### Supported gear

quadcam works with any analog DVR that writes MJPEG AVI files, and with EdgeTX "SD Logs" CSV files (Date and Time in the first two columns).

It was built with these devices in mind. The automated tests use synthetic clips and logs in the same formats, not real cards:

- Fat Shark Echo goggles (MJPEG AVI in `DCIM/`, FAT32 card).
- A RadioMaster Pocket radio on EdgeTX, with logging while armed.

## Install

quadcam needs an Apple Silicon Mac with macOS 13 or later. Install it with Homebrew:

```bash
brew install --cask noahkiss/tap/quadcam
```

The cask installs:

- `quadcam.app` in `/Applications`.
- The command-line tool `quadcam-cli` on your `PATH`.
- ffmpeg, which quadcam needs to convert and verify clips.

To update, run `brew upgrade --cask quadcam`. To remove the app, run `brew uninstall --cask quadcam`. Uninstall keeps your settings and your videos.

### Unsigned app

quadcam is not signed with an Apple Developer ID and Apple did not notarize it. The release has an ad-hoc signature only. Homebrew marks every download as quarantined, and macOS blocks a quarantined app that Apple did not notarize. For this reason, the cask removes the quarantine attribute from `quadcam.app` after it installs the app.

If you download the zip from the [releases page](https://github.com/noahkiss/quadcam/releases) yourself, remove the attribute before the first start:

```bash
xattr -dr com.apple.quarantine /Applications/quadcam.app
```

## Build from source

You need Rust (through [rustup](https://rustup.rs)), the Tauri 2 CLI, and ffmpeg:

```bash
cargo install tauri-cli --version "^2" --locked
brew install ffmpeg
```

Build the app in `src-tauri/`:

```bash
cd src-tauri
cargo tauri build
```

The result is `src-tauri/target/release/bundle/macos/quadcam.app`. The bundle also holds the command-line tool, `quadcam.app/Contents/MacOS/quadcam-cli`.

Other commands:

```bash
cargo tauri dev     # run the app from source
cargo test          # unit and integration tests (see Tests)
```

The frontend is plain HTML, CSS, and JavaScript in `ui/`, with no build step and no Node.js.

## Use the app

1. Open quadcam and insert the card. The app copies the clips at once.
2. Optional: under **Radio logs**, select your radio's `LOGS` folder, or the radio itself in USB storage mode.
3. Check each date, and type a short name and a note.
4. Select **Skip** for clips you do not want, such as bench tests.
5. Select the output folder and the format, then select **Import**.
6. On the summary, select **Add all to Photos** if you want the files in Photos.
7. Eject the card, or format it (see below).

Settings (the gear icon) hold the default name, the Photos album, the encoder, and the tolerances for log matching. The default output folder is `~/Movies/quadcam`. The app creates it on the first import.

## Command line

`quadcam-cli` works on the same core as the app. Each run continues one session, which is saved to `~/Library/Caches/app.quadcam/session.json`.

```bash
quadcam-cli cards                         # detected cards and radio log sources
quadcam-cli scan /Volumes/NO\ NAME        # list clips, copy nothing
quadcam-cli stage /Volumes/NO\ NAME       # copy clips to staging, start a session
quadcam-cli analyze                       # probe, recover, make thumbnails
quadcam-cli dates --logs /path/to/LOGS    # date clips from radio logs
quadcam-cli dates --set 2=2026-10-03      # set one clip's date
quadcam-cli import --name 0=backyard-loops --skip 3 --format mp4
quadcam-cli import --plan plan.json       # names, dates, notes and options from a file
quadcam-cli verify                        # check the outputs again
quadcam-cli photos --album Drone          # add verified outputs to Photos
quadcam-cli eject
quadcam-cli format --plan                 # show what would be erased
quadcam-cli format --device /dev/diskN --volume-uuid <uuid> --yes
```

A plan file looks like this:

```json
{
  "clips": [
    { "id": 0, "name": "backyard loops", "date": "2026-10-04", "note": "two packs" },
    { "id": 3, "skip": true }
  ],
  "format": "mp4",
  "output_dir": "/path/to/output"
}
```

Add `--json` to any command to get one JSON object on stdout:

- On success: `{"ok": true, "result": ...}`
- On failure: `{"ok": false, "error": {"code": ..., "exit": ..., "message": ...}}`

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | The command failed |
| 2 | Wrong usage |
| 3 | A safety check refused the command |
| 4 | No session, or no card |

When settings from the app exist, the command-line tool uses them as its defaults.

## MCP server

`quadcam-cli mcp` runs an MCP server on stdio. It lets a coding agent import clips with you. To add it to Claude Code:

```bash
claude mcp add quadcam -- "$(brew --prefix)/bin/quadcam-cli" mcp
```

If you built from source, give the path to `quadcam.app/Contents/MacOS/quadcam-cli` instead.

If the app is running, the server works on the app's session. You see every change in the app as it happens. If the app is not running, the server works on the session file, like the command-line tool.

| Tool | What it does |
|---|---|
| `quadcam_status` | Shows the mode (app or headless), the cards, the radios, and the session |
| `quadcam_load_clips` | Stages, checks, and dates a card or a folder |
| `quadcam_read_clips` | Reads the clips and their plans; returns thumbnails as images |
| `quadcam_match_logs` | Dates the clips from EdgeTX logs |
| `quadcam_suggest` | Suggests names, dates, notes, or skips |
| `quadcam_export` | Converts and verifies; can also add to Photos |
| `quadcam_verify` | Checks the outputs again |
| `quadcam_add_to_photos` | Adds verified outputs to Photos |
| `quadcam_eject` | Ejects the card |
| `quadcam_format_card` | Erases the card (see below) |

An agent's suggestions show in the app with a dashed outline and an **agent** badge. When you edit a field, the value becomes yours. The agent reads the final values back before it exports.

The app serves the agent through a Unix socket at `~/Library/Application Support/app.quadcam/control.sock`. The socket uses JSON-RPC 2.0, one object per line. Only your user account can open it: the folder is `0700` and the socket is `0600`.

## Format safety

The format step erases a disk. quadcam makes it hard to erase the wrong one.

The **Format card** button unlocks only when all of these are true:

- The clips came from a card, not from a folder.
- Every clip copied off the card.
- Every clip that you did not skip verified in the output folder.

Before it erases, quadcam reads the disk information again and checks every guard:

- The disk is not internal, and it is removable or ejectable.
- The disk is not the boot disk and not `disk0`.
- The disk is 64 GB or smaller.
- The disk is the same card that the clips came from: same device and same volume UUID.

Then quadcam runs `diskutil eraseDisk FAT32 <NAME> MBRFormat /dev/diskN` on the whole card, and ejects it at once.

Each way in adds its own confirmation:

| From | Confirmation |
|---|---|
| The app | A dialog names the disk, the volume, the size, and the clip count. Only a click on **Erase** starts the erase. The Return key does not. |
| The command line | `--device`, `--volume-uuid`, and `--yes` must all be given, and they must match the card. |
| An agent (MCP) | The agent must give the device, the volume UUID, and `confirm=true`. If the app is running, you must also select **Erase** in the app. |

The format checkbox is never saved as on. If your goggles can format the card from their menu, that works too.

## Photos

Adding to Photos uses PhotoKit. The first time, macOS asks for permission. With an album set (the default is "Drone"), macOS asks for full library access, because quadcam must find the album. With no album, it asks only for permission to add.

Use the app to add to Photos. If you use the command-line tool, macOS asks for permission for your terminal app instead.

The environment variable `QUADCAM_PHOTOS` controls the Photos access:

| Value | Effect |
|---|---|
| `real` | Use PhotoKit |
| `dry-run` | Add nothing; report what would be added |
| unset | Use PhotoKit, except in processes started by `cargo`: those always use `dry-run` |

## Tests

```bash
cd src-tauri
cargo test
```

The tests need ffmpeg. They make their own synthetic clips with `ffmpeg -f lavfi -i testsrc`. GitHub Actions runs the same tests, clippy, and a build on every push and pull request.

- The format tests attach small FAT32 disk images with `hdiutil`. They erase only an image that they created, and they check that the target is a disk image first.
- No test can reach your Photos library.
- `cargo test --test import size_and_speed -- --ignored --nocapture` measures MP4 against MOV size and speed.

`test-clips/README.md` describes an optional local corpus of real clips and logs.

## Project layout

| Path | Holds |
|---|---|
| `ui/` | The app's frontend: HTML, CSS, JavaScript, bundled fonts and icons |
| `src-tauri/src/core.rs` | The core that every frontend drives |
| `src-tauri/src/lib.rs` | The app's Tauri commands |
| `src-tauri/src/control.rs` | The control socket |
| `src-tauri/src/mcp.rs` | The MCP server |
| `src-tauri/src/bin/quadcam-cli.rs` | The command-line tool |
| `src-tauri/src/{scan,disk,media,logs,naming,pipeline,session,photos}.rs` | Scanning, disks, ffmpeg, radio logs, file names, the import steps, the session, Photos |
| `src-tauri/tests/` | Integration tests |

## License

MIT. See [LICENSE](LICENSE).
