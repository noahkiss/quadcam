# Changelog

User-facing changes in each release, newest first. Release notes on
[GitHub](https://github.com/noahkiss/quadcam/releases) hold the test steps for the later ones.

## Unreleased

- **EdgeTX flash: the right radio.** Before the erase, a flash compares the board the radio's current firmware names with the saved radio's board, and refuses another board. This also covers a DFU device not yet linked to a radio. **Read firmware** says when the image names another board than the radio you picked.
- **EdgeTX flash: one proven release.** A radio flash needs the exact release QuadCam has proven on the board (the Pocket on 2.12.4), not any 2.12 version, and a prerelease tag (`2.12.5-rc1`) always refuses. **Flash …** on the Firmware page follows the same rule.
## 0.12.0 (2026-10-11)

- **Sim sync:** two profiles in one sim file (for example two Liftoff profiles) now sync together. Before, the second write put the first profile back to its old rates.
- **Sim sync plan:** the check "Rewrites unchanged" is now "Reads back as written", which is what it checked.
- **Import card:** the unmount after an import, a later card step and **Safe to remove** find the card by its volume UUID first. If the card was pulled and another disk took its disk number, QuadCam leaves that disk alone.
- **Delete clips after import:** when the card does not mount again for the delete, each clip's result says so. Before, it said the clip was no longer on the card.
- **Docs:** `gear blackbox pull` erases with no `--yes` when **Erase blackbox after download** is on; MCP `blackbox_pull` needs `confirm=true`. The CLI guide now says so.
- **Surface guard:** a new MCP tool's required parameters no longer fail the 1.0 surface test, so the documented baseline update works. The guard now also checks the keys of object parameters (settings names under `values`, profile `fields`, cut and suggestion items) and fails when a value list narrows a parameter or flag that took any value.
- **ExpressLRS (preview) guards.** An ELRS job through an FC refuses an FC that does not identify itself over MSP as the saved FC. A flash checks the device first: a receiver must answer a CRSF ping with the name read before, and its bootloader reply must name the planned target (an empty reply or a bare `UNIFIED` refuses); a radio must be the board the read went through, before its module's boot pin is held. A receiver flash through an FC runs esptool at the receiver UART's 420000 baud, not 460800. An options apply refuses a choice list that changed since the read, and reads the device again after each write. A flashed image now carries the target's hardware overrides from `targets.json` (such as its power table) and the prior target name in capitals, as ExpressLRS's configurator writes them; a target with a screen logo refuses. The `quadcam_gear_edit` description now says that `elrs_read` stops a radio's RF output.
- **A radio flies many aircraft.** A radio's page lists **Aircraft on this radio**: each aircraft profile that names the radio, its EdgeTX model, whether that model is on the card or in the latest backup, and which one the radio selects. **Add aircraft** and **Remove** set the profile's radio, and **Settings > Aircraft** has **Radio** and **EdgeTX model file** for the same link. An FC keeps its one aircraft.
- A radio's own aircraft link from 0.11 or earlier moves onto the profile the first time QuadCam reads `gear.json`. A link the profile cannot take stays in the radio's entry as `legacy_aircraft`.
- CLI `gear devices save --aircraft` and MCP `device_save` `aircraft` add the aircraft to a radio. `gear devices` and MCP `devices` list a radio's aircraft in `radio_aircraft`; a radio's `aircraft` is the one whose model the radio selects, or empty.
- **Voices page** (Gear > Voices). The Voice studio and the voice packs moved out of a radio's Voice segment. A library lists every pack on this Mac with its model, line sets, lines, size and date, and the lines that need a re-take, plays a sample, and deletes a pack after a confirmation that names the radios that chose it (their choice is cleared; the raw takes stay unless you tick the box). **Apply to radios…** stages the pack on each EdgeTX radio you tick, or **All radios**, and shows which cards are connected now; the others apply when plugged in. A radio's Voice segment keeps its pack picker, the per-line overrides and its apply. CLI `gear voice choose --all-radios` and `gear voice delete`; MCP `voice_choose` with `radios` or `all_radios`, and `voice_delete`.
- **USB timer.** The FC's USB heat timer keeps counting while the port drops out for a reboot, USB disk mode or a flash. Before, every job that rebooted the FC started it again at zero.
- **USB heat checks.** A flash, an FC apply and a blackbox pull read the battery before their heat check. A battery in with no timer counting (reads paused, or just plugged in) refuses a flash and is a warning on the others.
- **Betaflight flash reports.** A flash that fails after the erase points to Betaflight Configurator to flash an official build, not to a second QuadCam flash. A flash that fails before the erase says nothing was erased and starts the old firmware again. The sheet and the guide list the firmware copy step.
- **Active profiles.** The settings a Betaflight flash carries over, and an FC Restore, select the PID and rate profile the FC had again. Before, the FC ended on profile 0 after a flash.
- **FC range check.** The check before an FC write reads the `get` answer of the setting itself. Betaflight's `get` also lists settings whose names contain the name, and their ranges could refuse a valid value.
- **Blackbox over USB disk mode.** A pull through the setting reads over MSP when the disk's files do not verify as the flash, instead of failing.

## 0.11.2 (2026-10-10)

- **Voice studio: pay only for what was priced.** **Sample and pay** and **Render and pay** confirm the exact plan the question priced, by its digest. A picker change closes the question. The CLI takes `--digest`, MCP `digest`.
- **Voice studio: re-takes.** A cut that fails a check (silent, too short, too long) no longer goes into the pack or onto the radio. The result lists it as needing a re-take, and **Re-take** renders again with a new seed, paying only for the batches that hold one. MCP takes `seed`.
- **Voice studio: no bad take kept.** A batch whose timestamps do not match its text is not cached, so the next render asks again instead of failing from the cache. A paid request is no longer retried after a server error, which may already be billed.

- **Safer card writes.** A card write that macOS does not finish in time is now waited for before QuadCam puts files back; if it is still running, QuadCam leaves the card mounted and names the backup to restore. "The card is as it was" now shows only after every file read back as before.
- **Model names and card paths.** A model name with `/`, `..` or a leading `.` is refused, since its checklist file is named after it. The card writer refuses any path outside the card.
- **Mount kept.** An apply that refuses no longer unmounts a card you mounted with **Mount**.
- **Clean ._ files** deletes only on a detected card or an EdgeTX card folder.
- **Firmware copies** made in the same second no longer replace each other.

## 0.11.0 (2026-10-10)

- **Blackbox.** The FC page has a Blackbox segment. QuadCam reads the flight controller's blackbox flash over MSP, or from its USB disk, and keeps each pull as a record. It pairs logs with flights by order. Erasing the flash needs a confirmation. CLI and MCP have matching actions.
- **ExpressLRS (preview).** Read an ExpressLRS device, stage changes to its options, and plan and apply a flash. The tools show on the Firmware page when **Show the ELRS tools** is on in Settings > Gear.
- **Betaflight flashing (preview).** The Firmware page plans and applies a Betaflight flash behind the **Betaflight flashing (preview)** setting: backup, bootloader, DFU flash, then your old settings.
- **Voice line sets.** The voice studio offers a full EdgeTX English set (747 prompts) and sets for aircraft kinds, cars and scripts, with more FPV extras.

## 0.10.0 (2026-10-09)

- **Voice studio** (Gear > your radio > Voice). Paste an ElevenLabs API key (stored in the macOS Keychain), pick voices and models, see your credits and a cost estimate, render a sample grid, pick line sets, render a pack and apply it. Raw takes are cached, so cutting again costs nothing.
- **Automatic mount and unmount** for every card job: import, backups, checks, repair, apply, card prep and cleanup. The card unmounts when the job ends.
- **Card prep** in the app: **Prepare card…** on the device page and in the DJI Finish step.
- **Sims page** with an "Out of date" badge, and **Restore** for a sim's backup.
- **ffmpeg through the app.** An **Install ffmpeg…** banner replaces the Homebrew requirement.
- **Radio over USB.** USB Storage and the EdgeTX serial CLI (identify, list files, play a sound, reboot).
- **Firmware.** **Read firmware** saves a copy of a radio's firmware over DFU and changes nothing. A flash needs a verified copy first.
- **MCP and CLI schema version 1**, with a guard against silent removals or renames.

## 0.9.0 (2026-10-09)

- **Rate editor.** Edit a rate profile (type, rates, expo, limits, throttle curve, name) with live curves. **Convert** between Actual and Betaflight rates.
- **Sim sync.** Write a quad's rate profile into Liftoff, Micro Drones, The Zone or Uncrashed. QuadCam backs up the sim's file, refuses while the sim runs, reads back, and rolls back on failure.
- **Scoped Revert.** Revert undoes only that change's lines and warns when a later change touched them.
- **Radio model editors.** Checklists, telemetry screens, logging, timers, RF alarms and callouts, staged as card changes.
- **Voice.** A Voice segment with spelling rules, a render cache, loudness trim, pack building and **Choose voice**. Providers: macOS `say` and a local OpenAI-compatible speech endpoint.
- **Firmware.** Firmware check, Firmware page, splash image editor, and an EdgeTX flash plan over DFU with a backup first.

## 0.8.0 (2026-10-09)

- **OSD editor.** Drag elements on the grid or move them with the arrow keys, turn elements on or off per profile, copy a layout between profiles.
- **Rates segment.** Curves for Betaflight, Actual and Quick rates per profile, the throttle curve, and a compare view for profiles or two quads.
- **Radio card apply.** Staged card changes are backed up, written, read back and rolled back on any mismatch.
- **Mount button.** Mount a radio card to browse it. It unmounts at **Done** or after 10 minutes.
- **Bench page.** Every device's staged changes with Try, Keep and Revert. **Apply ready changes** on plug-in is optional and off by default.
- **Copy settings** between quads, with a compatibility check and a diff.

## 0.7.2 (2026-10-09)

- **Staged changes and FC apply.** Stage changes for a quad, review them in the apply sheet, then apply. QuadCam backs up first, saves, waits through the reboot, verifies every line against a fresh `dump all`, and backs up again. The first editor is **Edit setting**.
- **Sim preview** (off by default): a first-person view in a plain room.
- **Pack-up check** for devices that are not plugged in, from the latest backup.
- **Session report** can be saved from the app, the CLI and MCP.
- **Range trend** per place on Flights.
- **Switch map** shows combined switch positions and lists switches it cannot map.
- The Import sheet shows the card's latest check.

## 0.7.1 (2026-10-09)

- **Fixed:** a changed radio log stored as `<name> (2).csv` no longer shows as a second flight.
- **Fixed:** QuadCam no longer reads an FC port that another program has open. **Pause reads** on the Gear Overview stops FC reads until you resume.

## 0.7.0 (2026-10-07)

- **Gear.** A new sidebar section and status bar for the FPV bench. QuadCam finds EdgeTX radios, DJI goggles and air units, DVR cards, Betaflight flight controllers, ExpressLRS modules and radios in DFU mode. Save a device to name it and link it to an aircraft.
- **Backups** of radio cards and flight controllers on connect, with each file stored once, retention, browse and diff, and **Gear > Storage**.
- **Cues.** QuadCam speaks, plays a sound or posts a notification when a job ends. Mute, per-cue toggles and quiet hours are in Settings.
- **Betaflight link** over MSP and the CLI, with a USB timer that warns before a quad on USB heats up.
- **EdgeTX cards.** Reads the board, version, models and the radio clock.
- **Card check** with `diskutil verifyVolume` before a backup, and **Repair…**.
- **OSD** drawn per profile on the NTSC, PAL or HD grid, with overlap checks.
- **Switch map and Controls.** What each radio switch does, position by position, live in USB Joystick mode.
- **Flights and packs.** Flights read from radio logs (hover, sag, resting voltage, mAh, dropouts). Packs, a session report, **Pack up** and **Repairs**.
- **Modules.** **Settings > Modules** installs ffmpeg and esptool from their upstream, checked against pinned hashes. **QuadCam > Acknowledgements** lists third-party notices.
- **Card prep** formats a card with no import session. A removable DJI goggles card becomes exFAT.
- **Built-in SD slot.** A card in the Mac's SD slot counts as removable.
- **Fixed:** QuadCam detects a DVR's split length instead of assuming 600 s. Card format has no size refusal, and each source picks its file system. A radio-log armed segment is a "flight" everywhere.
- Every Gear feature has `quadcam-cli gear` commands and MCP tools.

## 0.6.4 (2026-10-07)

- Match analog clips to radio logs by their picture, for clips with dead air.

## 0.6.3 (2026-10-07)

- **Split by flight** makes one cut per radio-log flight.
- **Joined recordings.** Files an analog DVR split from one recording import as one clip. The **Join split recordings** setting controls it.

## 0.6.2 (2026-10-06)

- Shape-first radio-log matching for every source, including logs from a radio with a reset clock.
- Re-match radio logs to clips already in the library.
- **Delete clips after import** deletes clip files that verified from the card. It is off by default.

## 0.6.1 (2026-10-06)

- First release signed with a Developer ID and notarized.

## 0.6.0 (2026-10-06)

- **DJI O4 import.** Detect DJI units and goggles cards, date clips by their clock, and byte-copy the MP4 files. A kept `.SRT` sidecar moves with its original.
- **Clip clock skew** field under Log matching.
- No **Format card** for a DJI device.

## 0.5.3 (2026-10-02)

- The toolbar shows the app emblem.
- New README with screenshots; the reference moves to `docs/`.

## 0.5.2 (2026-10-02)

- Faster releases. No change to the app.

## 0.5.1 (2026-10-02)

- **Fixed:** the library scrolls again.

## 0.5.0 (2026-10-02)

- A new interface in the Mocha (dark) and Latte (light) themes.
- Clip ids come from a specified hash. Libraries from 0.4 and earlier move to the new ids on load, and the old ids still work.
- Library cuts read their metadata back before they count.

## 0.4.1 (2026-10-01)

- New app icon.

## 0.4.0 (2026-10-01)

- The app is named QuadCam.
- Place search with Apple, OpenStreetMap, US Census and Google Places providers.
- Edit a clip's date, time and aircraft in the library. Rename in place. Undo and redo for library edits, including Move to Trash.
- Choose the file-name date format: `YYYY-MM-DD` or `YY.MM.DD`.
- Native menu bar, Share menu, previous and next clip, sort control and drag-and-drop import.
- Faster scrolling in large libraries.
- The full MCP surface and matching CLI commands.

## 0.3.0 (2026-10-01)

- The library is the home screen, and import is a sheet over it.
- Library folder layout by year and flying day. The index rebuilds from file metadata.
- Ratings and flags.
- One trim editor for the import review and the library.

## 0.2.1 (2026-10-01)

- QuadCam restores the last session on relaunch. **Start over** clears it.
- The MCP server survives an app restart.

## 0.2.0 (2026-10-01)

- Moments from radio logs and dead air from the video, with cuts and trimmed exports.
- QuickTime metadata that Photos reads: location, local creation date, gear and keywords.
- Places and aircraft profiles, in the app, the CLI and the MCP server.

## 0.1.0 (2026-10-01)

- First release: import analog FPV DVR clips on macOS.
