# Behaviour notes

How the core behaves where the code alone does not make it obvious. For contributors.

- **Library model:** the output folder is the library. `library::Layout` files clips as
  `YYYY/YYYY-MM-DD/` (default), `YYYY-MM-DD/` or flat, optionally with the place name on the
  day folder; originals go in `originals/` inside the day folder. The files are the source of
  truth: every detail (rating, flag, moments, keep ranges, flight stats, place, source
  fingerprint, Photos state) is an `app.quadcam.*` QuickTime item in the file.
  `<library>/.quadcam/index.json` is a rebuildable cache (`library::rebuild`); only unsaved cut
  ranges live in it alone. A rebuild never moves or writes a file, which is also how an
  existing export folder is adopted.
- **Identity:** a library clip's id is its source file's content fingerprint
  (`app.quadcam.source`), never a file name. A file QuadCam did not write is known by a hash of
  its first MB before `moov` (`identity::head_id`), which metadata rewrites never touch. Both
  are specified hashes (XXH64 over defined bytes; `identity.rs` states them), with test
  vectors. Ids from 0.4 and earlier (`DefaultHasher`) stay recognizable through
  `identity::legacy`, an explicit SipHash-1-3. An index without `id_scheme: 2` is moved when it
  loads: a clip whose kept original proves its 0.4 id, and every adopted file, get the current
  id, and the 0.4 id stays in `aliases`, which every lookup accepts. Other clips keep their 0.4
  id. No media file is written. `tests/fixtures/legacy-library` is a library 0.4.1 built.
- **Cuts:** `trim.rs` is the one cut model for the session and the library. Dropping a cut that
  was already exported needs a decision (`RemovedCuts::Keep`: the file stays as its own clip,
  marked `app.quadcam.detached`; `Trash`: it goes to the Trash). Without one, `Core` answers
  `CutChange::Confirm` (GUI) or refuses the patch (CLI `--removed`, MCP `removed_cuts`).
  `cuts::write_cut` writes every cut file, session and library alike: it verifies frames and
  reads the QuickTime items back before the rename.
- **Flight, not pack:** a radio-log armed segment is a "flight" in UI text, docs, API, MCP and
  CLI names; "pack" means a battery (Gear) only. `FlightStats` reads the 0.6.4 keys `packs`
  and `pack_spans` through serde aliases.
- **Split by flight:** `FlightStats.flight_spans` holds each radio-log flight's armed range in
  clip seconds (moved by the log offset). `trim::flight_cuts` makes one cut per flight, 2 s
  each side at most half the gap, clamped to the clip, and refuses when there is nothing to
  split; the cuts are added to the clip's list (`PlanPatch.split_by_flight`, `session_split`,
  `library_split`), so no exported cut is dropped.
- **Joined recordings:** at analysis `join::groups` finds analog files the DVR split (next
  number, same folder and format, the first full length by `join::full_length`: a
  `KNOWN_SPLITS` length (Echo 600 s) +/- 1 s, the same length as the file numbered next to it
  (a whole minute, >= 60 s), or a size just under 4 GiB; file times that agree); unsure
  stays apart. Each file stays a `Clip`; the first carries `Join` (its `source()` is an
  `ffconcat` list in staging, read by ffmpeg as one input) and the later ones `part_of`.
  Parts are left out of jobs and dating; `can_format` and `delete_clip_file` judge a part by
  its first file's result and the joined probe. The output names the later files in
  `app.quadcam.parts` (`LibClip.parts`, counted by `known_sources`); kept originals add
  `<stem>.part2.avi`. Setting `join_split_recordings` (on), per run `SourceParams.join`, per
  clip `PlanPatch.joined`.
- **Previews:** after `Core::analyse`, the GUI's `Hooks::analysed` runs `Core::make_previews` on a
  thread. Session and library previews both go through `Core::proxy_once`, which makes one
  proxy at a time (a static lock), so a Play click on a clip being made waits for it and reuses
  the file. Library MP4s play directly. Session and library Photos adds both go through
  `Core::share_files`, which also marks the library's clips as in Photos.
