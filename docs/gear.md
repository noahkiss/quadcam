# Gear

Gear is QuadCam's second half: the FPV bench next to the clip library. This page covers what
works today. The full plan is in [Gear design](gear-design.md).

## In the app

**Gear** is a section of the sidebar, under the library groups. Its triangle opens and closes
it.

- **Connected** lists what is plugged in now. Each device also has its own row under it.
- **Devices** lists the devices you saved, plugged in or not.
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

QuadCam looks for gear every 2 seconds while the app runs. It never opens a port or writes to a
device to do so.

| Device | How QuadCam finds it |
|---|---|
| EdgeTX radio | Its SD card in USB Storage mode, or its card in a reader. QuadCam reads `board` and `semver` from `RADIO/radio.yml`. A volume on the radio's own USB device (vendor `OpenTX` or `EdgeTX`, or a radio maker in the product name) is the radio itself, and shows its USB details |
| EdgeTX radio, serial | The radio's USB serial port (`<Maker> <Radio> Serial Port`). It is a radio, not a flight controller |
| Goggles card | A card with DJI clips |
| DVR card | A card with analog clips |
| Flight controller | A USB serial port with an STM32 or AT32 virtual COM port id |
| ELRS module | A USB serial port with a CP210x, CH340 or CH9102 id |
| Radio in DFU mode | The STM32 bootloader (`0483:df11`) |

When nothing shows, QuadCam says: "Nothing found. If macOS asked to allow an accessory, click
Allow." macOS keeps a new USB accessory off until you allow it, and QuadCam cannot see that
question.

A card in the built-in SD slot gets its id from a hash of the card's own serial number, so a
format keeps it. A radio card with QuadCam's marker file (`.quadcam-id`) gets its id from that
file. Other cards and radios get theirs from a hash of the volume UUID. A flight controller gets
its id when QuadCam identifies it.

A radio in USB Storage mode reports a USB serial number, but EdgeTX radios all report the same
generic one, so QuadCam does not use it as an id. In DFU mode the bootloader reports the chip's
own serial number, which is a stable id; QuadCam cannot match it to the storage-mode radio by
itself, so you link the two once.

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

The `gear_cues` setting holds:

- `mute`, and a toggle for each cue (`safe_to_unplug`, `still_inserted`, `step_failed`) and
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
`gear_auto_backup` is on. QuadCam adds the steps themselves in later releases.

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
dump…**. Once QuadCam keeps device backups, the page will show the FC's latest backup.

## Command line and agents

```bash
quadcam-cli --json gear status                       # gear folder, Gear settings, what is plugged in
quadcam-cli --json gear devices                      # saved devices
quadcam-cli --json gear devices save <id> --name "Bench radio" --aircraft Whoop
quadcam-cli --json gear devices forget <id>          # its backups stay
quadcam-cli gear osd quad.dump_all.txt --text        # each OSD profile drawn, and the check
quadcam-cli --json gear osd quad.dump_all.txt apply.cli --grid PAL
quadcam-cli --json gear card [--mount M | --device ID] [--model model01.yml]
quadcam-cli --json gear card preview --edits edits.json   # checks and diff; writes nothing
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

Card unmounts (`diskutil unmountDisk`) follow `QUADCAM_SERIAL`. Tests use the synthetic card
(`gear::edgetx::synth`) in a temporary folder.
