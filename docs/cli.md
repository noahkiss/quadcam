# Command line

`quadcam-cli` does everything the app does, on the same core. The Homebrew cask puts it on your `PATH`. A build from source has it at `QuadCam.app/Contents/MacOS/quadcam-cli`.

The command-line tool uses the app's settings as its defaults.

## Sessions

Each run continues one import session. QuadCam saves the session to `~/Library/Caches/app.quadcam/session.json`, and the app uses the same file. To use another file, pass `--session FILE`.

## Import

```bash
quadcam-cli status                        # the schema version, tools, settings, cards, radios and the session
quadcam-cli cards                         # detected cards (with their source) and radio log sources
quadcam-cli scan /Volumes/NO\ NAME        # the source and its clips; copies nothing
quadcam-cli stage /Volumes/NO\ NAME       # copy clips to staging, start a session
quadcam-cli analyze                       # probe, recover, make thumbnails
quadcam-cli show                          # the session: clips, plans, results
quadcam-cli dates --logs /path/to/LOGS    # date clips from radio logs
quadcam-cli dates --set 2=2026-10-03      # set one clip's date
quadcam-cli import --name 0=backyard-loops --skip 3 --format mp4
quadcam-cli import --plan plan.json       # names, dates, notes and options from a file
quadcam-cli moments                       # moments, dead air, keep ranges and cuts per clip
quadcam-cli cut 0 12.5-18 1:02-1:10       # set clip 0's cuts (seconds or m:ss)
quadcam-cli cut 0 --keep                  # cut clip 0 down to its keep ranges
quadcam-cli cut 0 --by-flight             # add one cut per radio-log flight
quadcam-cli cut 0 --log-offset 4.5        # the radio log starts 4.5 s into clip 0
quadcam-cli import --cut 0=20-26          # add a cut, then import
quadcam-cli import --time 0=18:30         # clip 0 was flown at 18:30 (the default is noon)
quadcam-cli import --keep-clips           # keep the clips on the card this run
quadcam-cli stage /Volumes/DVR --no-join  # keep split DVR recordings as separate clips this run
quadcam-cli stage --card <device id>       # a card that is unmounted but still in: QuadCam mounts it
quadcam-cli import --separate 0           # import clip 0's files (a split recording) one by one
quadcam-cli meta all --place "Home field" --keywords park,windy
quadcam-cli meta 2 --location 40.6892,-74.0445 --profile Whoop
quadcam-cli cut 0 --clear --removed trash # drop exported cuts and move their files to the Trash
quadcam-cli clear                         # forget the session (start over)
quadcam-cli verify                        # check the outputs again
quadcam-cli photos --album Drone          # add verified outputs to Photos
quadcam-cli eject                         # safe to remove: unmount the card
quadcam-cli format --plan                 # show what would be erased
quadcam-cli format --device /dev/diskN --volume-uuid <uuid> --yes
quadcam-cli format --prep --plan --mount /Volumes/CARD   # card prep: a card with no session
quadcam-cli format --prep --plan --card <device id>      # the same, for a card that is unmounted but still in
quadcam-cli format --prep --device /dev/diskN --volume-uuid <uuid> --yes
```

`clear` deletes the session file. The staged copies stay.

**Mount, work, unmount.** The end of `import` unmounts the session's card (a removable card; never a DJI device over USB), and the result says so under `card`. `format`, `format --plan` and `eject` mount it again when they need it. `stage --card` and `format --prep --card` take a card that an earlier job unmounted and mounted by its device id from `gear status`.

`eject` makes the card safe to remove. For a card it runs `diskutil unmountDisk`: every volume unmounts, and the card stays listed until you pull it. For a disk that is not a card it runs `diskutil eject`.

