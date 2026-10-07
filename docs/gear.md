# Gear

Gear is QuadCam's second half: the FPV bench next to the clip library. This page covers what
works today. The full plan is in [Gear design](gear-design.md).

## What QuadCam finds

QuadCam looks for gear every 2 seconds while the app runs. It never opens a port or writes to a
device to do so.

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
controller gets its id when QuadCam identifies it.

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

QuadCam can speak, play a sound, and post a notification:

| Cue | When |
|---|---|
| Safe to unplug | A card unmounted and is still inserted |
| Still inserted | The card is still in, every `still_inserted_every_s` seconds (default 60) |
| Step failed | A step that ran on its own, such as a backup, failed |

The `gear_cues` setting turns each cue and each channel (`speech`, `sound`, `notification`) on
or off. Speech and notifications are on by default; sound is off. `voice` picks a `say` voice.

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
```

Agents use the `quadcam_gear`, `quadcam_gear_edit` and `quadcam_gear_apply` tools. See
[MCP server](mcp.md#gear).

## Tests and real devices

A process started by cargo never reaches real gear:

| Variable | Without it, under cargo | `real` |
|---|---|---|
| `QUADCAM_SERIAL` | No serial ports, no presence checks | Real ports and `ioreg` |
| `QUADCAM_CUES` | Cues are recorded, not played | `say`, `afplay`, `osascript` |
