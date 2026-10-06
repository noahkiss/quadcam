# The library

The library is a folder of video files that QuadCam keeps in order. QuadCam opens on it: every clip you imported, newest first, grouped by flying day.

## Where the files go

The library folder is the output folder setting. The default is `~/Movies/quadcam`. QuadCam makes that folder on the first import. It never makes a folder that you picked.

Settings > Library sets how QuadCam files the clips:

| Layout | Where a clip goes |
|---|---|
| Year / Day (default) | `2026/2026-09-27/2026-09-27_<name>.mp4` |
| Day only | `2026-09-27/2026-09-27_<name>.mp4` |
| Flat | `2026-09-27_<name>.mp4` |

- **Add the place to day folders** names the day folder after the clip's saved place: `2026-09-27 Home field/`.
- **Keep originals** copies the source file (the DVR's AVI or the DJI MP4) into `originals/` in the day folder, named like the clip. A DJI clip's `.SRT` file goes next to its original, with the same name. It moves with the original on a rename, a new date or a move to the Trash.
- Cuts go next to their clip as `_cut1`, `_cut2`. See [Moments and cuts](moments-and-cuts.md).

## File names

A file name is `<date>_<name>.mp4`. You give each clip a short name and an optional note on import.

A clip without a name gets one that is unique in its day:

- If a radio log dated the clip, the name is the aircraft and the time (`Whoop 14:03`).
- Otherwise, the name is `flight-1`, `flight-2`, and so on. **Default short name** in Settings > Import sets the word.

**Date in file names** sets the date format: `2026-09-25_<name>.mp4` (default) or `26.09.25_<name>.mp4`. **Rename library files to this format** renames the clips already in the library, with their cuts and originals. Folder names keep `YYYY-MM-DD`.

QuadCam reads both formats. It takes a clip's date from the file's metadata first, then from the name.

## Dates and times

Analog DVRs have no clock, so their files carry no useful date. DJI units have one: the file name holds the unit's local time when the clip started. On import, each clip gets a date from the best source available:

- **Radio log:** if you point QuadCam at EdgeTX logs, it matches armed flight segments to clips.
- **Clip clock:** a DJI clip's own clock.
- **Import date:** today, when nothing else dates the clip.
- **Edited:** any date you type.

A radio log dates a DJI clip only when the log starts within 5 minutes of the clip clock. The log then also gives the clip its moments, flight numbers and EdgeTX model. A log further away leaves the clip clock in place, and the Review step shows a warning with the gap. **Clip clock skew (s)** under **Log matching** in Settings > Import sets the window, in seconds (default 300). In the settings file it is `clock_skew_s` in `tunables`.

A clip clock before 2015 or after tomorrow means the unit's clock reset. QuadCam ignores it, shows a warning, and dates the clip as it dates an analog clip.

A clip's time of day comes from its radio log or its clip clock, or you type it. Without one, the creation date holds noon, and QuadCam shows no time.

## The files are the library

QuadCam writes every detail it shows into the file's QuickTime metadata: the date, name, note, place, aircraft, moments, keep ranges, flight numbers, star rating, pick or reject flag, and whether the clip is in Photos. See [Metadata](metadata.md).

The index in `<library>/.quadcam/index.json` is a cache that makes the app start fast. **Rebuild from files** in Settings > Library makes it again from the files alone. A rebuild never moves or writes a file. Only cut ranges that you set but did not save yet live in the index alone.

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

### Existing exports

If the library folder already holds videos that are not in the index, the library shows a **Scan folder** banner. These can come from an older QuadCam or from another tool. A scan reads the files where they are. It moves nothing and writes nothing. If you rate or edit such a clip later, QuadCam writes into that clip's file.

## Work in the library

- **Select** a clip. Command-click and Shift-click select more. The arrow keys move the selection, and Command-A selects all.
- **Open** a clip with a double-click, Space, or T.
- **Rate** with the keys 1 to 5. The key 0 clears the rating. The stars on a card also work.
- **Flag** with P (pick), X (reject), or U (clear).
- **Rename** with Return.
- **Undo** (Command-Z) and **Redo** (Shift-Command-Z) work for ratings, flags, renames, notes, keywords, places and Move to Trash. Undo after Move to Trash puts the files back from the Trash.
- **Search** matches the name, note, place, aircraft, keywords and file name.
- **Thumbnails** show the clip as you move the pointer across them.
- **Rejected** in the sidebar shows a **Move to Trash** button for every rejected clip.
- The day summary shows the flights, armed time, packs, lowest battery voltage and the best moments of the day.

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
| Move to Trash | Command-Delete |

Move to Trash takes the clip's cuts and kept original with it.

**Share** opens the macOS Share menu, which includes Photos. You cannot drag clips out to Finder yet. Use **Show in Finder**.

## Edit a clip

Open a clip, then use the **Details** tab. It changes the name, date, time of day, aircraft, place, keywords, author and note. The **Flight** tab shows the radio log's numbers for the clip.

- A new date moves the clip, its cuts and its kept original to that day's folder. If the file name starts with the date, QuadCam renames them too.
- A new time of day rewrites the QuickTime creation date and the file times of the clip, its cuts and its original.
- A new aircraft rewrites the camera make and model, the aircraft, the video system and the profile's keywords in the file and its cuts.

## Unfinished imports

You can close the Import sheet at any time. The import stays in the sidebar as **Unfinished import**, and the app saves it as you work.

If you quit and open QuadCam again, the import comes back with the same clips, names, dates and cuts. This works only while the copies in QuadCam's cache still exist. **Start over** in the sheet clears the import. Clips already in the library stay where they are. After the app formats a card, the next launch starts empty.
