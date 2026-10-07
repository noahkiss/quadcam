# Gear

Gear is QuadCam's second half: the FPV bench next to the clip library. This page covers what
works today. The full plan is in [Gear design](gear-design.md).

## In the app

**Gear** is a section of the sidebar, under the library groups. Its triangle opens and closes
it.

- **Connected** lists what is plugged in now. Each device also has its own row under it.
- **Devices** lists the devices you saved, plugged in or not.
- **Pack up**, **Flights**, **Packs** and **Repairs** are described under
  [Flights and packs](#flights-and-packs).
- A device's page shows its kind and state, what it reports about itself (board, firmware,
  version), where it is mounted, its aircraft and its latest backup. **Save…** names a device
  QuadCam does not know and links it to an aircraft; **Edit…** changes that; **Forget…**
  removes it from `gear.json` and keeps its backups. **Show clips** opens the aircraft's clips.

The bar along the bottom of the window shows one item for each kind of device plugged in:
radio, quad, DJI and DVR card. Click one to open the device, or the Connected list when there
are several. Each item shows a state:

| State | Means |
|---|---|
| Working | A QuadCam job is using the device |
| Safe to unplug | The card is unmounted and still in |
| Still inserted | The "still inserted" reminder is due. **Dismiss** stops it |
| Needs attention | QuadCam does not know the device yet: save it |
| Connected | Plugged in, nothing to do |