- **Native feel (UI):** page text is not selectable and the cursor is the arrow; WebKit needs
  `-webkit-user-select`. Text a person may copy (paths, details values) gets the
  `selectable` class. The web view's context menu shows only over text fields and selected
  `selectable` text; the app's own menus call `preventDefault` first.
- **Removable cards:** `disk::is_removable` takes an external removable or ejectable disk, or
  a card in the built-in SD slot (`DiskInfo::is_slot_card`: removable media on the `Secure
  Digital` bus; macOS reports the slot as `Internal`). Internal storage (`is_internal_storage`)
  never counts. "Safe to remove" (`disk::safe_remove`, method `eject`, after a format or card
  prep) runs `diskutil unmountDisk` for a card, so the disk stays listed until it is pulled,
  and `diskutil eject` for anything else. Disk images report removable media, so the disk-image
  tests see an unmount: the image stays attached and `Image::is_mounted` is false.
- **Mount cycle:** every operation on a card mounts it if needed, works, and unmounts it when
  done; "safe to unplug" plays only after the unmount worked. One helper does it:
  `core/apply_card.rs` (`locate_volume`, `unmounted_cards`, `mount_picked`, `card_for_job`) for
  gear jobs, `core/session_card.rs` for the import session's card (`stage_device`, `import`
  unmounts it after "Delete clips after import", `format_plan`/`format`/`eject`/card prep mount
  it again). Do not add a second mount path. A DJI device over USB (an O4 air unit) follows the
  same cycle: it is unmounted (never ejected, never formatted).
  Lists and checks that only read (`gear_connected`, Pack up) never mount. The Mount button
  (10 minutes, `MOUNT_MINUTES`) stays for browsing. Tests: `tests/it/gear_apply_card.rs` (fake
  `diskutil`) and `tests/it/mount_cycle.rs` (disk images).
- **Cards** show in the sidebar with their source (Analog, DJI) and an "N new" count (content
  fingerprints not in the index); inserting a card never starts an import on its own.

- **Output folder:** defaults to `~/Movies/quadcam`, resolved from `$HOME` at runtime
  (`paths::default_output_dir`). The app creates that default, with its parents, on the
  first import. It never creates a folder the user picked. Picking a folder saves it as the
  setting.
- **Settings:** one file, `settings.json` in the support folder (camelCase keys `outputDir`,
  `format`, `formatLabel`, `photosAlbum`, `places`, `profiles`, `defaultProfile`, `geocoder`,
  and others). `settings.rs` is its only reader and writer: a write reads the file fresh,
  changes only the given keys, keeps unknown keys, and renames a temp file into place under
  an flock on `settings.json.lock`. No surface keeps a full copy and writes it back. The GUI
  (no Tauri store plugin) reads and writes through `Core` (`settings`, `settings_set`,
  `place_*`, `profile_*`); its Settings window saves only keys changed while it was open. The
  app watches the file and re-reads it (event `settings-changed`) when the CLI writes it;
  an agent's MCP writes go through the control socket when the app runs. `KEYS` names every
  setting (file key, CLI/MCP name, check). Add a new setting there with a default in
  `Defaults`; never drop a key a 0.3.0 file has (`settings::tests::settings_from_0_3_0_carry_over`).
- **Place search:** `geocode.rs`. `apple` (default): MapKit `MKLocalSearch` through
  objc2-map-kit. Its answer arrives on the main queue, so on the main thread (CLI, MCP) it
  spins the run loop; in the app it waits on a worker thread. `nominatim`: `/usr/bin/curl`
  with a quadcam User-Agent and at most one request a second across processes. `census`:
  US Census one-line address geocoder (keyless, US street addresses), also the automatic
  fallback when `apple` or `nominatim` finds nothing. `google`: Places API (New) text search,
  off unless chosen; the key comes from `QUADCAM_GOOGLE_PLACES_KEY`, else the
  `googlePlacesKey` setting, and goes to curl on stdin (never argv). Settings reads redact it
  (`settings::SECRET_KEYS`); `Defaults` never serializes it. Never put a key in code or
  tests. Search only on an explicit request, never per keystroke.
