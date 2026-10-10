# Changelog

User-facing changes in each release, newest first. Release notes on
[GitHub](https://github.com/noahkiss/quadcam/releases) hold the test steps for the later ones.

## Unreleased

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