Settings for backups, steps on connect and cues are in **Settings > Gear**
([Settings](settings.md#gear)).

## What QuadCam finds

QuadCam looks for gear every 2 seconds while the app runs. Finding gear opens no port and
writes to no device. The one exception is the USB timer for flight controllers (below): it
reads the battery voltage over MSP every 30 seconds and closes the port again.

| Device | How QuadCam finds it |
|---|---|
| EdgeTX radio | Its SD card in USB Storage mode, or its card in a reader. QuadCam reads `board` and `semver` from `RADIO/radio.yml`. A volume on the radio's own USB device (vendor `OpenTX` or `EdgeTX`, or a radio maker in the product name) is the radio itself, and shows its USB details |
| EdgeTX radio, serial | The radio's USB serial port (`<Maker> <Radio> Serial Port`). It is a radio, not a flight controller |
| Goggles | A DJI volume: a goggles card, or an air unit over USB (DJI clips under `DCIM/DJI_*`) |
| DVR card | A card with analog clips, from a DVR or analog goggles |
| Flight controller | A USB serial port with an STM32 or AT32 virtual COM port id |
| ELRS module | A USB serial port with a CP210x, CH340 or CH9102 id |
| Radio in DFU mode | The STM32 bootloader (`0483:df11`) |

When nothing shows, QuadCam says: "Nothing found. If macOS asked to allow an accessory, click
Allow." macOS keeps a new USB accessory off until you allow it, and QuadCam cannot see that
question.

A card in the built-in SD slot gets its id from a hash of the card's own serial number, so a
format keeps it. A radio card with QuadCam's marker file (`.quadcam-id`) gets its id from that
file. Other cards and radios get theirs from a hash of the volume UUID. A flight
controller gets its id when QuadCam identifies it: a hash of the MCU's unique id, which MSP and
the CLI's `mcu_id` both report. An FC that reports none gets a hash of its board name and USB
serial number. The raw id never leaves the Mac.

A radio in USB Storage mode reports a USB serial number, but EdgeTX radios all report the same
generic one, so QuadCam does not use it as an id. In DFU mode the bootloader reports the chip's
own serial number, which is a stable id; QuadCam cannot match it to the storage-mode radio by
itself, so you link the two once.

## Flight controllers

QuadCam talks to Betaflight over USB in two ways:

- **MSP** (identify): board, firmware, version, the device id. No reboot.
- **The CLI** (read): `version`, `status`, `diff all`, `dump all`, `get NAME`. When the read
  ends, QuadCam leaves the CLI and the FC reboots, as it does in the Betaflight Configurator.

Every job opens the port, works, and closes it. QuadCam never keeps an FC port open between
jobs, so another app can use it, and the FC can be unplugged as soon as the job says "done".
When another app has the port open, QuadCam says so and waits for you.

Plug USB in before the battery: many FCs do not show up on USB when the battery is first.
When several FCs are plugged in, name the port; QuadCam does not guess.

**Writes.** QuadCam writes an FC only through a staged change (a later release). It writes
only boards and Betaflight builds it has tested; any other FC is read-only, and QuadCam says
why ("Board X is not proven."). Today these are proven:

| Board | Betaflight build |
|---|---|
| `BETAFPVG473_V2` | 2026.6.0 |
| `BETAFPVG473` | 2025.12.5 |

**Known issues.** QuadCam lists known issues of some boards and builds on the device and after
each job on it. For example, one 2025.12.5 build leaves the beeper silent after a USB session
until the battery is plugged in again.

**USB timer.** A small quad on USB with its battery in has no airflow and heats fast. While an
FC is on USB with a battery in (more than 1 V on its battery lead), QuadCam counts the minutes.
At the limit it says "Unplug <FC> now." once, and the device shows the time left. The limit is
the `gear_usb_minutes` setting (20), or the board's own limit when it is shorter (10 minutes for
the boards above). `gear_usb_minutes` 0 turns the timer off. Pulling the battery resets it.

## Saved devices

QuadCam keeps the devices you name in `gear.json`, in the gear folder:

```
~/Library/Application Support/app.quadcam/gear/gear.json
```

The `gear_dir` setting moves the gear folder. `gear.json` follows the same rules as the settings
file: every writer changes only what it was asked to change, keeps every other key, and the
running app sees a change from the command line or an agent within 2 seconds.

An aircraft profile can name its gear (`gear`: `fc`, `radio`, `edgetx_model`, `rx`,
`pack_type`). Profiles without it load and save as before.

## EdgeTX cards

QuadCam reads an EdgeTX SD card, in the radio over USB or in a card reader:

- the board and EdgeTX version, and whether QuadCam may write this pair. EdgeTX 2.12 on the
  RadioMaster Pocket is proven; other pairs are read only;
- the models, and the model the radio has selected, with the aircraft profile that names it
  (its `edgetx_model` file, or one of its EdgeTX model names). The app can then warn when
  the radio is set to another quad than the one just connected;
- the radio clock: a log dated 2000-01-01 means the clock was reset, so its clock battery may
  be dead;
- one model in full: timers, mixes, logical switches, special functions, switch warnings,
  telemetry sensors and screens.

QuadCam can also preview card edits: the checks and a line diff, and how long the write would
take. A preview writes nothing. Edits change only the lines they must; every other line stays
byte for byte, line endings included. They never change the radio's selected model unless
the edit asks for it. The power-on checklist is a model setting QuadCam turns on or off.

Writing the edits is part of applying a staged change, a later release.

**Over the radio's USB, writes are slow** (about 0.3 MB/s, a 37 MB voice pack in about 2
minutes). QuadCam writes one file at a time under a temporary name and renames it, so the
card is always whole. Cancel finishes the current file, then unmounts the card, then says
"safe to unplug". Do not pull the radio while a file is being written: once, that wedged the
Mac's disk service until a reboot. If macOS stops answering, QuadCam says a reboot may be
needed instead of waiting forever.

If the radio's firmware stops at an error, hold both horizontal trims inward while you power
it on. The bootloader then shows the SD card over USB.

## Unplugging cards

QuadCam tracks four device events: connected, identified, unmounted but still inserted, and
removed.

A card in a USB card reader is "still inserted" only after `diskutil unmountDisk`: the disk
stays listed until you pull the card. After `diskutil eject` the disk goes at once, so QuadCam
cannot tell an ejected card from a pulled one. The built-in SD slot reports the card until you
pull it. A DJI air unit stays on USB after its volume unmounts.

## Cues

QuadCam can speak, play a sound, and post a notification. It does so once, when a job ends,
never for its own mounts and unmounts along the way:

| Cue | When |
|---|---|
| Done, safe to unplug | A job on a device finished. A run over several devices says it once |
| Still inserted | A card is still in after "done": after 60 seconds, then every 5 minutes, 3 times at most |
| Step failed | A step of a job failed |
| Unplug now | An FC has run on USB with its battery in past its limit (once per battery) |

The `gear_cues` setting holds:

- `mute`, and a toggle for each cue (`safe_to_unplug`, `still_inserted`, `step_failed`,
  `unplug_now`) and
  each channel (`speech`, `sound`, `notification`). Speech and notifications are on by
  default; sound is off.
- `debounce_s` (30): the same cue for the same device plays once in this time.
- `reminder_grace_s` (60), `still_inserted_every_s` (300), `reminder_max` (3).
- `quiet_hours` (`{"start": "22:00", "end": "07:00"}`): no speech or sound in these hours.
  Notifications follow macOS Focus.
- `voice`: a `say` voice. `voice_source`: `macos` (default) or `voice_pack`. Voice packs
  cannot play cues yet, so `voice_pack` also speaks with macOS.

Inside the app bundle, a notification comes from QuadCam through macOS notifications; the
first one asks you to allow them. Outside a bundle (`cargo tauri dev`, a CLI built on its own)
QuadCam posts through `osascript`.

## Steps on connect

The `gear_on_connect` setting names the steps that run when a device of each kind is plugged in:
`backup`, `import` and `apply_ready`. Only `backup` is on by default, and only while
`gear_auto_backup` is on. `backup` runs two steps: **Card check** (a card QuadCam knows) and
**Backup** (a radio card or an FC). `import` and `apply_ready` come in later releases.

A card is unmounted (`diskutil unmountDisk`) at the end of every job: the on-connect steps, a
backup, a card check, a repair. "Done, safe to unplug" plays only after the unmount worked.
When it fails, "Unmount failed" plays, the device shows **Needs attention**, and its
**Backups** segment shows the reason. The card stays in the list until you pull it.

## Backups

QuadCam keeps backups in the gear folder (`gear_dir`). A backup of a radio holds every file on
its SD card except `LOGS/`. A backup of a flight controller holds `version`, `status`,
`diff all` and `dump all`, read through the CLI, so the FC reboots after it.

- **Each file once.** Backups share files: QuadCam stores each file once by its content, under
  `blobs/`, and a backup is a short list of files in `snapshots/<device>/`. Sound files cost
  their size once.
- **Nothing new, nothing kept.** A backup whose files equal the latest one writes nothing. The
  FC's `status` (uptime, load) does not count.
- **Slow links.** A radio over USB reads about 0.5 MB/s. QuadCam reads only files whose size or
  modified time changed since the latest backup, and shows its progress. **Stop** ends a
  backup between files; no backup is saved.
- **Radio logs** are flight data, not backup content. Each log is kept once per radio in
  `logs/<radio>/`. A log that grew replaces the kept one; a log that changed another way is kept
  as a second file (`<name> (2).csv`). Logs are never pruned.
- **Retention** (Settings > Gear): backups taken before an apply or a flash and pinned backups
  stay. Of the rest, QuadCam keeps the newest `gear_keep_recent` (10), then one a week for
  `gear_keep_weeks` (8), then one a month (`gear_keep_monthly`). Pruning runs after each new
  backup, then removes stored files no backup, log or staged change uses.

On a device page, **Backups** lists the device's backups. Pick one to see its changes from the
backup before it, or its files and each text file. **Pinned** keeps a backup. **Back up now**
backs the device up.

**Gear > Storage** shows the gear folder's size in total and per device. **Prune now** shows
what would go and asks first. **Export…** writes a device's backups as plain folders, one per
backup. **Import backups…** takes an old backup folder:

- a folder with `RADIO/radio.yml` or `MODELS/` is a radio card copy; its `LOGS/` go to the
  log store;
- `<name>.diff_all.txt` and `<name>.dump_all.txt` in one folder are one FC backup; a text file
  that reads as Betaflight `diff all` or `dump all` output counts too;
- a plain `LOGS/` folder gives logs only.

Each is dated from a `YYYY-MM-DD` in its folder names, else its newest file. An FC's MCU id
names its device. A card copy goes to the one saved radio with its board; when no saved radio
or more than one matches, the import skips it and says so (pass a device on the command line).
A copy the same as a backup QuadCam has from that day or before is not kept twice. The import
shows what it would do first, and never changes the folder.

## Card check

