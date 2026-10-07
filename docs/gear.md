# Gear

Gear is QuadCam's second half: the FPV bench next to the clip library. This page covers what
works today. The full plan is in [Gear design](gear-design.md).

## What QuadCam finds

QuadCam looks for gear every 2 seconds while the app runs. Finding gear opens no port and
writes to no device. The one exception is the USB timer for flight controllers (below): it
reads the battery voltage over MSP every 30 seconds and closes the port again.

| Device | How QuadCam finds it |
|---|---|
| EdgeTX radio | Its SD card in USB Storage mode. QuadCam reads `board` and `semver` from `RADIO/radio.yml` |
| Goggles card | A card with DJI clips |
| DVR card | A card with analog clips |
| Flight controller | A USB serial port with an STM32 or AT32 virtual COM port id |
| ELRS module | A USB serial port with a CP210x, CH340 or CH9102 id |
| Radio in DFU mode | The STM32 bootloader (`0483:df11`) |

When nothing shows, QuadCam says: "Nothing found. If macOS asked to allow an accessory, click
Allow." macOS keeps a new USB accessory off until you allow it, and QuadCam cannot see that
question.

A card in the built-in SD slot gets its id from a hash of the card's own serial number, so a
format keeps it. Other cards and radios get theirs from a hash of the volume UUID. A flight
controller gets its id when QuadCam identifies it: a hash of the MCU's unique id, which MSP and
the CLI's `mcu_id` both report. An FC that reports none gets a hash of its board name and USB
serial number. The raw id never leaves the Mac.

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
- `voice`: a `say` voice.

## Steps on connect

The `gear_on_connect` setting names the steps that run when a device of each kind is plugged in:
`backup`, `import` and `apply_ready`. Only `backup` is on by default, and only while
`gear_auto_backup` is on. QuadCam adds the steps themselves in later releases.

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
```

Agents use the `quadcam_gear`, `quadcam_gear_edit` and `quadcam_gear_apply` tools. See
[MCP server](mcp.md#gear).

## Tests and real devices

A process started by cargo never reaches real gear:

| Variable | Without it, under cargo | `real` |
|---|---|---|
| `QUADCAM_SERIAL` | No serial ports, no presence checks | Real ports and `ioreg` |
| `QUADCAM_CUES` | Cues are recorded, not played | `say`, `afplay`, `osascript` |
