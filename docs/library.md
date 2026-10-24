# The library

The library is a folder of video files that QuadCam keeps in order. QuadCam opens on it: every clip you imported, newest first, grouped by flying day.

The window can be as narrow as 420 px. Pages fill the window up to 1760 px and then centre. At 720 px and narrower the sidebar hides: the button at the left of the toolbar shows it over the page, and a pick or Escape hides it. The library's sort and Grid or List move into the toolbar's **…** menu, and the grid's width sets its columns.

## Where the files go

The library folder is the output folder setting. The default is `~/Movies/quadcam`. QuadCam makes that folder on the first import. It never makes a folder that you picked.

Settings > Library sets how QuadCam files the clips:

| Layout | Where a clip goes |
|---|---|
| Year / Day (default) | `2026/2026-09-27/2026-09-27_<name>.mp4` |
| Day only | `2026-09-27/2026-09-27_<name>.mp4` |
| Flat | `2026-09-27_<name>.mp4` |

- **Add the place to day folders** names the day folder after the clip's saved place: `2026-09-27 Home field/`. A new place on a library clip moves the clip, its cuts and its original to that place's folder.
- **Keep originals** copies the source file (the DVR's AVI or the DJI MP4) into `originals/` in the day folder, named like the clip. A DJI clip's `.SRT` file goes next to its original, with the same name. It moves with the original on a rename, a new date or a move to the Trash.
- Cuts go next to their clip as `_cut1`, `_cut2`. See [Moments and cuts](moments-and-cuts.md).

## File names

A file name is `<date>_<name>.mp4`. You give each clip a short name and an optional note on import.

A clip without a name gets one that is unique in its day:

- If a radio log dated the clip, the name is the aircraft and the time (`Whoop 14:03`).
- Otherwise, the name is `flight-1`, `flight-2`, and so on. **Default short name** in Settings > Import sets the word.

A name that ends like a cut's (`cut2`) gets a trailing `_` (`2026-09-27_cut2_.mp4`), so the file never reads as a cut. A file that QuadCam wrote is a cut only when its metadata says so (`app.quadcam.cut`), whatever its name.

### On-device suggestions

On a Mac with Apple's on-device model (macOS 26 or later, Apple Intelligence on), QuadCam suggests a short name, keywords and a note for each clip after a card loads. The model reads what QuadCam knows about the clip: the date and time of day, the place, the aircraft, the length, the radio-log moments and the flight. On macOS 27 it also looks at one frame from mid-flight. It runs on the Mac, offline and free; nothing leaves the Mac.

- **One outing, one name.** Clips from the same day, place and aircraft share one name with a pack number in time order: `morning loops pack 1`, `morning loops pack 2`. The numbers continue after the day's numbered clips in the library. The model picks the words; QuadCam numbers the packs, in the word your earlier names use.
- **Your values stay.** A suggestion fills only an empty name, note or keyword list, or one that still holds the model's last suggestion. A name you typed, or an agent suggested, never changes.
- **Marked on-device.** A suggested field has a dotted outline and an **on-device** badge until you edit it. When you typed a name, the model's name shows under it with **Use**.
- **Suggest names** in the Review step asks again, for example after you match radio logs.
- **On-device suggestions** in Settings > Import turns it off. On an older Mac, or with Apple Intelligence off, QuadCam skips it without a message.

The command line runs it with `quadcam-cli names [IDS]`; an agent passes `on_device: true` to `quadcam_suggest`.

**Date in file names** sets the date format: `2026-09-25_<name>.mp4` (default) or `26.09.25_<name>.mp4`. **Rename library files to this format** renames the clips already in the library, with their cuts and originals. Folder names keep `YYYY-MM-DD`.

QuadCam reads both formats. It takes a clip's date from the file's metadata first, then from the name.

## Dates and times

Analog DVRs have no clock, so their files carry no useful date. DJI units have one: the file name holds the unit's local time when the clip started. On import, each clip gets a date from the best source available:

- **Radio log:** if you point QuadCam at EdgeTX logs, it matches armed segments ("flights") to clips. "Pack" in QuadCam means a battery, never a flight. See [Radio logs](metadata.md#radio-logs) for how.
- **Clip clock:** a DJI clip's own clock.
- **Import date:** today, when nothing else dates the clip.
- **Edited:** any date you type.

A radio log dates a DJI clip only when the log starts within 5 minutes of the clip clock. A log further away leaves the clip clock in place, marks the match "likely", and the Review step shows a warning with the gap. Either way, a matched log gives the clip its moments, flight numbers and EdgeTX model. **Clip clock skew (s)** under **Log matching** in Settings > Import sets the window, in seconds (default 300). In the settings file it is `clock_skew_s` in `tunables`.

A clip clock before 2015 or after tomorrow means the unit's clock reset. QuadCam ignores it, shows a warning, and dates the clip as it dates an analog clip.

A log written while the radio's clock was reset (dated `2000-01-01`) still matches by flight lengths. It gives flight numbers and moments, but no date: the clip keeps its own.

A clip's time of day comes from its radio log or its clip clock, or you type it. Without one, the creation date holds noon, and QuadCam shows no time.

## The files are the library

QuadCam writes every detail it shows into the file's QuickTime metadata: the date, name, note, place, aircraft, moments, keep ranges, flight numbers, star rating, pick or reject flag, and whether the clip is in Photos. See [Metadata](metadata.md).

The index in `<library>/.quadcam/index.json` is a cache that makes the app start fast. **Rebuild from files** in Settings > Library makes it again from the files alone. A rebuild never moves or writes a file. Only cut ranges that you set but did not save yet live in the index alone.

The app, the command line and the MCP server take turns on the index: each holds a lock on `.quadcam/index.lock` while it reads, changes and saves it. An index that a newer QuadCam wrote is refused: update QuadCam to open that library.

### Clip identity

QuadCam knows a clip by the content of its source file, not by its name. DVRs start again at `PICT0001` after every format, so a name proves nothing. The card's "N new" count in the sidebar uses the same identity.

- A clip that QuadCam imported has the fingerprint of its source file: XXH64 over defined bytes of the AVI or MP4 file.
- A file that QuadCam did not write gets an XXH64 hash of its first MB before `moov`. Metadata edits never touch those bytes.

`src-tauri/src/identity.rs` states both hashes, and tests pin them with test vectors. The index records the scheme as `id_scheme: 2`.

QuadCam 0.4 and earlier used other ids. When QuadCam loads an older index, it moves it to the current scheme:

- A clip whose kept original proves its old id gets the current id. So does every adopted file.
- The old id stays as an alias, and every lookup accepts it.
- Other clips keep their old id.

QuadCam writes no media file during this step.

### Joined recordings

An analog DVR splits a long recording into files of a fixed length, or of a fixed size just under 4 GiB (the largest file FAT32 holds). A Fat Shark Echo writes about 600 seconds per file: `PICT0001.AVI`, `PICT0002.AVI`, and so on. The next file starts where the last one ended.

QuadCam knows one DVR's split length: the Fat Shark Echo, 600 seconds. For any other DVR it reads the split from the files themselves.

On import, QuadCam joins such files into one clip. It joins two files only when all of these are true:

- Both are analog files in the same folder, with the same name and the next number (`PICT0001.AVI`, then `PICT0002.AVI`).
- The first file is whole and full length. The next file may be any length, and may be cut off at its end. A file is full length when one of these is true:
  - It is a known DVR's split length, give or take 1 second (600 seconds for a Fat Shark Echo).
  - The file numbered just before or after it has the same length, give or take 1 second. The length must be at least 1 minute and a whole number of minutes, give or take 2 seconds. Two full files in a row show the DVR's split, whatever it is.
  - Its size is just under 4 GiB (within 128 MiB).
- Both have the same picture size, frame rate and streams.
- The file times, when the DVR keeps a clock, do not contradict it.

When QuadCam is not sure, it keeps the files separate. Examples: the first file is 596 seconds long, or a DVR QuadCam does not know wrote one 300-second file and then a shorter one.

- The Review step shows a joined clip as one row with its files. **Keep files separate** imports them as clips of their own. **Join N files** joins them again. You cannot change it after the clip is in the library.
- **Join split recordings** in Settings > Import turns joining on or off. It is on by default. The command line takes `stage --no-join` for one import, and `import --separate ID` or `import --join ID` for one clip. An agent passes `join` to `quadcam_load_clips`, or `joined` to `quadcam_suggest`.
- QuadCam converts every file into one output file and verifies it against all of them together: frame count and duration.
- The radio log is matched to the whole recording, so a flight that runs across a file boundary stays one flight. Moments and dead air use the joined timeline.
- The clip keeps the identity of every file. The output names the later files in `app.quadcam.parts`, and the card's "N new" count counts every file as imported.
- **Keep originals** keeps every file: the first as `<name>.avi`, the next as `<name>.part2.avi`, and so on. They move and rename with the clip.
- **Delete clips after import** deletes the files of a joined clip only when the joined output verifies again.

Clips imported as separate files before QuadCam 0.6.3 stay separate. QuadCam does not join clips already in the library.

### Existing exports

If the library folder already holds videos that are not in the index, the library shows a **Scan folder** banner. These can come from an older QuadCam or from another tool. A scan reads the files where they are. It moves nothing and writes nothing. If you rate or edit such a clip later, QuadCam writes into that clip's file.

## Work in the library

- **Select** a clip. Command-click and Shift-click select more. The arrow keys move the selection, and Command-A selects all.
- **Open** a clip with a double-click, Space, or T.
- **Rate** with the keys 1 to 5. The key 0 clears the rating. The stars on a card also work.
- **Flag** with P (pick), X (reject), or U (clear).
- **Rename** with Return.
- **Undo** (Command-Z) and **Redo** (Shift-Command-Z) work for ratings, flags, renames, notes, keywords, places and Move to Trash. Undo after Move to Trash puts the files back from the Trash. It puts back only files that QuadCam moved to the Trash since the app started.
- **Search** matches the name, note, place, aircraft, keywords, file name and [sun tag](#sun-and-weather) (`golden hour`).
- **Thumbnails** show the clip as you move the pointer across them.
- **Rejected** in the sidebar shows a **Move to Trash** button for every rejected clip.
- Each day's head shows, on one line, the clips, armed time, radio-log flights, lowest battery voltage, the place and aircraft, the best moments of the day, and **Share**. In a narrow window it wraps: the date, then the numbers, then the moments. **Best moments** in the sidebar lists them for the whole library (see [Moments](moments-and-cuts.md#moments)).
- A thin strip at the bottom of a card's thumbnail is stronger where the clip's moments are.
- **Shift time…** in the bar for two or more selected clips fixes a wrong camera clock. See [Shift the time of many clips](#shift-the-time-of-many-clips).

Right-click a clip for these commands:

| Command | Key |
|---|---|
| Rename | Return |
| Edit details | Command-I |
| Trim and cuts | T |
| Share… | |
| Add to *album* (shown when Settings names a Photos album) | |
| Show in Finder | Command-R |
| Find dead air again | |
| Find highlights again (macOS 27; see [Highlights](moments-and-cuts.md#highlights-from-video-analysis)) | |
| Move to Trash | Command-Delete |

Move to Trash takes the clip's cuts and kept original with it. The clip's own file goes last, and only after all the others went: if one of them cannot go, the clip stays in the library with what is left.

**Share** opens the macOS Share menu, which includes Photos. You cannot drag clips out to Finder yet. Use **Show in Finder**.

## Edit a clip

Open a clip, then use the **Details** tab. It changes the name, date, time of day, aircraft, place, keywords, author and note. The **Flight** tab shows the radio log's numbers for the clip.

- A new date moves the clip, its cuts and its kept original to that day's folder. If the file name starts with the date, QuadCam renames them too.
- With **Add the place to day folders** on, a new place moves them to the folder of that place and day.
- A move that fails partway puts back the files it already moved. A rename never lets two files take one name.
- A new time of day rewrites the QuickTime creation date and the file times of the clip, its cuts and its original.
- A new aircraft rewrites the camera make and model, the aircraft, the video system and the profile's keywords in the file and its cuts.

## Shift the time of many clips

A DVR or goggles clock that was wrong puts every clip of a day at the wrong time. Fix one clip and shift the rest by the same amount:

1. Select the clips. Command-click or Shift-click selects more.
2. Select **Shift time…** in the bar at the bottom.
3. Pick the **Clip** whose real time you know. Set its **Real date** and **Real time**.
4. Check the table. It shows each clip's time now, its new time and where the new time comes from.
5. Select **Shift N clips**.

| From | New time |
|---|---|
| Set | The real date and time you gave |
| Offset | The clip's own time plus the same offset as the clip you set |
| Radio log | The start of the radio log that matches the clip |
| No time | None: the clip has no time of day to shift, and no log matched it |

QuadCam looks for radio logs in the log folder setting. It matches each clip against the logs of the day the offset moves it to, as [Match radio logs](metadata.md#match-clips-already-in-the-library) does. A clip that matches gets the log's time instead of the offset, and the table says **Radio log**. A new day moves the files, as a date edit does.

The CLI runs it as `library shift`, and an agent as `quadcam_library_edit` with `shift`. Both only show the plan until you confirm.

## Sun and weather

A clip with a place and a time of day gets a **sun tag**: where the sun stood when you flew. QuadCam works it out on your Mac from the place, the date and the time; nothing leaves the Mac. A clip gets at most one tag:

| Tag | When |
|---|---|
| Sunrise, Sunset | Within 15 minutes of the sun crossing the horizon |
| Golden hour | The sun up, at most 6 degrees high |
| Dawn, Dusk | The sun below the horizon, at most 6 degrees (civil twilight) |

Daylight and night have no tag. The grid shows the tag as a sun icon, the list in its **Sun** column, and an open clip under **Details**. Search finds it (`golden hour`, `sunset`). The time is your Mac's time zone, as the library shows it.

With [Look up weather](settings.md#look-up-weather) on, an open clip also shows the **Weather** at its place and hour, from Open-Meteo:

| Row | Shows |
|---|---|
| Weather | Temperature, wind speed and direction, and gusts |
| Sky | An icon and the sky: clear, partly cloudy, cloudy, fog, drizzle, rain, heavy rain, sleet, snow, thunderstorm or hail. A clear or partly cloudy night shows the moon |
| Cloud cover | Percent of the sky |
| Precipitation | Rain and snow of that hour, in mm. Shown only above 0 |
| Humidity | Relative humidity, percent |
| Visibility | In km. Only Open-Meteo's historical forecast has it, so a clip from its archive shows none |

Temperatures show in °F, speeds in mph and visibility in mi where your region uses them. QuadCam asks once per clip and saves the answer in the file (`app.quadcam.weather`). A clip looked up before QuadCam saved the sky has only the Weather row; QuadCam asks once more for it. A new place, date or time removes the weather, and QuadCam asks again for the new hour. Open-Meteo's archive answers up to the previous day; for a newer clip QuadCam asks its historical forecast, and a clip from the last hours waits until the next launch.

## The map

With [Show the Map page](settings.md#show-the-map-page) on, **Map** in the sidebar shows your places and clips on an Apple map.

Click a pin to show its callout:

| Pin | Stands for | Its callout shows | Button |
|---|---|---|---|
| Red, with a number | A place and its clip count | The place, its clip count, and the first and last day of its clips | **Show clips**: the place's clips in the library |
| Gray | A saved place without clips | The place and **No clips** | None |
| Blue | A clip with a location but no place | The clip's name, day and time, its poster frame, and the sky at flight time when [weather](#sun-and-weather) has it | **Open**: the clip |

- Return does what the callout's button does. Escape, or a click elsewhere on the map, closes the callout.
- VoiceOver reads each pin's name, kind and callout lines.
- A place sits at its saved point. A place that is not saved sits in the middle of its clips' locations.
- Pins close together merge into one at a wide zoom. Click it to zoom in.
- **Map**, **Hybrid** and **Satellite** pick the look, and the map opens in it the next time ([Map style](settings.md#map-style)). **Show all** zooms to every pin.
- The header counts the places, the clips on the map, and the clips without a location.
- The **Places** list beside the map shows each saved place with its clip count and the day you last flew there; click one to show its clips. **Add place…** searches for a place or uses this Mac's location, and saves it. A place's **…** menu holds **Rename…**, **Show clips** and **Delete…**. In a narrow window the map sits on top and the list under it.
- Scroll or pinch to zoom; drag to move. The map keeps its view when you come back to the page.

## Unfinished imports

You can close the Import sheet at any time. The import stays in the sidebar as **Unfinished import**, and the app saves it as you work.

If you quit and open QuadCam again, the import comes back with the same clips, names, dates and cuts. This works only while the copies in QuadCam's cache still exist. **Start over** in the sheet clears the import. Clips already in the library stay where they are. After the app formats a card, the next launch starts empty.
