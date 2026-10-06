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
| Flight numbers | `app.quadcam.flight`, `app.quadcam.stats` | The matched radio log: armed time, packs, lowest receiver voltage, link quality and RSSI, highest throttle |
| Library | `app.quadcam.source`, `.dvr`, `.import`, `.place`, `.profile`, `.moments`, `.keep`, `.cut` | The source file's content fingerprint and file name, the import, the place and aircraft names, the radio-log moments, the keep ranges, a cut's range |
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

`edgetx_models` holds the model names as they start the radio's log files. `AIR65-2026-10-04-101500.csv` is model `AIR65`.

## Radio logs

QuadCam reads EdgeTX "SD Logs" CSV files, with Date and Time in the first two columns. In the Import sheet, under **Radio logs**, select **Choose…** and pick your radio's `LOGS` folder. You can also pick the radio itself in USB storage mode.

A matched log gives the clip its date, time of day, flight numbers and moments. See [Moments and cuts](moments-and-cuts.md).
