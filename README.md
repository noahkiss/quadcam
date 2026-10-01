# quadcam

quadcam turns the SD card from analog FPV goggles into dated, named, playable video files, and keeps them in a library. It copies the clips off the card, gives each clip a date and a name, and converts it to MP4. It verifies every file, files it by flying day, and then it can clear the card for the next flight.

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
7. **Trim (optional).** quadcam marks moments on each clip's timeline: rolls, flips, punch-outs and dives from the radio log's sticks, and dead air from the video itself. You can export any range as an extra file. See [Moments and cuts](#moments-and-cuts).
8. **Verify.** quadcam compares every output with its source: frame count, duration, streams, and metadata. Only verified files count as imported.
9. **Share.** You can add the files to the Photos app, into an album.
10. **Format (optional).** When every clip verified, you can erase the card as FAT32 and eject it. See [Format safety](#format-safety).
11. **Library.** The files land in the library folder, a folder per year and per flying day. You can rate, flag, search, rename and trim them there later. See [The library](#the-library).

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

quadcam opens on the library: every clip you imported, newest first, grouped by flying day.

1. Insert the card. It shows in the sidebar under **Import from**, with the number of clips that are not in the library yet. Nothing loads until you select it.
2. Select the card (or **Import…**, or **Folder…**). The Import sheet opens and copies the clips at once. The sheet header shows how many files are left and the progress of the current one.
3. **Review**: set the aircraft, the place and the date for every clip at once, or per clip. Optional: under **Radio logs**, select your radio's `LOGS` folder, or the radio itself in USB storage mode. Type a short name and a note for each clip, and a time of day when you know it (empty means noon, unless a radio log dated the clip). Select **Skip** for clips you do not want, such as bench tests. Select a clip to play it, see its moments and set cuts.
4. **Export** converts and verifies each clip.
5. **Finish**: add the files to Photos, eject the card, or format it (see below). **Done** shows the new clips in the library under **Last import**.

You can close the sheet at any time; the import stays in the sidebar as **Unfinished import**. The app saves it as you work. If you quit and open quadcam again, it comes back with the same clips, names, dates and cuts, as long as the copies in its cache still exist. **Start over** in the sheet clears it. Files you already exported stay where they are. After the app formats a card, the next launch starts empty.

## The library

The library folder is the output folder setting (default `~/Movies/quadcam`). Settings > Library sets how files are filed in it:

| Layout | Where a clip goes |
|---|---|
| Year / Day (default) | `2026/2026-09-27/2026-09-27_<name>.mp4` |
| Day only | `2026-09-27/2026-09-27_<name>.mp4` |
| Flat | `2026-09-27_<name>.mp4` |

- **Date in file names**: `2026-09-25_<name>.mp4` (default) or `26.09.25_<name>.mp4`. **Rename library files to this format** renames the clips already in the library, with their cuts and originals; folders stay as they are. quadcam reads both formats, and takes a clip's date from its metadata first.
- **Add the place to day folders** names the day folder after the clip's saved place: `2026-09-27 Home field/`.
- **Keep originals** copies the DVR file into `originals/` in the day folder, named like the clip.
- Cuts go next to their clip as `_cut1`, `_cut2`.

The files are the library. Every detail quadcam shows is written into the file's QuickTime metadata (see [Metadata](#metadata)): the date, name, note, place, aircraft, moments, keep ranges, flight numbers, star rating, pick or reject flag, and whether it is in Photos. The index in `<library>/.quadcam/index.json` is a cache that makes the app start fast. **Rebuild from files** in Settings > Library makes it again from the files alone. Only cut ranges you set but did not save yet live in the index alone.

A clip is known by the content of its DVR file, not by its name, because DVRs start again at `PICT0001` after every format. That is also how the card's "N new" count works.

**Existing exports.** If the library folder already holds videos that are not in the index (from an older quadcam, or another tool), the library shows a **Scan folder** banner. Scanning reads them where they are. It moves nothing and writes nothing. Rating or editing a clip later writes into that clip's file.

In the library:

- **Select** a clip; Command-click and Shift-click select more. Double-click, Space or T opens the clip.
- **Rate**: keys 1 to 5 (0 clears). **Flag**: P picks, X rejects, U clears. The stars on a card also work.
- **Search** matches the name, note, place, aircraft, keywords and file name.
- **Thumbnails** show the clip as you move the pointer across them.
- **Right-click** a clip: Rename (Return), Edit details (Command-I), Trim and cuts, Add to Photos, Show in Finder (Command-R), Find dead air again, Move to Trash (Command-Delete). Move to Trash takes the clip's cuts and kept original with it.
- **Rejected** shows a **Move to Trash** button for every rejected clip.
- **Details** in an open clip changes its date, time of day, aircraft, place, keywords, author and note. A new date moves the clip, its cuts and its kept original to that day's folder and renames them when the file name starts with the date. A new aircraft rewrites the camera make and model, the aircraft, the video system and the profile's keywords in the file and its cuts.
- The day summary shows the flights, armed time, packs, lowest battery voltage and the best moments of the day.

Dragging clips out to Finder and the Share menu are not built yet; use **Show in Finder**.

## Moments and cuts

The same trim editor appears in the library (open a clip) and in the Import sheet (select a clip). The timeline shows:

- **Dead air** (striped): the receiver's blue no-signal screen, static, a colour-bar test pattern, black, or a colourless breakup (the torn grey picture an analog receiver shows when it loses the colour signal). Each stretch is at least 3 seconds. A shorter breakup inside a flight stays in. A black-and-white camera would read as colourless breakup; set `MONO_MAX_SAT` in `moments::tune` to 0 for one.
- **Keep ranges** (green line): the clip without its dead air. Select **Use keep ranges** to turn them into cuts.
- **Moments** (markers): found in the radio log's stick channels.
- **Cuts** (blue line): the ranges that export as extra files.

| Moment | What quadcam looks for |
|---|---|
| Roll | Aileron at 80 % or more of full stick, held 0.3 to 1.5 s |
| Flip | Elevator at 80 % or more of full stick, held 0.3 to 1.5 s |
| Punch-out | Throttle from 35 % or less to 85 % or more within 0.6 s |
| Dive | Throttle at 15 % or less for 1 s or more mid-flight, then a punch-out |
| Crash? | Big stick inputs in the last 1.5 s before the log stops (disarm). Low confidence |

Each moment has a score from 0 to 1. When the receiver sends attitude telemetry and it shows the quad upside down during a roll or flip, the score goes up. Select a moment to put the in and out points around it. Play the clip, then use **Set in** and **Set out** (I and O) to adjust them, drag the blue handles, or type the time. Select **Add cut** (C). Space plays, J, K and L go back, stop and forward, and the arrow keys step one frame. A clip can have up to 20 cuts.

In the library, a new cut is written when you select **Save cuts**. quadcam cuts from the kept original when there is one, else from the clip itself.

**Removing a cut that is already a file** asks first: **Keep the file** leaves it where it is as a clip of its own, **Move to Trash** moves it to the Trash. The command line needs `--removed keep` or `--removed trash`, and an agent needs `removed_cuts`.

On import, each cut becomes its own file next to the clip: `YYYY-MM-DD_<name>_cut1.mp4`, `_cut2`, and so on. quadcam cuts from the original DVR file, not from the converted one. MJPEG has a keyframe on every frame, so each cut starts and ends on the exact frame. MP4 cuts are re-encoded like the full clip; MOV cuts copy the original frames. quadcam verifies every cut (streams, frame count, duration). Cuts that verified are not written again, so you can add cuts later and import again. **Add to Photos** adds a clip's cuts with it.

Two tips for radio-log moments:

- **Log every 0.1 s.** EdgeTX logs every 0.5 s or 1 s unless you change it. In the model's Special Functions, set the **SD Logs** function's interval to 0.1 s. At 0.5 s, quadcam still finds moments, but their times are only good to half a second, short moves can be missed, and the scores are lower.
- **Line up the log.** The log starts when you arm, but the DVR usually starts recording earlier. quadcam assumes the clip starts at arm. Play the clip to the moment you arm, then select **Arm is here**. The moments move to match.

Dead-air detection needs no radio log. quadcam samples two frames per second at 64 × 48 pixels and decodes only those frames, so a 10-minute, 1.4 GB clip takes about 4 seconds. It times each sample by the frame's own timestamp, because DVRs drop frames. The thresholds were checked on real Fat Shark Echo footage: keep ranges start and end within about a second of the picture coming and going.

## Metadata

Every file quadcam writes carries QuickTime metadata that Apple Photos and exiftool read. Select a clip and use the **Metadata** section under its preview:

- **Aircraft profile**: the gear the clip was flown with. **Automatic** picks the profile whose EdgeTX model names include the model of the clip's radio log, then the default profile.
- **Location**: type a saved place (the list filters as you type), or latitude and longitude such as `40.6892, -74.0445`. Select **Save as place** to keep a typed location for next time.
- **Keywords** and **Author**: recent values are offered as you type. Notes in the clip list work the same way.
- **Apply to all clips** copies the clip's profile, location, keywords and author to every clip.

| What | QuickTime key | Where the value comes from |
|---|---|---|
| Location | `com.apple.quicktime.location.ISO6709`, and `©xyz` | The clip, else the profile's default place |
| Creation date | `com.apple.quicktime.creationdate` | The clip's date and time of day (from the radio log, or set by hand; noon otherwise), with your time zone's UTC offset, so Photos shows the local time |
| Camera make and model | `com.apple.quicktime.make`, `.model` | The profile (your goggles or DVR) |
| Software | `com.apple.quicktime.software` | `quadcam <version>` |
| Title, description, comment | `com.apple.quicktime.title`, `.description`, `.comment` | The short name, the DVR file and date source, the note |
| Author | `com.apple.quicktime.author` | The clip, else the profile |
| Keywords | `com.apple.quicktime.keywords` | `FPV`, the profile's, the clip's, and the moment kinds found (roll, flip, punch-out, dive) |
| Aircraft, video system | `app.quadcam.aircraft`, `app.quadcam.video_system` | The profile |
| Flight numbers | `app.quadcam.flight`, `app.quadcam.stats` | The matched radio log: armed time, packs, lowest receiver voltage, link quality and RSSI, highest throttle |
| Library | `app.quadcam.source`, `.dvr`, `.import`, `.place`, `.profile`, `.moments`, `.keep`, `.cut` | The DVR content fingerprint and file name, the import, the place and aircraft names, the radio-log moments, the keep ranges, a cut's range |
| Rating, flag, Photos | `app.quadcam.rating`, `.flag`, `.photos` | Set in the library |
| Time source | `app.quadcam.time` | `log` or `manual` when the clip has a time of day. Without one, quadcam shows no time (the creation date holds noon) |

quadcam reads every key back before a file counts as verified: with ffprobe, and the location also with exiftool when it is installed. ffmpeg cannot write these keys where Apple's frameworks find them, so quadcam adds them to the file itself after ffmpeg finishes. One side effect: MP4 files no longer have the index at the front (`+faststart`). That matters only for streaming from a web server; local players and Photos do not care.

What was checked: AVFoundation, the framework Photos uses to read video, returns the location, creation date, make, model, software, title, description, author and keywords from these files. Whether Photos shows each one in its Info panel was not checked; keywords in particular may not appear. Photos does not read quadcam's star rating: Photos keeps ratings in its own library, not in the file.

### Places and profiles

Settings (the gear icon) has editors for places and aircraft profiles. They are saved in the app's settings file, `~/Library/Application Support/app.quadcam/settings.json`, which the command-line tool and the MCP server read and write too. quadcam ships with none.

Every writer changes only the settings it was asked to change and keeps the rest, including keys a newer or older quadcam wrote. A change from the command line or an agent shows in the running app at once, and the app never writes an older copy over it. Settings from quadcam 0.3.0 carry over as they are.

**Find a place.** In Settings > Places, type an address or the name of a place (a park, a landmark) and press Return or **Search**. Pick a result to fill in the name and the coordinates. The search button on a row fills that row. Typing coordinates by hand still works. **Search with** picks the provider:

- **Apple Maps** (default): MapKit's search, free and without an account.
- **OpenStreetMap (Nominatim)**: the public Nominatim server, also free. quadcam searches only when you ask, at most once a second, as its usage policy requires.
- **US Census**: the US Census Bureau geocoder, free and without a key, for US street addresses only. It finds rural addresses the others can miss. When Apple Maps or OpenStreetMap finds nothing, quadcam asks it too.
- **Google Places**: Google Places API (New) text search. Off unless you pick it. It needs an API key from a Google Cloud project with the Places API (New) and billing turned on (light use stays in the free tier). Type the key in **Google Places API key**, or set `QUADCAM_GOOGLE_PLACES_KEY` in the environment, which wins. quadcam never shows the key again, and passes it to Google in a request header.

The search sends what you type to the provider you picked (and to the US Census Bureau when the fallback runs). Nothing else leaves your Mac.

A profile looks like this in the file:

```json
{
  "profiles": [
    {
      "name": "Whoop",
      "aircraft": "65 mm whoop",
      "camera_make": "Fat Shark",
      "camera_model": "Echo",
      "video_system": "Analog",
      "keywords": ["tinywhoop"],
      "author": "Your Name",
      "place": "Home field",
      "edgetx_models": ["AIR65"]
    }
  ],
  "places": [{ "name": "Home field", "lat": 40.6892, "lon": -74.0445 }],
  "defaultProfile": "Whoop"
}
```

`edgetx_models` holds the model names as they start the radio's log files: `AIR65-2026-10-04-101500.csv` is model `AIR65`.

## Settings

Settings (the gear icon) has these sections:

- **Library**: the library folder (default `~/Movies/quadcam`, created on the first import), the layout, place folders, keep originals, and **Rebuild from files**. **Archive** is not built yet.
- **Aircraft** and **Places**: the profiles and places above, and the place search provider.
- **Import**: the format, the MP4 encoder, the default short name, the time in file names, and the tolerances for log matching.
- **Photos**: the album.
- **Advanced**: where ffmpeg and the agent socket are.

## Command line

`quadcam-cli` works on the same core as the app. Each run continues one session, which is saved to `~/Library/Caches/app.quadcam/session.json`. The app uses the same file.

```bash
quadcam-cli cards                         # detected cards and radio log sources
quadcam-cli scan /Volumes/NO\ NAME        # list clips, copy nothing
quadcam-cli stage /Volumes/NO\ NAME       # copy clips to staging, start a session
quadcam-cli analyze                       # probe, recover, make thumbnails
quadcam-cli dates --logs /path/to/LOGS    # date clips from radio logs
quadcam-cli dates --set 2=2026-10-03      # set one clip's date
quadcam-cli import --name 0=backyard-loops --skip 3 --format mp4
quadcam-cli import --plan plan.json       # names, dates, notes and options from a file
quadcam-cli moments                       # moments, dead air, keep ranges and cuts per clip
quadcam-cli cut 0 12.5-18 1:02-1:10       # set clip 0's cuts (seconds or m:ss)
quadcam-cli cut 0 --keep                  # cut clip 0 down to its keep ranges
quadcam-cli cut 0 --log-offset 4.5        # the radio log starts 4.5 s into clip 0
quadcam-cli import --cut 0=20-26          # add a cut, then import
quadcam-cli import --time 0=18:30         # clip 0 was flown at 18:30 (the default is noon)
quadcam-cli meta all --place "Home field" --keywords park,windy
quadcam-cli meta 2 --location 40.6892,-74.0445 --profile Whoop
quadcam-cli cut 0 --clear --removed trash # drop exported cuts and move their files to the Trash
quadcam-cli clear                         # forget the session (start over)
quadcam-cli verify                        # check the outputs again
quadcam-cli photos --album Drone          # add verified outputs to Photos
quadcam-cli eject
quadcam-cli format --plan                 # show what would be erased
quadcam-cli format --device /dev/diskN --volume-uuid <uuid> --yes
```

The library commands work on the library folder:

```bash
quadcam-cli library list                          # every clip, newest first
quadcam-cli library list --query flips --group picks
quadcam-cli library list --day 2026-09-27         # groups: last_import, moments, picks, rejected, not_in_photos
quadcam-cli library rate <id> --stars 4 --pick    # also --reject, --unflag; --stars 0 clears
quadcam-cli library rebuild                       # make the index again from the files
quadcam-cli library rename <id> "fence flips"
quadcam-cli library edit <id> --note "windy" --keywords park,windy --place "Home field"
quadcam-cli library edit <id> --date 2026-09-28 --time 18:30   # moves the files to that day
quadcam-cli library edit <id> --profile Whoop     # rewrites make, model, aircraft and keywords
quadcam-cli library cut <id> 12-18 1:02-1:10 --export   # set the cuts and write the new ones
quadcam-cli library apply-name-format             # rename clips to the name_date_format setting
quadcam-cli library trash <id>                    # clip, cuts and original to the Trash
quadcam-cli library photos <id> --album Drone
```

Clip ids come from `library list`.

Places, aircraft profiles and settings live in the settings file the app uses:

```bash
quadcam-cli places                                # saved places
quadcam-cli places search "Golden Gate Park"      # name, address, latitude, longitude
quadcam-cli places search "Golden Gate Park" --provider nominatim
quadcam-cli places save "Home field" --search "Golden Gate Park" --pick 1
quadcam-cli places save "Home field" --location 37.7694,-122.4862
quadcam-cli places save "Home field" --rename "Park"   # profiles that use it follow
quadcam-cli places delete "Park"
quadcam-cli profiles                              # profiles and the default
quadcam-cli profiles save Whoop --aircraft "65 mm whoop" --camera-make "Fat Shark" \
  --camera-model Echo --keywords tinywhoop --place "Home field" --models AIR65 --default
quadcam-cli profiles save Whoop --author "Your Name"    # changes only that field
quadcam-cli profiles default Whoop
quadcam-cli profiles delete Whoop
quadcam-cli settings                              # the file, its values, the effective settings
quadcam-cli settings set layout=day place_folders=true photos_album=Drone
quadcam-cli settings set output_dir=null          # back to the default
```

`settings set` takes `output_dir`, `format`, `encoder`, `keep_originals`, `add_time`, `default_name`, `photos_album`, `format_label`, `log_dir`, `layout`, `place_folders`, `tunables`, `geocoder`, `name_date_format` (`YYYY-MM-DD` or `YY.MM.DD`) and `default_profile`. A value is JSON or plain text.

A plan file looks like this:

```json
{
  "clips": [
    { "id": 0, "name": "backyard loops", "date": "2026-10-04", "time": "18:30", "note": "two packs" },
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

The command-line tool uses the app's settings as its defaults.

## MCP server

`quadcam-cli mcp` runs an MCP server on stdio. It lets a coding agent import clips with you. To add it to Claude Code:

```bash
claude mcp add quadcam -- "$(brew --prefix)/bin/quadcam-cli" mcp
```

If you built from source, give the path to `quadcam.app/Contents/MacOS/quadcam-cli` instead.

If the app is running, the server works on the app's session. You see every change in the app as it happens. If the app is not running, the server works on the session file, like the command-line tool. The server checks for the app on every call, so you can quit and open the app again without restarting the server.

| Tool | What it does |
|---|---|
| `quadcam_status` | Shows the mode (app or headless), the cards, the radios, and the session |
| `quadcam_library` | Lists and searches the clips already in the library (read-only) |
| `quadcam_library_edit` | Changes library clips: rating, flag, name, note, keywords, author, place, aircraft, date, time |
| `quadcam_library_files` | Library files: sets and writes cuts, moves clips to the Trash, adds them to Photos, rebuilds the index, renames clips to the file-name date format |
| `quadcam_places` | Lists, searches (address or place name), saves and deletes saved places |
| `quadcam_profiles` | Lists, saves and deletes aircraft profiles; sets the default |
| `quadcam_settings` | Reads and writes the app's settings |
| `quadcam_load_clips` | Stages, checks, and dates a card or a folder |
| `quadcam_read_clips` | Reads the clips, their plans, moments, keep ranges and cuts; returns thumbnails as images |
| `quadcam_match_logs` | Dates the clips from EdgeTX logs |
| `quadcam_suggest` | Suggests names, dates, times, notes, skips, cuts, the log offset, or metadata (profile, place or location, keywords, author) |
| `quadcam_export` | Converts and verifies clips and cuts; can also add to Photos |
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
- No test can reach your Photos library or your Trash: under `cargo`, "Move to Trash" moves files into a temporary folder.
- `cargo test --test import size_and_speed -- --ignored --nocapture` measures MP4 against MOV size and speed.

`test-clips/README.md` describes an optional local corpus of real clips and logs.

## Project layout

| Path | Holds |
|---|---|
| `ui/` | The app's frontend: HTML, CSS, JavaScript, bundled fonts and icons |
| `ui/trim.js` | The trim editor that the library and the Import sheet share |
| `src-tauri/src/core.rs`, `core_library.rs`, `core_settings.rs` | The core that every frontend drives; its library half; its settings, places and profiles |
| `src-tauri/src/settings.rs` | The settings file: the one reader and writer, the setting names and their checks |
| `src-tauri/src/geocode.rs` | Place search: Apple MapKit and OpenStreetMap Nominatim |
| `src-tauri/src/library.rs` | The library: layout, the index, and its rebuild from the files |
| `src-tauri/src/trim.rs` | Cut ranges and the rule for exported cuts, shared by the session and the library |
| `src-tauri/src/trash.rs` | Moving files to the Trash |
| `src-tauri/src/lib.rs` | The app's Tauri commands |
| `src-tauri/src/control.rs` | The control socket |
| `src-tauri/src/mcp.rs` | The MCP server |
| `src-tauri/src/bin/quadcam-cli.rs` | The command-line tool |
| `src-tauri/src/{scan,disk,media,logs,naming,pipeline,session,photos}.rs` | Scanning, disks, ffmpeg, radio logs, file names, the import steps, the session, Photos |
| `src-tauri/src/moments.rs` | Moments from radio-log sticks and dead air from video frames. Every threshold is in `moments::tune` |
| `src-tauri/src/metadata.rs`, `qtmeta.rs` | Places, profiles and per-clip metadata; writing QuickTime metadata into the files |
| `src-tauri/tests/` | Integration tests |

## License

MIT. See [LICENSE](LICENSE).
