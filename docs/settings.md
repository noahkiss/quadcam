# Settings

Open Settings with the gear icon or Command-comma. **Done** saves every change at once. **Cancel** or Escape leaves the settings as they were.

## Sections

| Section | Holds |
|---|---|
| **Library** | The library folder (default `~/Movies/quadcam`, made on the first import), the layout, the date in file names, place folders, keep originals, and **Rebuild from files**. See [The library](library.md). |
| **Aircraft** | The aircraft profiles and the default profile. See [Metadata](metadata.md#places-and-profiles). |
| **Places** | The saved places and the place search provider. See [Metadata](metadata.md#place-search-providers). |
| **Import** | The format (MP4 or MOV), the MP4 encoder, the default short name, the time in file names, **Delete clips after import**, **Join split recordings**, and the tolerances for log matching, the DJI clip clock skew included. |
| **Photos** | The album. An empty album means the library only. See [Photos](photos.md). |
| **Gear** | Back up on connect, how many backups to keep, the USB power warning, the steps per device kind when it is plugged in, and the cues: which play, as speech, sound or notification, mute, the voice and quiet hours. See [Gear](gear.md). |
| **Modules** | Tools QuadCam downloads on request (ffmpeg, esptool): install, update, remove, check for updates, and where ffmpeg comes from. See [Modules and notices](modules.md). |
| **Advanced** | Where ffmpeg and the agent socket are. |

### Formats

- **MP4, H.264** (default) uses the VideoToolbox hardware encoder. **MP4 encoder** can pick x264 (software) instead. The files play everywhere and are much smaller: 14 times smaller on the synthetic test clips. Real, noisy footage compresses less.
- **MOV, original MJPEG frames** is a lossless remux that keeps the original frames.

### Delete clips after import

A DJI air unit stops recording when its storage is full. With this setting on, QuadCam deletes the imported clips from the card after each import, so the next flight has room. It works for every source: DJI and analog, a card or a folder.

The setting is off by default. Turning it on is your consent: the app, the command line and an agent then all delete without asking again.

After the import, QuadCam deletes a clip's file only when all of these are true:

- The clip copied off the card and was not skipped.
- Its output verified. QuadCam checks the output again just before the delete: frame count, duration, streams and metadata.
- The file is inside the card or folder that the clips came from, and it is the same file that QuadCam copied: same size and same content fingerprint.

Everything else stays:

- Skipped clips, clips that failed, and clips whose output did not verify again.
- Every file that is not a clip. On a DJI O4 air unit, QuadCam deletes `DCIM/DJI_001/*.MP4` and keeps `MISC/`, the `.SRT` files and the empty `DCIM` folders. On an analog card, it deletes only the imported `.AVI` files.

QuadCam never formats a card for this and never changes its file system. Format card is a separate step, with its own rules; see [Format safety](format-safety.md).

Each import can turn the delete off for that import only:

| From | Keep the clips for one import |
|---|---|
| The app | Clear **Delete clips after import** above **Add to Library**. The checkbox shows only while the setting is on. |
| The command line | `quadcam-cli import --keep-clips` |
| An agent (MCP) | `quadcam_export` with `keep_clips=true` |

Nothing in an import turns the delete on while the setting is off. The import result lists each clip as deleted, or kept with the reason. The Finish step shows the same.

### Join split recordings

On by default. QuadCam imports a recording that an analog DVR split into several files as one clip. Off, every file imports as a clip of its own. Each import, and each recording in the Review step, can choose otherwise. See [Joined recordings](library.md#joined-recordings). In the settings file it is `joinSplitRecordings`.

## The settings file

QuadCam keeps every setting in one file:

```
~/Library/Application Support/app.quadcam/settings.json
```

The app, the command-line tool and the MCP server all read and write this file.

- Every writer changes only the settings it was asked to change. It keeps the rest, including keys that a newer or older QuadCam wrote.
- A change from the command line or an agent shows in the running app at once. The app never writes an older copy over it.
- The Settings window saves only the keys that you changed while it was open.
- Settings from QuadCam 0.3.0 carry over as they are.
- The bottom of the section list shows the QuadCam version and the build (the commit it was built from). **About QuadCam** shows them too.

## Gear

**Settings > Gear** sets most of these. `gear_dir`, `firmware_check`, `tts_provider` and the other voice settings are set with `quadcam-cli settings set` or an agent.

| Setting | Default | Does |
|---|---|---|
| `gear_dir` | `~/Library/Application Support/app.quadcam/gear` | The gear folder |
| `gear_auto_backup` | on | Backs up a device when it is plugged in |
| `gear_keep_recent` | 10 | Plug-in and manual backups kept per device |
| `gear_keep_weeks` | 8 | Then one backup a week for this many weeks |
| `gear_keep_monthly` | on | Then one a month |
| `gear_usb_minutes` | 20 | Minutes an FC may run on USB with its battery in before "Unplug now"; a board's own shorter limit wins (0: off). See [Gear](gear.md#flight-controllers) |
| `gear_on_connect` | `backup` for every kind | Steps per device kind on plug-in: `backup`, `import`, `apply_ready` (opens the apply sheet for the device's Ready changes; nothing is written until you click Apply), `blackbox` (FCs: pulls the blackbox flash; see [Gear](gear.md#blackbox)) |
| `gear_erase_blackbox` | off | Erases an FC's blackbox flash after a pull verified. Off by default, like `delete_clips_after_import`. The CLI and agents respect it and can only skip it for one run |
| `gear_blackbox_msc` | off | A blackbox pull tries the FC's USB disk mode before MSP. Not proven on a real FC |
| `gear_cues` | speech and notifications on | Cues, their channels, mute, debounce, the reminder and quiet hours. See [Gear](gear.md#cues) |
| `sim_preview` | off | Shows **Gear > Sim**, the flight sim preview. Set it in Settings > Gear, or with `quadcam-cli settings set sim_preview=true` |
| `betaflight_flash_preview` | off | Offers **Flash…** for a Betaflight FC on the Firmware page. A preview: it has not been tried on a real FC. Set it in Settings > Gear, or with `quadcam-cli settings set betaflight_flash_preview=true`. See [Gear](gear.md#flash-a-betaflight-fc-preview) |
| `firmware_check` | `manual` | `manual` or `daily`. `manual`: the Firmware page reads the network only when you select **Check for updates**. `daily`: it also checks when the last answer is a day old. See [Gear](gear.md#check) |
| `tts_provider`, `tts_key` | `say`, none | The voice provider: `say` or `openai`. The Voice studio always uses ElevenLabs, with a key in the Keychain, whatever this says. `QUADCAM_TTS_KEY` in the environment wins over the key. Reads show the key only as `(set)` |
| `tts_base_url`, `tts_model`, `tts_voice` | none | For `openai`: the server (such as `http://127.0.0.1:8880`), its model, and the voice your own lines render with. For `say`, `tts_voice` is a `say` voice |
| `voice_index` | none | The address or path of the voice pack index (`voices.json`) |

To read and change settings from a terminal, see [Command line](cli.md#places-profiles-and-settings).