- **File-name date:** `naming::DateFormat` (`nameDateFormat`: `YYYY-MM-DD` default, or
  `YY.MM.DD`) starts new file names, renames and redates. `naming::split_date` reads either
  format; a clip's date comes from its creation date first, then the name.
  `library_apply_name_format` renames existing clips (with cuts, originals and the originals' sidecars) in place.
  Folder names keep `YYYY-MM-DD`.
- **Time of day:** a clip's plan time comes from its radio log, its clip clock (DJI), or by hand (`PlanPatch.time`,
  `HH:MM`, empty for none). Without one the creation date is local noon. A library date or
  time edit (`library_edit`) rewrites the QuickTime creation date, the `mvhd` time and the
  mtimes of the clip, its cuts, its original and the original's sidecars (a DJI `.srt`,
  found on disk by stem, `LibClip::sidecars`); a new day moves them all (`relocate`, which
  rename uses too). A library profile edit rewrites make, model, aircraft, video system,
  profile name and the profile's keywords.
- **Session restore:** the GUI's core uses the same session file as the CLI. At launch,
  `Core::forget_unrestorable` drops it unless it was analysed and every staged clip still
  exists. "Start over" (`Core::clear`, method `clear`, `quadcam-cli clear`) deletes the file;
  the GUI also clears it after it erases a card. The staged copies stay.
- **Staging folders** (`core/staging.rs`): a stage writes `.owners` (the session files that
  staged it) into `<cache>/staging/<key>`. A load that replaces a session deletes the old
  session's folder, unless the new session uses it (same card) or another owner's session file
  names it. The sweep (app launch, after each stage) deletes a folder only with `.owners`, no
  owner naming it, not the live session's, and unchanged for a day (`SWEEP_AGE`). A folder
  without `.owners` is never swept. Staging stays after an export: a second import, cut
  changes, previews and the delete from the card read it.
- **Delete clips after import:** the setting `deleteClipsAfterImport` (off by default) is the
  only consent to delete clip files from a card or folder. `Core::import` runs it after the
  export; a run can turn it off (`ImportOptions.keep_clips`, CLI `--keep-clips`, MCP
  `keep_clips`), never on. MCP `quadcam_settings` writes it off, never on. It never formats. Its guards all run in
  `core::import::delete_clip_file`, immediately before the unlink: the source's
  `CardPolicy.delete_clips_offered`, a verified result, the output verified again, the path a
  plain file inside `Session.source` that the source's `pick` takes as a clip, and the same size
  and fingerprint as the staged copy; a recovered (half-written) clip only when its original
  was kept. Sidecars and every other file stay. The result lists each
  clip (`ImportOutcome.clip_deletion`).
- **Add to Photos:** one "Add N files" button on the import's Finish step, and per clip in the library. `photos::PhotoKit` uses
  PhotoKit through `objc2-photos`: add-only access for the library, read-write when an
  album is set (default album `Drone`, created if missing; an empty setting means library
  only). Denied access returns a message that points to System Settings > Privacy &
  Security > Photos. Only outputs this session verified can be shared.

- **ExpressLRS (preview):** every job refuses unless the `elrsPreview` setting is on. `gear elrs read`
  puts a radio or an FC in passthrough until it is restarted or unplugged (a second job needs that
  restart). The binding phrase is the write-only setting `elrsBindingPhrase` (in `SECRET_KEYS`);
  it is never an option, never in a plan, report, log or test, and plans show only
  `uid::fingerprint`. A flash is the usual plan, digest and `confirm`; it keeps the chip's flash as
  a backup first, runs only the `esptool` module, and counts as verified only when esptool says it
  verified the data. Tests use `gear::elrs::fake` and a recording `Runner`; ExpressLRS and esptool
  never download or run for real. Nothing has touched a real device: say so in anything a user
  reads.