Pulling a card while it is mounted can leave its file system damaged. Before QuadCam backs up
a card it knows (on connect), it runs `diskutil verifyVolume` on it: read-only, about 30
seconds over a radio's USB. The card unmounts and mounts again while it is checked. **Stop**
ends a check. The result shows on the device's **Backups** segment, and a failed check marks
the device **Needs attention**. The backup still runs.

After a failed check, **Repair…** asks first, backs the card up when it can read it (that
backup is always kept), runs `diskutil repairVolume`, and checks the card again. A repair
cannot be stopped once it starts. Neither command needs an administrator password. QuadCam logs
each check per card in `health/<device>.jsonl` in the gear folder.

## OSD

QuadCam draws a Betaflight OSD layout from a `dump all` or `diff all` file, one screen per OSD
profile, and checks each screen.

- **Positions:** each element has one position, shared by every profile. A profile only turns
  the element on or off.
- **Grids:** the file's `vcd_video_system` picks the grid: NTSC 30 x 13, PAL 30 x 16, HD
  53 x 20. `AUTO` or no setting draws on NTSC. You can pick another grid, or any W x H.
- **Element sizes:** each element draws as sample text (`B4.20V`, `L2:99`, `T00:00`), with
  letters for the font's symbols. The craft name draws as the `craft_name` value. An element
  QuadCam does not know draws 5 wide and is listed. Widths are QuadCam's own estimates, so treat
  a near miss as a hint.
- **Horizon:** the level line is drawn. The check uses its full sweep: 4 columns either side
  and 10 rows down from its position.
- **Check:** two elements on the same cells in one profile, or cells off the grid. The
  crosshairs may sit on the horizon, and other elements may sit on the camera frame.
- **Several files:** QuadCam reads them in order and a later line wins. A dump followed by an
  apply file shows the layout after the apply.
- **A diff alone:** a diff leaves out every setting at its default. QuadCam shows what the diff
  lists and says so.

Open a flight controller's page under **Gear > Devices** and choose **OSD**, then **Open
dump…**. A saved FC with a backup shows its latest backup's `dump all` until you open a file.

## Switch map

The switch map says what each radio control does, position by position. Open a flight
controller's or a radio's page under **Gear > Devices** and choose **Switches**. A device with a
backup shows the latest backups of its aircraft's radio and FC (or its own, with no aircraft);
the radio's model is the one the aircraft profile names under EdgeTX models, else the radio's
selected model. To read files instead, open the radio's card (**Open card…**: a mounted card or
a copy of its folder) or one model file (**Open model…**), and the FC's dump (**Open dump…**).

- **Rows:** each switch (with its type from `radio.yml`: 2-position, 3-position or toggle),
  each trim used as a switch, and each stick a logical switch reads.
- **Each position:** the channel values it sends in µs (−100 % is 988, +100 % is 2012), the
  Betaflight modes it turns on (`aux` lines; AUX1 is CH5) and adjustments it selects
  (`adjrange`; a 3-position select picks rate, OSD or LED profile 1 to 3), and the radio's own
  effects: logical switches that turn on, special functions (sounds, read-outs, screens) and
  timers.
- **How it works out a position:** QuadCam runs the model's mixes with that control there and
  every other control at rest (other switches in their first position, sticks centred,
  throttle low). Mix switches, `ADD`, `MUL` and `REPL` lines, inputs and limits all count. A
  logical switch on telemetry or a timer is left out.
- **Conflicts:** two modes on one range, a switch that does nothing, a mode no control
  reaches, and a sound file the card does not have.
- **Live:** choose **Radio** to follow the radio in USB Joystick mode, or **FC** to read the
  FC's channels over USB once a second. The position each control is in is marked, with the
  modes on.

## Controls

**Gear > Controls** shows the radio live while it is in USB Joystick mode: plug the radio in
and choose **USB Joystick** on it. The page reads the radio directly (the web view cannot see
it as a game controller).

- **Sticks:** both sticks in your stick mode (**Mode 1** to **Mode 4**; Mode 2 by default,
  saved as the `stickMode` setting). Each axis says what it does: throttle is power, yaw turns
  the nose, pitch tilts forward and back, roll banks.
- **Channels:** CH1 to CH8 in µs, and the 24 buttons.
- **Switch map:** open the card or model file and the dump, and the map marks what each
  control does as you move it.

