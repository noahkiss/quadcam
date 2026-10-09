# Modules and notices

## Modules

A module is a tool that QuadCam does not ship. QuadCam downloads it from its upstream when you install it, checks it, and keeps it in its own folder:

```
~/Library/Application Support/app.quadcam/modules/<name>/<version>/
```

The app bundles no GPL tool. Each QuadCam release pins one version of each module.

| Module | Version | Used for | License | Download |
|---|---|---|---|---|
| ffmpeg and ffprobe | 9.0.2 | Converting, checking and previewing clips | GPL-3.0-or-later | Martin Riedl's FFmpeg build server, macOS arm64 release build |
| esptool | 5.4.0 | Flashing ESP32 and ESP8266 chips (ExpressLRS) | GPL-2.0-or-later | Espressif's GitHub release, macOS arm64 |

### What QuadCam needs

Every program QuadCam runs is a macOS system tool, a module, or optional. `tests/external_tools.rs` fails when the code runs a program that is not on this list.

| Program | Where it comes from | Used for |
|---|---|---|
| ffmpeg and ffprobe | The ffmpeg module. Homebrew's copy is the fallback | Converting, checking and previewing clips |
| esptool | The esptool module | Flashing ExpressLRS chips (planned for 1.1) |
| exiftool | Optional. QuadCam uses Homebrew's copy when it finds one | A second reader for the location tag when it verifies a clip. Nothing needs it |
| `diskutil`, `ioreg`, `system_profiler`, `df`, `ps` | macOS | Cards, disks, USB devices and free space |
| `curl`, `tar`, `unzip`, `zip`, `ditto`, `codesign`, `xattr` | macOS | Downloads and unpacking, signing a module |
| `say`, `afplay`, `osascript` | macOS | Spoken cues and voice previews |

QuadCam bundles no tool. The cask installs only the app and `quadcam-cli`.

### First run

When QuadCam finds no ffmpeg, a banner says so and import stays off. Select **Install ffmpeg** in the banner. QuadCam shows the version, size, license, source and download address first, and downloads nothing until you select **Download**. The tool works at once, with no restart. **Cancel** downloads nothing. Settings > Modules does the same for every module, and you can remove one there at any time.

### Install, update and remove

Open **Settings > Modules**.

- **Install** shows the module's version, size, license, source and download address. Nothing downloads until you select **Download**.
- QuadCam checks each download against the SHA-256 that the release pins. A mismatch deletes the file and stops the install: "The download does not match the expected checksum."
- **Check for updates** reads the newest pins that the latest QuadCam release published (`modules.json`). It installs nothing. A newer pin counts only when its download addresses are on the module's known hosts.
- **Update** installs the new version next to the old one, then removes the old one.
- **Remove** deletes the module's folder.

From a terminal or an agent:

```bash
quadcam-cli modules                       # list: pinned and installed versions, license, size, source
quadcam-cli modules install ffmpeg        # refuses (exit 3) and shows the license
quadcam-cli modules install ffmpeg --yes  # downloads, checks and installs
quadcam-cli modules check                 # newest pins from the latest release
quadcam-cli modules remove ffmpeg
```

An agent uses `quadcam_settings` with the actions `modules`, `module_install` (with `confirm=true`, only after it showed you the license) and `module_remove`.

### Firmware images

EdgeTX firmware is not a module. QuadCam downloads a release's zip from EdgeTX on GitHub when you plan a flash, checks it against the SHA-256 the release lists (or records the hash at the first download), and keeps it in `~/Library/Caches/app.quadcam/firmware/`. It is never run. QuadCam flashes over DFU with its own code, so it needs no `dfu-util`. See [Gear](gear.md#firmware-and-splash).

### Where ffmpeg comes from

The **Use ffmpeg from** setting (`ffmpegSource`) picks the source:

| Setting | QuadCam uses |
|---|---|
| **The QuadCam module, else Homebrew** (`module`, the default) | The ffmpeg module when it is installed. Without it, Homebrew (`/opt/homebrew/bin`, `/usr/local/bin`), then `PATH`, as before |
| **Homebrew** (`homebrew`) | Homebrew, then `PATH` |

The `modules` setting can name a file per tool, for example `{"ffmpeg": "/path/to/ffmpeg"}`. A file named there wins over both.

Settings > Modules shows the ffmpeg that is in use. The Homebrew cask does not install ffmpeg: the first run offers the module (see [First run](#first-run)). A Homebrew ffmpeg that is already installed keeps working as the fallback.

### How QuadCam runs a module

- QuadCam downloads with `/usr/bin/curl`, over HTTPS only. A downloaded file carries no quarantine attribute. If a module folder has one (a file copied in by hand), QuadCam removes it only after the checksum matched.
- Both pinned modules are signed with their maker's Developer ID and notarized by Apple. An unsigned upstream binary would get a local ad-hoc signature after the checksum matched.
- `installed.json` in the version folder records each tool's SHA-256. Before a tool runs, QuadCam checks the file against it. A changed file is refused: "ffmpeg was changed after install; reinstall it."
- A module runs only as a separate process, by its full path, with an argument list (never a shell). Nothing is loaded into QuadCam itself.

### How the pins were checked

Each pin in `src-tauri/resources/modules.toml` is an official release asset. For each one:

| Asset | SHA-256 | Checked |
|---|---|---|
| `ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffmpeg.zip` | `c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924` | Matches the server's `ffmpeg.zip.sha256`. The binary is signed by "Developer ID Application: Martin Riedl (KU3N25YGLU)"; `spctl` reports "Notarized Developer ID". `ffmpeg -version`: 9.0.2, configured with `--enable-gpl --enable-version3`, with `libx264` and `h264_videotoolbox` |
| `ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffprobe.zip` | `fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6` | Matches the server's `ffprobe.zip.sha256`. Signed and notarized as above |
| `github.com/espressif/esptool/releases/download/v5.4.0/esptool-v5.4.0-macos-arm64.tar.gz` | `ba332671130939e2e6db90c2784488f7e62a1459b0fe3c5ec66e9a366821de7a` | Matches the digest GitHub's release API reports for the asset. `esptool` is signed by "Developer ID Application: ESPRESSIF SYSTEMS (SHANGHAI) CO., LTD. (QWXF6GB4AV)" and notarized |

FFmpeg itself publishes source code only. Its download page links an Intel-only macOS build (evermeet.cx), so the ffmpeg module uses Martin Riedl's build server, which publishes signed and notarized arm64 release builds with checksums. No signed LGPL arm64 build was found; the build is GPL, which is fine for a tool QuadCam downloads and runs as a separate process. QuadCam's own MP4 encoder is VideoToolbox; the x264 setting needs a GPL build.

To change a pin: download the new asset, check its SHA-256 against the publisher's, check its signature with `codesign -dv` and `spctl -a -vv -t install`, update `modules.toml` and this table, and run the module tests.

## Third-party notices

The app includes Rust crates, npm packages, two fonts (Space Grotesk and JetBrains Mono, SIL Open Font License 1.1), the Solar icons (CC BY 4.0) and the Catppuccin palette. **QuadCam > Acknowledgements** shows their licenses. The same text is in the app at `QuadCam.app/Contents/Resources/THIRD_PARTY_NOTICES.txt`.

The build writes the file: `pnpm build` in `app/` runs `scripts/notices.mjs`, which reads `cargo metadata` and the installed npm packages. CI fails when a dependency has no license or one outside the allow list in that script (MIT, Apache-2.0, BSD, ISC, MPL-2.0, OFL-1.1, Unicode, Zlib, and the permissive BSL-1.0, CC0-1.0, Unlicense and CC-BY-4.0).
