# Metadata, places and profiles

Every file QuadCam writes carries QuickTime metadata that Apple Photos and exiftool read. QuadCam fills it from the clip, the aircraft profile, the place and the radio log.

## Set metadata on import

In the Import sheet, select a clip. The panel on the right shows its preview, its timeline and three tabs: **Details**, **Flight** and **File**. Use **Details**:

- **Aircraft**: the profile of the gear the clip was flown with. **Automatic** picks the profile whose EdgeTX model names include the model of the clip's radio log. If none matches, it picks the first profile whose **Video system** names the clip's source (`Analog` or `DJI`). If none matches either, it picks the default profile.
- **Place**: type a saved place (the list filters as you type), or a latitude and longitude such as `40.6892, -74.0445`. To keep a typed location for next time, name it and select **Save as place**.
- **Keywords** and **Author**: QuadCam offers recent values as you type. Notes in the clip list work the same way.
- **Apply to all clips** copies the clip's aircraft, place, keywords and author to every clip.

The row above the clip list sets the aircraft, the place and the date for every clip at once.

In the library, open a clip and use its **Details** tab. See [The library](library.md#edit-a-clip).

## What QuadCam writes

| What | QuickTime key | Where the value comes from |
|---|---|---|
| Location | `com.apple.quicktime.location.ISO6709`, and `©xyz` | The clip, else the profile's default place |
| Creation date | `com.apple.quicktime.creationdate` | The clip's date and time of day (from the radio log, the clip clock, or set by hand; noon otherwise), with your time zone's UTC offset, so Photos shows the local time |
| Camera make and model | `com.apple.quicktime.make`, `.model` | The profile (your goggles or DVR) |
| Software | `com.apple.quicktime.software` | `QuadCam <version>` |
| Title, description, comment | `com.apple.quicktime.title`, `.description`, `.comment` | The short name, the source file and date source (`DVR PICT0001.AVI; date source: radio log`, `DJI DJI_20261004183012_0001_D.MP4; date source: clip clock`), the note |
| Author | `com.apple.quicktime.author` | The clip, else the profile |
| Keywords | `com.apple.quicktime.keywords` | `FPV`, the profile's, the clip's, and the moment kinds found (roll, flip, punch-out, dive) |
| Aircraft, video system | `app.quadcam.aircraft`, `app.quadcam.video_system` | The profile |
| Flight numbers | `app.quadcam.flight`, `app.quadcam.stats` | The matched radio log: armed time, flights and each flight's time in the clip, lowest receiver voltage, link quality and RSSI, highest throttle. QuadCam 0.6.4 and earlier wrote `packs` and `pack_spans` here; QuadCam still reads them |
| Library | `app.quadcam.source`, `.dvr`, `.parts`, `.import`, `.place`, `.profile`, `.moments`, `.keep`, `.cut` | The source file's content fingerprint and file name, the later files of a [joined recording](library.md#joined-recordings), the import, the place and aircraft names, the radio-log moments, the keep ranges, a cut's range |
| Rating, flag, Photos | `app.quadcam.rating`, `.flag`, `.photos` | Set in the library |
| Time source | `app.quadcam.time` | `log`, `clip` (the DJI clip clock) or `manual` when the clip has a time of day. Without one, QuadCam shows no time (the creation date holds noon) |

QuadCam reads every key back before a file counts as verified. It reads them with ffprobe, and the location also with exiftool when exiftool is installed.

ffmpeg cannot write these keys where Apple's frameworks find them. QuadCam adds them to the file itself after ffmpeg finishes.

A DJI clip as MP4 is a byte copy of the original with these keys added. It keeps every stream: the video, the DJI data streams that Gyroflow reads, and the cover picture. As MOV, ffmpeg copies the streams into a new container. The data streams stay, but the cover picture does not. QuadCam never re-encodes a full DJI clip. One side effect: MP4 files no longer have the index at the front (`+faststart`). That matters only for streaming from a web server. Local players and Photos do not care.

### What Photos reads

AVFoundation is the framework that Photos uses to read video. It returns these values from QuadCam's files: location, creation date, make, model, software, title, description, author and keywords.

Nobody checked whether Photos shows each one in its Info panel. Keywords in particular can be missing there. Photos does not read QuadCam's star rating, because Photos keeps ratings in its own library, not in the file.

## Places and profiles

Settings has editors for places (Settings > Places) and aircraft profiles (Settings > Aircraft). QuadCam ships with none. QuadCam saves them in the settings file. See [Settings](settings.md).

### Find a place

1. In Settings > Places, type an address or the name of a place, such as a park or a landmark.
2. Press Return or select **Search**.
3. Pick a result. QuadCam fills in the name and the coordinates.

The search button on a row fills that row. You can also type coordinates by hand.

QuadCam searches only when you ask, never per keystroke.

### Place search providers

**Search with** picks the provider:

| Provider | Notes |
|---|---|
| **Apple Maps** (default) | MapKit's search. Free, no account. |
| **OpenStreetMap (Nominatim)** | The public Nominatim server. Free. QuadCam sends at most one request a second, as its usage policy requires. |
| **US Census (US street addresses)** | The US Census Bureau geocoder. Free, no key, US street addresses only. It finds rural addresses that the others can miss. When Apple Maps or OpenStreetMap finds nothing, QuadCam asks it too. |
| **Google Places (API key)** | Google Places API (New) text search. Off unless you pick it. |

Google Places needs an API key from a Google Cloud project with the Places API (New) and billing turned on. Light use stays in the free tier.

- Type the key in **Google Places API key**, or set `QUADCAM_GOOGLE_PLACES_KEY` in the environment. The environment variable wins.
- QuadCam never shows the key again. It sends the key to Google in a request header.

The search sends what you type to the provider you picked, and to the US Census Bureau when the fallback runs. Nothing else leaves your Mac.

### A profile in the settings file

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

`edgetx_models` holds the model names as they start the radio's log files. `AIR65-2026-10-04-101500.csv` is model `AIR65`. In Settings > Aircraft, this is **Radio model names**.

The model names tie a radio model to an aircraft and its video system. A log of a listed model matches only the clips that profile fits: clips set to that profile, or clips from its **Video system** (`Analog` or `DJI`). So with `AIR65` on an analog profile and `METEOR75` on a DJI profile, a DJI clip never takes the `AIR65` log. A log whose model no profile lists matches any clip, by shape alone, and its match says so.

## Radio logs

QuadCam reads EdgeTX "SD Logs" CSV files, with Date and Time in the first two columns. In the Import sheet, under **Radio logs**, select **Choose…** and pick your radio's `LOGS` folder. You can also pick the radio itself in USB storage mode.

A matched log gives the clip its date, time of day, flight numbers and moments. See [Moments and cuts](moments-and-cuts.md).

### How a log matches

QuadCam matches by shape first, then checks clocks:

- **Flight lengths.** A clip claims the flights (armed segments) whose span fits inside its length, plus 30 s of slack. A gap of more than 5 s ends a flight.
- **Order.** Clips claim flights in recording order, and two clips never share a flight. Gaps between DJI clip clocks must agree with gaps between their flights.
- **Clock.** The clip clock (DJI) breaks ties: the flight nearest it wins. It never rules a match out.
- **Model.** The log's EdgeTX model must fit the clip's profile (see above).

An analog clip with dead air matches by its picture instead of its length. On a whoop where the battery powers the camera and the VTX, the DVR keeps recording across battery swaps. So each stretch of picture between dead air is one battery. QuadCam slides the log along the clip and scores each place:

- **Flights sit inside a picture stretch.** Unarmed picture around a flight is fine (the battery goes in, the quad sits on the ground). A flight in dead air, or across it, costs a lot. Flight edges get 4 s of slack.
- **Short dead air while armed is signal breakup.** Dead air of 10 s or less inside a flight, with picture on both sides, is the video breaking up (range, a crash), not a battery swap. It costs little, and the picture on both sides counts as one battery.
- **Several flights may share a stretch.** A disarm and a re-arm on one battery (after a crash, say) gives two flights in one stretch.
- **A battery swap is dead air.** A flight that starts well above the last flight's end voltage (`RxBt`, 0.3 V a cell or more) is on a new battery, and dead air must lie between the two flights. A flight within 0.15 V a cell of the last one's end, with dead air between, is the same battery unplugged and plugged in again. QuadCam estimates the cell count from the highest battery reading at arm: that voltage over 4.5 V, rounded up.
- **A stretch without a flight is suspicious.** Up to 30 s costs nothing; a longer one costs more the longer it is.
- **A recording may start late or stop early.** A flight may run past either end of the clip, when the picture runs to that end.
- **Missed recordings are normal.** A flight with no clip costs nothing. But every flight of the session that would fall inside the clip must be part of the match.

The match also places the log in the clip: the clip's log offset is set to where the first flight starts, so moments, flight numbers and **Split by flight** line up with the picture. With a believable radio clock, the clip's time of day is its start, not the arm.

Files an analog DVR split from one recording are matched as one timeline, so a flight may run across the split. In an import they are one clip already (see [Joined recordings](library.md#joined-recordings)). In the library, two clips match as one timeline when the second file has the next DVR number, the first is full length (the same rule as [Joined recordings](library.md#joined-recordings), without the file size), and both have the same date. Each file then gets the flights it shows.

Each match gets a badge and a reason, shown on the Review step's date chip:

| Badge | Meaning |
|---|---|
| matched | The flights fill the clip, with at most 60 s of unarmed time, and no other fit is nearly as good. By picture: no flight in dead air, no battery swap without dead air, little empty picture, and no other fit nearly as good |
| likely | A fit, but with much unarmed time, a long picture stretch without a flight, a flight in dead air, another flight that fits as well, or a clip clock more than 5 minutes off |
| unmatched | No flight fits. QuadCam leaves a clip unmatched rather than guess |

The reason reads like `1 flight from 18:36:34 (113 s armed over 113 s) in a 113 s clip; clip clock 2 s off; log METEOR75 → profile Meteor75`. A match by picture adds how the flights fit: `flights fit 3 of 4 picture stretches; 2 battery swaps at dead air; first flight at 54 s (±7 s)`. The ± is how far the log could slide and fit as well.

### A radio clock that reset

When the radio's clock battery is flat, EdgeTX dates every log `2000-01-01`, and the time starts again at midnight on each power-on. QuadCam keeps such a log's rows in file order and treats each power-on as its own session. Clips still match by flight lengths or picture, and order. The clips keep their own date, and the Review step warns that the radio clock was wrong. When no log has a believable date, QuadCam picks the reset-clock day by itself.

### Match clips already in the library

Settings > Library > **Match radio logs** matches the log folder to every library clip. It shows what matched first. **Write** puts the flight numbers and moments of the matched clips into their files, placed at the log offset the picture gave. It never changes a date, a time or a name.

Each clip is matched to the logs of its own day. Clips of a day without a log are matched to a reset-clock log, if there is one. Clips of different days never share one power-on of the radio.

The CLI and MCP server do the same: `quadcam-cli library match-logs [ids] [--logs DIR] [--day DAY] [--apply]`, and `quadcam_library_files` with action `match_logs`. Without `--apply`, nothing changes. With it, "likely" clips are written only when you name them by id.
