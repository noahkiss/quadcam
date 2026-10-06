# Settings

Open Settings with the gear icon or Command-comma. **Done** saves every change at once. **Cancel** or Escape leaves the settings as they were.

## Sections

| Section | Holds |
|---|---|
| **Library** | The library folder (default `~/Movies/quadcam`, made on the first import), the layout, the date in file names, place folders, keep originals, and **Rebuild from files**. See [The library](library.md). |
| **Aircraft** | The aircraft profiles and the default profile. See [Metadata](metadata.md#places-and-profiles). |
| **Places** | The saved places and the place search provider. See [Metadata](metadata.md#place-search-providers). |
| **Import** | The format (MP4 or MOV), the MP4 encoder, the default short name, the time in file names, and the tolerances for log matching, the DJI clip clock skew included. |
| **Photos** | The album. An empty album means the library only. See [Photos](photos.md). |
| **Advanced** | Where ffmpeg and the agent socket are. |

### Formats

- **MP4, H.264** (default) uses the VideoToolbox hardware encoder. **MP4 encoder** can pick x264 (software) instead. The files play everywhere and are much smaller: 14 times smaller on the synthetic test clips. Real, noisy footage compresses less.
- **MOV, original MJPEG frames** is a lossless remux that keeps the original frames.

## The settings file

QuadCam keeps every setting in one file:

```
~/Library/Application Support/app.quadcam/settings.json
```

The app, the command-line tool and the MCP server all read and write this file.

- Every writer changes only the settings it was asked to change. It keeps the rest, including keys that a newer or older QuadCam wrote.
- A change from the command line or an agent shows in the running app at once. The app never writes an older copy over it.
- The Settings window saves only the keys that you changed while it was open.
- Settings from QuadCam 0.3.0 carry over as they are.

To read and change settings from a terminal, see [Command line](cli.md#places-profiles-and-settings).