When the `delete_clips_after_import` setting is on, `import` deletes each clip that verified from the card or folder, and its result lists every clip under `clip_deletion` as `deleted`, or `kept` with a `reason`. `--keep-clips` keeps them for that run. No flag turns the delete on. See [Settings](settings.md#delete-clips-after-import).

`cards` and `scan` name each source: `analog` or `dji`.

`format` refuses with exit code 3 unless every guard passes. It always refuses a DJI session. `--device` (the card's whole disk), `--volume-uuid` and `--yes` must all match the staged card. With `--prep`, `format` erases a card that has no session instead: a new card, or a card whose clips are all in the library. `--plan` then needs `--mount`, and `--device` and `--volume-uuid` must match that card. See [Format safety](format-safety.md#card-prep).

### Plan files

A plan file sets names, dates, times, notes and skips for many clips at once:

```json
{
  "clips": [
    { "id": 0, "name": "backyard loops", "date": "2026-10-04", "time": "18:30", "note": "two packs" },
    { "id": 3, "skip": true }
  ],
  "format": "mp4",
  "output_dir": "/path/to/output"
}
```

## Library

The library commands work on the library folder:

```bash
quadcam-cli library list                          # every clip, newest first
quadcam-cli library list --query flips --group picks
quadcam-cli library list --day 2026-09-27         # groups: last_import, moments, picks, rejected, not_in_photos
quadcam-cli library rate <id> --stars 4 --pick    # also --reject, --unflag; --stars 0 clears
quadcam-cli library rebuild                       # make the index again from the files
quadcam-cli library rename <id> "fence flips"
quadcam-cli library edit <id> --note "windy" --keywords park,windy --place "Home field"
quadcam-cli library edit <id> --date 2026-09-28 --time 18:30   # moves the files to that day
quadcam-cli library edit <id> --profile Whoop     # rewrites make, model, aircraft and keywords
quadcam-cli library cut <id> 12-18 1:02-1:10 --export   # set the cuts and write the new ones
quadcam-cli library cut <id> --by-flight --export        # add one cut per radio-log flight and write them
quadcam-cli library apply-name-format             # rename clips to the name_date_format setting
quadcam-cli library match-logs --logs /path/to/LOGS   # report which clips match a radio log; --apply writes flight numbers and moments
quadcam-cli library trash <id>                    # clip, cuts and original to the Trash
quadcam-cli library photos <id> --album Drone
```

Clip ids come from `library list`.

## Places, profiles and settings

These commands work on the settings file that the app uses. See [Settings](settings.md).

```bash
quadcam-cli places                                # saved places
quadcam-cli places search "Golden Gate Park"      # name, address, latitude, longitude
quadcam-cli places search "Golden Gate Park" --provider nominatim
quadcam-cli places save "Home field" --search "Golden Gate Park" --pick 1
quadcam-cli places save "Home field" --location 37.7694,-122.4862
quadcam-cli places save "Home field" --rename "Park"   # profiles that use it follow
quadcam-cli places delete "Park"
quadcam-cli profiles                              # profiles and the default
quadcam-cli profiles save Whoop --aircraft "65 mm whoop" --camera-make "Fat Shark" \
  --camera-model Echo --keywords tinywhoop --place "Home field" --models AIR65 --default
quadcam-cli profiles save Whoop --author "Your Name"    # changes only that field
quadcam-cli profiles default Whoop
quadcam-cli profiles delete Whoop
quadcam-cli settings                              # the file, its values, the effective settings
quadcam-cli settings set layout=day place_folders=true photos_album=Drone
quadcam-cli settings set output_dir=null          # back to the default
quadcam-cli modules                               # downloaded tools: versions, license, source
quadcam-cli modules install ffmpeg --yes          # without --yes: refuses (exit 3), shows the license
quadcam-cli modules check                         # newest pins from the latest release
quadcam-cli modules remove ffmpeg
```

See [Modules and notices](modules.md) for what a module install checks.

`--provider` takes `apple`, `nominatim`, `census` or `google`. Without it, the search uses the `geocoder` setting.

`settings set` takes these names:

- `output_dir`, `format`, `encoder`, `keep_originals`, `add_time`, `default_name`
- `delete_clips_after_import` (`true` or `false`; see [Settings](settings.md#delete-clips-after-import))
- `join_split_recordings` (`true` or `false`; see [Joined recordings](library.md#joined-recordings))
- `photos_album`, `format_label`, `log_dir`, `layout`, `place_folders`, `tunables`
- `geocoder` (`apple`, `nominatim`, `census` or `google`), `google_places_key`
- `name_date_format` (`YYYY-MM-DD` or `YY.MM.DD`), `default_profile`
- Gear: `gear_dir`, `gear_auto_backup`, `gear_erase_blackbox`, `gear_blackbox_msc`, `gear_keep_recent`, `gear_keep_weeks`, `gear_keep_monthly`, `gear_usb_minutes`, `gear_on_connect`, `gear_cues`, `firmware_check`, `tts_provider`, `tts_base_url`, `tts_model`, `tts_voice`, `voice_index`, `tts_key`, `sim_preview` (see [Settings](settings.md#gear))
- `ffmpeg_source` (`module` or `homebrew`), `modules` (`{"ffmpeg": "/path/to/ffmpeg"}`: a file per tool; see [Modules](modules.md#where-ffmpeg-comes-from))

A value is JSON or plain text.

## Gear

```bash
quadcam-cli --json gear status                       # gear folder, Gear settings, what is plugged in
quadcam-cli --json gear devices                      # saved devices
quadcam-cli --json gear devices save <id> --name "Bench radio" --aircraft Whoop
quadcam-cli --json gear devices forget <id>
quadcam-cli --json gear fc identify [--port /dev/cu.usbmodemX]   # MSP identity, no reboot
quadcam-cli --json gear fc read [--cmd "get osd_ah_pos"]... [--out STEM]   # the FC reboots after
quadcam-cli --json gear fc check STEM.diff_all.txt expected.cli  # every expected line in the diff
quadcam-cli --json gear fc notes [--board BETAFPVG473]            # known issues of boards
quadcam-cli --json gear fc pause|resume [--port ...]                 # pause the running app's FC reads
quadcam-cli --json gear fc usb                                   # USB timers
quadcam-cli --json gear blackbox pull [--port P] [--keep] [--mode auto|msp|msc] [--force]
quadcam-cli --json gear blackbox list [--device ID]
quadcam-cli --json gear blackbox export PULL DIR [--split]
quadcam-cli --json gear blackbox erase [--port P] --yes
quadcam-cli --json gear osd FILE [FILE ...] [--grid NTSC|PAL|HD|WxH] [--text]
quadcam-cli --json gear osd DEVICE [--staged] [--grid ...] [--text]   # --staged: with the device's staged OSD edits on top
quadcam-cli --json gear osd-edit DEVICE [--move ELEMENT=X,Y]... [--profiles ELEMENT=1,3|none]... [--copy FROM:TO]   # stages into the one "OSD layout" change; writes nothing to the FC
quadcam-cli --json gear voice [--radio ID] [--refresh] [--text]   # lines, packs, the provider; --refresh reads the pack index
quadcam-cli --json gear voice render [--voice V] [--lines SOUNDS/en/armed.wav,...] [--dry-run] [--confirm] [--speed S --tempo T --trim-db D --tail-ms N --fade-out-ms N --seed N]   # render with the tts_provider into a local pack; a provider that may charge needs --confirm
quadcam-cli gear voice key set|delete|status   # the ElevenLabs key in the Keychain; `set` reads one line from stdin (hidden on a terminal); never printed
quadcam-cli --json gear voice sets|voices|models|credits   # the line sets; the account's voices; its models with USD per 1K characters, the estimated credits a character and the request limit; the credits left
quadcam-cli --json gear voice estimate --set quad,easter --voice Callum --model eleven_turbo_v2_5 [--lines PATH,...] [--carrier T --max-lines N --snap-ms N]   # the characters (carriers included), batches and credits; no paid call
quadcam-cli --json gear voice sample --voices Callum,Daniel --models eleven_v4,eleven_turbo_v2_5 [--set sample] [--lines PATH,...] [--dry-run] [--confirm]   # a few hard lines per voice and model as WAVs in the cache; bills ElevenLabs, so it needs --confirm and refuses when the credits fall short
quadcam-cli --json gear voice render --set quad,extras --voice Callum --model eleven_turbo_v2_5 [--dry-run] [--confirm]   # the sets as carrier-sentence batches into a local pack; same confirm and credit check
quadcam-cli --json gear voice install PACK [--source INDEX]   # a pack from the index, after a hash check
quadcam-cli --json gear voice edit --radio ID --line PATH [--text T | --pack P | (neither: clear)] [--confirm]   # one line on one radio
quadcam-cli --json gear voice choose --radio ID --pack P [--drop-overrides]   # stages one card change; nothing is written to the card
quadcam-cli --json gear voice build-pack --voice V --out DIR [--id ID --version N --lang en --license L --attribution A] [--dry-run] [--confirm] [render flags]   # maintainer tool: a zip and a voices.json entry
quadcam-cli --json gear model DEVICE [--model model01.yml] [--no-staged] [--text]   # a radio's model for the editors, from the card or the latest backup
quadcam-cli --json gear model-edit DEVICE --model model01.yml [--timer 2:name=FLT,minuteBeep=1]... [--timer-off N]... [--screen 1:'{RxBt},Tmr1/{Capa}']... [--screen-script N:NAME]... [--screen-off N]... [--logging SA2:0.5|off] [--log-sensor RxBt=off]... [--rf-alarm 50:40] [--callout lowbat:below:'{RxBt}':3.5:2:5]... [--callout armed:switch:L1:'!1x']... [--callout-off TRACK]... [--checklist-file FILE|-] [--checklist on|off] [--ops FILE]   # stages into the one "Model edits" change; writes nothing to the card
quadcam-cli --json gear rates FILE [FILE ...]|DEVICE [--backup ID] [--text]   # rate profiles and throttle curve
quadcam-cli --json gear sims [FILE ...|DEVICE] [--backup ID] [--rate-profile N] [--text]   # sims' rates, against the quad
quadcam-cli --json gear sims SIM --restore [--backup ID] [--digest D --yes]   # put a sim's file back from a backup (the plan without --digest)
quadcam-cli --json gear sims FILE|DEVICE [--rate-profile N] --sync --to SIM[:PROFILE][@FILE] [--to ...] [--digest D --yes]   # write the quad's rates into sim profiles
quadcam-cli --json gear firmware [--check]                    # each device's firmware against the newest release; --check reads the network
quadcam-cli --json gear splash IMAGE.png [--threshold N] [--invert] [--board pocket] [--out preview.png]   # a PNG as the radio's 128x64 splash; --out writes the 4x preview
quadcam-cli --json gear firmware --read [--device RADIO]       # read-only DFU trial: copy the firmware, compare the version
quadcam-cli --json gear firmware --plan --device RADIO [--version V] [--splash IMAGE.png] [--threshold N] [--invert]   # an EdgeTX flash: checks, diff, digest
quadcam-cli --json gear firmware --device RADIO [--version V] [--splash IMAGE.png] --digest D --yes      # flash the radio in DFU mode
quadcam-cli --json gear map --radio CARD|MODEL.yml [--model model01.yml] --fc FILE [--fc FILE] [--live [--port P]] [--channels 1500,...] [--text]
quadcam-cli --json gear map --aircraft NAME | --device ID [--device ID]   # from the latest backups
quadcam-cli --json gear radio [--wait-ms 500]                    # the radio in USB Joystick mode
quadcam-cli --json gear sim calibration [RADIO] [--set FILE.json [--product NAME]]   # the sim's radio calibration
quadcam-cli --json gear sim defaults [--aircraft NAME | --radio CARD --fc FILE]     # what the sim pre-fills
quadcam-cli --json gear sim validate PROFILE --logs DIR [--poles N] [--text]      # a sim profile against decoded blackbox logs
quadcam-cli --json gear flights [--day D] [set <flight> --pack L | folders --add DIR]
quadcam-cli gear report [--day D] --markdown                     # the session report
quadcam-cli --json gear report [--day D] --out FILE [--force]     # write the Markdown to FILE
quadcam-cli --json gear preflight                                # the Pack up check
quadcam-cli --json gear packs [save L --type T --charged | type save T ... | notes TEXT]
quadcam-cli --json gear crashes [--clip ID save --time S --broke TEXT --parts a,b]
quadcam-cli --json gear backup [--device ID | --port P | --mount M] [show|diff|pin ...]
quadcam-cli --json gear backups [--device ID]
quadcam-cli --json gear storage [--prune [--dry-run]] [--export <backup|device> DIR]
quadcam-cli --json gear import-backups FOLDER [--device ID] [--dry-run]
quadcam-cli --json gear card-check [--device ID | --mount M] [--log]
quadcam-cli --json gear card-repair --check <check id> --yes
quadcam-cli --json gear card-clean [--device ID | --mount M] [--remove --yes]   # list or delete the ._ files macOS left on a card
quadcam-cli --json gear radio-cli identify|ls|play|beep|reboot|verify [--port P] [--path P] [--device ID] [--yes]   # a radio on its USB serial port (EdgeTX CLI)
quadcam-cli --json gear dfu-link [--device ID] [--serial S] [--unlink]   # link the radio in DFU mode to a saved radio
quadcam-cli --json gear stage --device ID --set NAME=VALUE [--pid-profile N | --rate-profile N] [--title T]   # stage; writes nothing
quadcam-cli --json gear stage --device ID --cli FILE   # raw CLI lines, no save/exit/defaults
quadcam-cli --json gear stage --device RADIO_ID --edits FILE.json   # card edits (JSON array)
quadcam-cli --json gear changes [--device ID] [--status ready] [--history]
quadcam-cli --json gear update CHANGE --status draft|ready|try|read_first [--title T] [--note N] [--order N]
quadcam-cli --json gear apply CHANGE --plan            # every check, the diff, the digest; no reboot
quadcam-cli --json gear apply CHANGE --digest D --yes  # FC: backup, write, save, read back, verify; card: mount, backup, write, read back, roll back on a mismatch, unmount
quadcam-cli --json gear restore BACKUP [--path P]...   # stage a backup back (a card backup needs --path)
quadcam-cli --json gear keep CHANGE                    # an applied Try change becomes Verified
quadcam-cli --json gear revert CHANGE                  # stage a restore of the backup its apply took
quadcam-cli --json gear copy --from FC|BACKUP --to FC --part rates --part osd [--setting NAME]... [--plan]   # --plan shows the checks and diff; without it, stages the copy
quadcam-cli --json gear card-mount RADIO_ID [--minutes 10]   # mount an unmounted card to browse it
quadcam-cli --json gear card-unmount RADIO_ID
quadcam-cli --json gear discard CHANGE
quadcam-cli --json gear stop <handle>
```

`gear fc read --out STEM` writes `STEM.diff_all.txt` and `STEM.dump_all.txt` and refuses to
overwrite. `gear fc check` is right only for values that differ from the default: a default
value never shows in `diff all`.

`gear blackbox pull` reads an FC's blackbox flash, verifies and stores it, and erases the flash only when the
`gear_erase_blackbox` setting is on and the checks pass. `--keep` skips the erase for that run; no flag turns it on.
It refuses a pull that would outlast the USB heat timer (`--force` overrides that, never an erase that could not
finish) and a port another program has open. `list` shows the stored pulls newest first, each log paired with a
radio-log flight by order (a labelled guess). `erase --yes` empties the flash when QuadCam holds a stored pull of
exactly what it holds. See [Gear](gear.md#blackbox).

See [Gear](gear.md).

## JSON output

Add `--json` to any command to get one JSON object on stdout:

- On success: `{"ok": true, "result": ...}`. A run that used a deprecated command or flag adds `"warnings": [...]`.
- On failure: `{"ok": false, "error": {"code": ..., "exit": ..., "message": ...}}`

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | The command failed |
| 2 | Wrong usage |
| 3 | A safety check refused the command |
| 4 | No session, no card, or no device |

## Stability

From 1.0 the command line is a contract. The schema version is 1; `quadcam-cli --schema-version` prints it, and `quadcam-cli --json status` and the MCP server report the same number.

- **Additions stay allowed.** A new command, flag, positional argument, value or output field does not change the schema version.
- **Removals and renames need a schema version bump and a deprecation period.** QuadCam marks the old command or flag deprecated first. It keeps working and prints a warning on stderr (`warnings` in `--json` output) that names the replacement. A later schema version removes it.
- **Output fields stay.** A field that scripts read does not change its meaning or type.
- **Exit codes stay** as the table below lists them.
- `tests/surface/cli.txt` records every command, flag and argument. A test compares it with the built tool, so a removal or a rename fails the build.

### MCP equivalents

Every MCP tool has a command. The command line also has `scan`, `analyze`, `clear`, `moments`, `modules manifest`, `gear fc check` and `gear voice build-pack`. An MCP `snake_case` action is the same `kebab-case` command under `gear`. The exceptions:

| MCP | Command |
|---|---|
| `quadcam_status` | `status` (the card list alone: `cards`; the session alone: `show`) |
| `quadcam_load_clips` | `stage`, then `analyze` and `dates` |
| `quadcam_suggest` | `import` flags (`--name`, `--note`, `--date`, `--cut`, ...), `meta`, `cut`, `dates` |
| `quadcam_settings` `read`, `write` | `settings show`, `settings set` |
| `quadcam_profiles` `set_default` | `profiles default` |
| `quadcam_gear` `switch_map` | `gear map` |
| `quadcam_gear` `session_report` | `gear report` |
| `quadcam_gear` `board_notes`, `usb_timers` | `gear fc notes`, `gear fc usb` |
| `quadcam_gear` `*_plan` | the same command without `--yes` or `--digest` (or with `--plan`) |
| `quadcam_gear_apply` | the same command with `--digest D --yes` (MCP `confirm=true`) |
| `quadcam_library_files` `cuts`, `split_by_flight`, `export_cuts` | `library cut` (`--by-flight`, `--export`) |

Flags that mean one thing everywhere: `--yes` is MCP `confirm`, `--plan` shows the checks and digest and writes nothing, `--device` is a saved device id, `--mount` is a mount point, `--profile` is an aircraft profile. `gear voice` keeps `--confirm`: it allows a voice provider that may charge, which is a different consent.

## MCP server

`quadcam-cli mcp` runs an MCP server. See [MCP server](mcp.md).