The joystick's axes are the radio's channel outputs, CH1 first, so the model's mixes say which
stick each channel follows. Without a model, CH1 to CH4 read as AETR.

## Flights and packs

QuadCam reads flights from EdgeTX radio logs (`LOGS/*.csv`). A flight is a run of armed log
rows. A row counts as disarmed when the flight mode (`FM`) ends in `*`, as Betaflight sends it.
A log without an `FM` column gives the same flights as log matching. "Pack" always means a
battery.

QuadCam reads logs from three places: the gear folder's `logs/`, folders you add (**Add log
folder…** on the Flights page), and the `LOGS/` of a radio plugged in as USB Storage. It keeps a
cache of each log's flights in `<gear>/flights.json` and reads a log again only when it changes.

**Flights** shows one day's flights. For each flight:

| Measure | How QuadCam finds it |
|---|---|
| Hover | Median throttle over windows of 2 s or more where the throttle moves less than 3 % and roll and pitch stay within 5 % of centre. Also for the first and last third of the flight |
| Sag | Receiver battery (`RxBt`) while armed: the 5th percentile and the minimum. 0 V readings do not count |
| Resting voltage | The median `RxBt` after disarm while the log goes on; else the first reading of the next flight of that model that day |
| Used | `Capa` at disarm |
| Warning | When the flight passed the pack type's mAh warning |
| Worst link | The lowest `RQly` and `1RSS` |
| Dropouts | Spans of 0.3 s or more while armed with `FM` blank and `RxBt` at 0. Each shows the link before and after it. "Telemetry only" means the flight went on and no failsafe mode showed |

Pick the pack for each flight in its row. QuadCam suggests the next label after the last
pack used that day, in label order. The flight's aircraft comes from the profile whose EdgeTX
model names match the log. Its place comes from the place you set, else the clip that holds
the flight, else the profile.

**Session report** (a segment of Flights) sums up one day: flights, air time, the longest
flight, the worst link, dropouts, pack use and crashes. By default it shows the days of the
last import; the import's Finish step has **Show report**. **Copy as Markdown** copies it to
share.

**Packs** lists each pack with its type, its charge state, its cycles (flights on it), and
its median resting voltage and flight time. A pack whose resting voltage sits 0.05 V a cell
under its type's median, or whose flights are 20 % shorter, is marked. **Mark charged** records
a charge; a flight after it makes the pack "Flown" again. Click a label for its history.
**Charging** lists the pack types (chemistry, cells, capacity, connector, full and storage
volts a cell, charge current, mAh warning) with a suggested warning: the mAh where the
resting voltage reaches 3.7 V a cell, from a straight line through the type's flights. The
notes field is free text.

**Pack up** checks, before a session:

| Row | Pass when |
|---|---|
| Packs charged | Every pack in use is marked charged after its last flight |
| Radio model | A radio is plugged in and its selected model belongs to an aircraft |
| Card space | Each goggles or DVR card plugged in has 2 GB and 10 % free |
| Backups | Each saved radio and FC was backed up in the last 14 days |
| Cards out | No card is still in the Mac |

A row QuadCam cannot judge (nothing plugged in, no backups kept yet) reads as unknown.

**Repairs** lists crashes by aircraft. Log a crash from a clip: open the clip, choose the
**Flight** tab, then **Log crash…**. The time is the playhead; add what broke and the parts
used. The clip's Flight tab lists its crashes.

Packs, pack types, the pack set on each flight, added log folders and crashes are yours, in
`gear.json`. No file is written to a clip.

## Command line and agents

