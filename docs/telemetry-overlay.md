# Telemetry overlay

A DJI O4 air unit records a clean picture: the Betaflight OSD lives only in the goggles. An
analog DVR records the OSD in the picture, but often too small to read. The telemetry overlay
puts the flight's numbers back on a library clip: battery, link, throttle, flight mode and the
flight timer.

## What it makes

For one library clip, QuadCam writes:

- **A subtitle file** (`.srt`, SubRip), always. IINA, VLC and QuickTime Player with a
  subtitle plug-in show it, and editors such as DaVinci Resolve and Final Cut Pro import it as
  a caption track. Nothing is re-encoded.
- **A burned-in copy** (`.mp4`), with `burn`. The numbers are drawn into the picture at the
  bottom left, in a fixed-width font so they do not jitter. This re-encodes the clip with the
  encoder setting (VideoToolbox, else x264) and takes about as long as an analog import.

The files go to the `out` folder, else the [`overlay_dir` setting](settings.md#telemetry-overlay-folder),
by default `~/Movies/quadcam-overlays/`. They are named after the clip: `<clip name>.telemetry.srt`
and `<clip name>.telemetry.mp4`.

- QuadCam never replaces a file there unless the run says `overwrite`. Without it, a second run
  adds ` 2`, ` 3` to the name.
- Each file is written under a hidden `.part` name and moved into place when it is complete. A
  failed burn leaves no half-written copy.
- The library folder is never written: a video in it would show up as a clip to scan. QuadCam
  refuses an `out` folder inside it, also by a symlink, and a relative path.
- The clip file is only read, never opened for writing.

## Where the numbers come from

| Source | When | Numbers |
|---|---|---|
| `log`: the EdgeTX radio log | The clip matches a radio log, as `match_logs` reports it ("matched" or "likely") | Receiver battery, battery %, mAh used, current, link quality, RSSI, SNR, transmit power, throttle, flight mode, flight timer |
| `srt`: the goggles' `.srt` sidecar | The clip has a kept DJI `.SRT` beside its original, or `srt` names a file | Air-unit battery, goggles battery, signal, delay, bitrate, distance, flight time |

`source` picks one; without it QuadCam takes the radio log when the clip matches one, else the
sidecar. The sidecar reader takes the DJI goggles format (`signal:4 ... uavBat:16.2V
delay:25ms`) and the Walksnail Avatar format (`Signal:4 CH:8 ... SBat:9.9V Delay:48ms`). Keys it
does not know are listed in the answer, so a new format shows what it holds.

A radio-log overlay shows numbers only while the log has rows for the clip: from the first
armed row of the first flight to the last armed row of the last. The flight timer counts armed
time only, across the clip's flights.

## Fields

`fields` picks which numbers to show and in what order; without it, every number the source
has. Names: `battery`, `battery_pct`, `used`, `current`, `lq`, `rssi`, `snr`, `power`,
`throttle`, `mode`, `timer`, `goggles_battery`, `signal`, `delay`, `bitrate`, `distance`.

The overlay is at most three lines: battery, then link, then throttle, mode and timer. A
multi-cell battery shows its per-cell voltage too (`15.8V 3.95/c`); the cell count comes from
the highest voltage in the log.

## Use it

```bash
quadcam-cli library overlay <id>                      # subtitle file from the radio log or sidecar
quadcam-cli library overlay <id> --burn               # also the burned-in copy
quadcam-cli library overlay <id> --source srt --srt /path/to/DJIG0042.srt --fields battery,delay,bitrate
quadcam-cli library overlay <id> --logs /path/to/LOGS --out /path/to/folder
quadcam-cli library overlay <id> --overwrite              # replace this clip's overlay files
```

MCP: `quadcam_library_files` with `action: "overlay"`, one id in `ids`, and `burn`,
`overlay_source`, `srt`, `fields`, `log_dir`, `out`, `overwrite` as above. A burn re-encodes and
is slow, so through MCP it needs `confirm: true` too. Without it the call writes nothing and
says what would run.

The app has no control for the overlay yet: it runs from the command line and MCP.

The burned-in copy needs an ffmpeg with the `subtitles` filter (libass). Homebrew's ffmpeg has
it. When the ffmpeg in use lacks it, QuadCam still writes the subtitle file and says why it
made no copy.

## Design notes

- **Why subtitles and not drawn text.** A SubRip file is the one format every player and
  editor reads, it costs no re-encode, and the same file drives the burn-in through libass. A
  `drawtext` chain per sample would need one filter per cue.
- **Where the rows come from.** The library keeps a clip's flight numbers, not the log's rows.
  The overlay matches the clip to the logs again, read-only (`library_match_logs` without
  `apply`), and takes the rows that match claimed. An analog clip placed by its picture keeps
  its log offset, so the numbers line up with the picture as the moments do.
- **What is left out.** Drawing a `.osd` sidecar (Walksnail, WTFOS) as Betaflight glyphs needs
  the font tiles, and neither source QuadCam imports today writes one. A later step can read
  the `.osd` frames (`BTFL` or `MSPOSD` header) and draw them with a font the person picks.
