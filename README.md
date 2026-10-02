# QuadCam

QuadCam is a macOS app that imports the recordings from your FPV goggles into a video library. It copies the clips off the card, dates and names them, converts them to MP4, verifies every file, and files it by flying day.

![The QuadCam library in dark mode: clips grouped by flying day, with ratings, flags and a sidebar of aircraft, places and smart groups](docs/images/library-dark.png)

## Features

- **Dated clips.** Analog DVRs have no clock. QuadCam dates each clip from your EdgeTX radio logs, or uses the import date.
- **Named files.** Each file is `YYYY-MM-DD_<name>.mp4`, in a folder per year and per flying day.
- **Small MP4s.** Hardware H.264 makes files that play everywhere and are much smaller than the DVR's MJPEG. A lossless MOV option keeps the original frames.
- **Verified imports.** QuadCam compares every output with its source: frame count, duration, streams and metadata. It recovers half-written clips, for example after a power-off during recording, and marks empty ones.
- **Moments and dead air.** QuadCam finds rolls, flips, punch-outs and dives in the radio log, and the no-signal stretches in the video. You export any range as a cut.
- **A library you can trust.** Rate, flag, search, rename and edit your clips. Every detail is written into the file itself, and the index rebuilds from the files.
- **Metadata that Photos reads.** Date, time, place, aircraft and keywords go into QuickTime tags. Add clips to Photos, into an album.
- **Safe card format.** When every clip verified, QuadCam can erase the card as FAT32 and eject it, behind strict guards.
- **Scripts and agents.** A command-line tool and an MCP server do everything the app does.

## Supported gear

Today, QuadCam reads **analog goggle and DVR recordings**: MJPEG video in AVI files (`PICT0001.AVI` and similar), on a card or in a folder. The app watches for removable volumes that hold AVI files, at the root or in folders such as `DCIM/`. For dates and moments, it reads EdgeTX "SD Logs" CSV files.

Support for **DJI, HDZero and Walksnail** recordings is planned. QuadCam does not read them yet.

QuadCam was built with a Fat Shark Echo and a RadioMaster Pocket on EdgeTX in mind. The automated tests use synthetic clips and logs in the same formats, not real cards.

## Install

QuadCam needs an Apple Silicon Mac with macOS 13 or later. Install it with Homebrew:

```bash
brew install --cask noahkiss/tap/quadcam
```

The cask installs:

- `QuadCam.app` in `/Applications`
- the command-line tool `quadcam-cli` on your `PATH`
- ffmpeg, which QuadCam needs to convert and verify clips

To update, run `brew upgrade --cask quadcam`. To remove the app, run `brew uninstall --cask quadcam`. Uninstall keeps your settings and your videos.

### The app is not notarized

QuadCam has an ad-hoc signature only. It has no Apple Developer ID, and Apple did not notarize it. macOS blocks such an app when the download is quarantined, so the cask removes the quarantine attribute after it installs the app.

If you download the zip from the [releases page](https://github.com/noahkiss/quadcam/releases) yourself, remove the attribute before the first start:

```bash
xattr -dr com.apple.quarantine /Applications/QuadCam.app
```

## Quick start

1. **Load the clips.** Insert the card. It shows in the sidebar under **Import from**, with the number of new clips. Select it. You can also select **Folder…**, or drag a folder or AVI files onto the window.
2. **Wait for the copy.** The Import sheet opens and copies every clip to a local folder first. The sheet header shows how many files are left and the progress of the current one. You can pull the card when the copy is done.
3. **Review.** Set the aircraft, the place and the date for all clips at once, or per clip. Type a short name for each clip. Select **Skip** for clips you do not want, such as bench tests.
4. **Add radio logs (optional).** Under **Radio logs**, select **Choose…** and pick your radio's `LOGS` folder. QuadCam then dates the clips and finds their moments.
5. **Trim (optional).** Select a clip to play it, see its moments and set cuts.
6. **Add to Library** (Command-Return). QuadCam converts and verifies each clip.
7. **Finish.** Add the files to Photos, eject the card, or format it. **Done · show in Library** shows the new clips under **Last import**.

![The Import sheet on the Review step: a list of clips with names, dates and times, and the selected clip's preview and trim editor](docs/images/import-review-dark.png)

You can close the sheet at any time. The import stays in the sidebar as **Unfinished import** until you finish it.

Open a clip in the library to edit its details, rate it, and trim it:

![An open clip: the video preview, its moments, the trim timeline with a cut, and the Details tab](docs/images/detail-dark.png)

QuadCam follows the system's light or dark appearance. [A light-mode screenshot](docs/images/library-light.png) shows the library in light mode.

## Documentation

| Topic | Covers |
|---|---|
| [The library](docs/library.md) | Folder layout, file names, dates, the index, clip identity, keys and menus |
| [Moments and cuts](docs/moments-and-cuts.md) | Dead air, radio-log moments, the trim editor, cut files |
| [Metadata](docs/metadata.md) | QuickTime tags, places and place search providers, aircraft profiles, radio logs |
| [Settings](docs/settings.md) | Each Settings section and the settings file |
| [Photos](docs/photos.md) | Albums, permissions, `QUADCAM_PHOTOS` |
| [Format safety](docs/format-safety.md) | When QuadCam erases a card, and the guards it checks first |
| [Command line](docs/cli.md) | `quadcam-cli`: import, library, places, profiles, settings, JSON output |
| [MCP server](docs/mcp.md) | Connect a coding agent, the 16 tools, the control socket |
| [Development](docs/development.md) | Build from source, tests, project layout, releases |

## Privacy

QuadCam works on your Mac. It sends data out only when you search for a place: the search text goes to the provider you picked. See [Metadata](docs/metadata.md#place-search-providers).

## License

MIT. See [LICENSE](LICENSE).