```bash
quadcam-cli --json gear status                       # gear folder, Gear settings, what is plugged in
quadcam-cli --json gear devices                      # saved devices
quadcam-cli --json gear devices save <id> --name "Bench radio" --aircraft Whoop
quadcam-cli --json gear devices forget <id>          # its backups stay
quadcam-cli --json gear fc identify [--port /dev/cu.usbmodemX]   # MSP: board, version, id
quadcam-cli --json gear fc read [--cmd "diff all"]... [--out STEM]   # CLI read; the FC reboots
quadcam-cli --json gear fc check STEM.diff_all.txt expected.cli  # offline: lines in the diff
quadcam-cli --json gear fc notes [--board B] [--version V]       # known issues
quadcam-cli --json gear fc usb                                   # USB timers
quadcam-cli gear osd quad.dump_all.txt --text        # each OSD profile drawn, and the check
quadcam-cli --json gear osd quad.dump_all.txt apply.cli --grid PAL
quadcam-cli --json gear card [--mount M | --device ID] [--model model01.yml]
quadcam-cli --json gear card preview --edits edits.json   # checks and diff; writes nothing
quadcam-cli gear map --radio /Volumes/RADIO --fc quad.diff_all.txt --text   # the switch map
quadcam-cli --json gear map --radio model01.yml --fc quad.dump_all.txt --live   # positions now
quadcam-cli gear map --aircraft Whoop --text         # from the latest backups of its radio and FC
quadcam-cli --json gear radio                        # the radio in USB Joystick mode now
quadcam-cli --json gear sim calibration             # the sim's calibration of that radio (see sim.md)
quadcam-cli --json gear flights [--day 2026-10-04] [--aircraft A] [--pack P] [--logs DIR]
quadcam-cli --json gear flights set <flight> --pack A1 [--place NAME]   # "" clears
quadcam-cli --json gear flights folders [--add DIR | --remove DIR]
quadcam-cli gear report [--day 2026-10-04] --markdown  # the session report
quadcam-cli --json gear preflight                    # the Pack up check
quadcam-cli --json gear packs [--target-v 3.7]       # packs, types, charging notes
quadcam-cli --json gear packs save A1 --type "1S 300" [--charged] [--retired true]
quadcam-cli --json gear packs type save "1S 300" --chemistry lihv --cells 1 --warn-mah 250
quadcam-cli --json gear packs notes "Storage after each session"
quadcam-cli --json gear crashes [--aircraft A] [--clip ID]
quadcam-cli --json gear crashes --clip ID save --time 65 --broke "arm" --parts arm,prop
quadcam-cli --json gear backup [--device ID | --port P | --mount M]   # back up a radio card or an FC
quadcam-cli --json gear backups [--device ID]        # newest first
quadcam-cli --json gear backup show <backup> [PATH]  # its files, or one file
quadcam-cli --json gear backup diff <a> [<b>] [--path P]   # from the backup before a, or a to b
quadcam-cli --json gear backup pin <backup> [--off]
quadcam-cli --json gear storage [--prune [--dry-run]] [--export <backup|device> DIR]
quadcam-cli --json gear import-backups FOLDER [--device ID] [--dry-run]
quadcam-cli --json gear card-check [--device ID | --mount M] [--log]
quadcam-cli --json gear card-repair --check <check id> --yes
quadcam-cli --json gear stop <handle>                # stop a backup or card check
```

An edits file is a list. Each edit is a model (`{"kind": "model", "file": "model01.yml",
"name": "<its name>", "ops": [...]}`), the radio (`{"kind": "radio", "ops": [...]}`), a
checklist (`{"kind": "checklist", "model": "model01.yml", "text": "=Props tight"}`), a model
copy or a model delete. Model ops: `rename`, `set_model_id`, `set_flags`, `set_checklist`,
`set_mixes`, `set_logical_switch`, `special_functions`, `move_special_function`,
`set_timer`, `remove_timer`, `swap_timers`, `set_switch_warnings`, `set_screen`. Radio ops:
`set_scalar` and `select_model`. A logical switch or special function may name a telemetry
sensor by label, `tele({RxBt}),35`; QuadCam finds its slot.

Agents use the `quadcam_gear`, `quadcam_gear_edit` and `quadcam_gear_apply` tools. See
[MCP server](mcp.md#gear).

## Tests and real devices

A process started by cargo never reaches real gear:

| Variable | Without it, under cargo | `real` |
|---|---|---|
| `QUADCAM_SERIAL` | No serial ports, no presence checks | Real ports and `ioreg` |
| `QUADCAM_CUES` | Cues are recorded, not played | `say`, `afplay`, `osascript` |
| `QUADCAM_CARD_WRITE` | No card writes under `/Volumes` | Card writes allowed |
| `QUADCAM_HID` | No radio in USB Joystick mode | hidapi |

Card unmounts (`diskutil unmountDisk`) follow `QUADCAM_SERIAL`. Tests use the synthetic card
(`gear::edgetx::synth`) in a temporary folder.
