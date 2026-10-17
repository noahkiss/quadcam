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
quadcam-cli dates --no-logs               # ignore radio logs; every clip gets the import date
quadcam-cli dates --logs /path/to/LOGS --day 2026-10-03   # match against one log day
quadcam-cli names                         # names, keywords and notes from Apple's on-device model (empty fields only)
quadcam-cli import --name 0=backyard-loops --skip 3 --format mp4
quadcam-cli import --plan plan.json       # names, dates, notes and options from a file
quadcam-cli moments                       # moments, dead air, keep ranges and cuts per clip
quadcam-cli cut 0 12.5-18 1:02-1:10       # set clip 0's cuts (seconds or m:ss)
quadcam-cli cut 0 --keep                  # cut clip 0 down to its keep ranges
quadcam-cli cut 0 --by-flight             # add one cut per radio-log flight
quadcam-cli cut 0 --log-offset 4.5        # the radio log starts 4.5 s into clip 0
quadcam-cli import --cut 0=20-26          # add a cut, then import
quadcam-cli import --time 0=18:30         # clip 0 was flown at 18:30 (the default is noon)
quadcam-cli import --keep-originals --add-to-photos --album Drone   # also --encoder, --output, --add-time, --note, --date
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
quadcam-cli format --plan                 # show what would be erased; --label sets the volume name (default DVR)
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
quadcam-cli library shift <id>... --from <id> --time 18:30 [--date 2026-09-28] [--logs DIR]   # print the plan: every clip shifts by the offset of --from, a matching radio log wins; --yes writes it
quadcam-cli library analyze-video [<id>...]       # highlights and key frame (macOS 27); skips clips done; --again redoes them
quadcam-cli library weather [<id>...]             # wind, temperature and sky at each clip's place and hour (Open-Meteo; needs weather=true); a clip with weather keeps it, except one saved before the sky values
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
quadcam-cli places here                           # this Mac's location (Location Services), named by Apple's geocoder; saves nothing
quadcam-cli places reverse 37.7694,-122.4862      # name a point with Apple's geocoder
quadcam-cli profiles                              # profiles and the default
quadcam-cli profiles save Whoop --aircraft "65 mm whoop" --camera-make "Fat Shark" \
  --camera-model Echo --keywords tinywhoop --place "Home field" --models AIR65 --default
quadcam-cli profiles save Whoop --author "Your Name"    # changes only that field
quadcam-cli profiles default Whoop
quadcam-cli profiles delete Whoop
quadcam-cli settings                              # the file, its values, every setting in effect (`named`)
quadcam-cli settings set layout=day place_folders=true photos_album=Drone
quadcam-cli settings set output_dir=null          # back to the default
op read "op://<vault>/<item>/<field>" | quadcam-cli settings set-secret google_places_key   # a secret from stdin
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
- `video_analysis` (`true` or `false`; see [Settings](settings.md#find-highlights-in-new-clips))
- `weather` (`true` or `false`, off by default; see [Settings](settings.md#look-up-weather))
- `on_device_suggestions` (`true` or `false`; see [On-device suggestions](settings.md#on-device-suggestions))
- `update_check` (`true` or `false`; see [Updates](settings.md#updates))
- `map_view` (`true` or `false`, off by default; see [Show the Map page](settings.md#show-the-map-page))
- `map_style` (`standard`, `hybrid` or `satellite`; see [Map style](settings.md#map-style))
- `photos_album`, `format_label`, `log_dir`, `layout`, `place_folders`, `tunables`
- `geocoder` (`apple`, `nominatim`, `census` or `google`), `google_places_key`
- `name_date_format` (`YYYY-MM-DD` or `YY.MM.DD`), `default_profile`
- Gear: `gear_dir`, `gear_auto_backup`, `gear_erase_blackbox`, `gear_blackbox_msc`, `gear_keep_recent`, `gear_keep_weeks`, `gear_keep_monthly`, `gear_usb_minutes`, `gear_on_connect`, `gear_cues`, `firmware_check`, `betaflight_flash_preview`, `tts_provider`, `tts_base_url`, `tts_model`, `tts_voice`, `voice_index`, `tts_key`, `sim_preview`, `elrs_preview`, `elrs_binding_phrase`, `elrs_region`, `elrs_wifi_interval` (see [Settings](settings.md#gear))
- `ffmpeg_source` (`module` or `homebrew`), `modules` (`{"ffmpeg": "/path/to/ffmpeg"}`: a file per tool; see [Modules](modules.md#where-ffmpeg-comes-from))

A value is JSON or plain text.

`google_places_key`, `tts_key` and `elrs_binding_phrase` are secrets. `settings set-secret KEY` reads one from one line of stdin (hidden at a terminal), so it never sits in argv or shell history. `settings set` still sets one, and prints a note on stderr that other processes can read a command's arguments. `settings show` lists every setting by its command-line name with its value in effect in `named` (secrets aside). It keeps `"(set)"` in `values` and puts each secret's hint in `secrets` (`hint`, `prefix`, `suffix`, `length`; see [Secrets](settings.md#secrets)).

## Gear

```bash
quadcam-cli --json gear status                       # gear folder, Gear settings, what is plugged in
quadcam-cli --json gear devices                      # saved devices (`devices list` is the same)
quadcam-cli --json gear devices save <id> --name "Bench radio" --aircraft Whoop   # a radio: adds Whoop to its aircraft
quadcam-cli --json gear devices save <fc> --firmware-channel vendor|official|auto   # an FC's firmware source; auto follows what QuadCam detects
quadcam-cli --json gear devices forget <id>
quadcam-cli --json gear fc identify [--port /dev/cu.usbmodemX]   # MSP identity, no reboot
quadcam-cli --json gear fc read [--cmd "get osd_ah_pos"]... [--out STEM]   # the FC reboots after
quadcam-cli --json gear fc check STEM.diff_all.txt expected.cli  # every expected line in the diff
quadcam-cli --json gear fc notes [--board BOARD] [--version V]     # known issues of boards
quadcam-cli --json gear fc pause|resume [--port ...]                 # pause the running app's FC reads
quadcam-cli --json gear fc usb                                   # USB timers
quadcam-cli --json gear blackbox pull [--port P] [--keep] [--mode auto|msp|msc] [--force]
quadcam-cli --json gear blackbox summary [--port P]                # the flash's size and used bytes; no pull
quadcam-cli --json gear blackbox list [--device ID]
quadcam-cli --json gear blackbox export PULL DIR [--split]
quadcam-cli --json gear blackbox erase [--port P] --yes
quadcam-cli --json gear osd FILE [FILE ...] [--grid NTSC|PAL|HD|WxH] [--text]
quadcam-cli --json gear osd DEVICE [--staged] [--grid ...] [--text]   # --staged: with the device's staged OSD edits on top
quadcam-cli --json gear osd-edit DEVICE [--move ELEMENT=X,Y]... [--profiles ELEMENT=1,3|none]... [--copy FROM:TO]   # stages into the one "OSD layout" change; writes nothing to the FC
quadcam-cli --json gear voice [--radio ID] [--refresh] [--text]   # lines, packs, the provider; --refresh reads the pack index
quadcam-cli --json gear voice render [--voice V] [--lines SOUNDS/en/armed.wav,...] [--dry-run] [--confirm] [--speed S --tempo T --trim-db D --tail-ms N --fade-out-ms N --seed N]   # without --set: render with the tts_provider (say or openai) into a local pack; a provider that may charge needs --confirm
quadcam-cli gear voice key set|delete|status   # the ElevenLabs key in the Keychain; `set` reads one line from stdin (hidden on a terminal); prints only the key's hint
quadcam-cli --json gear voice sets [--voice ID --model M]   # the line sets with what each holds and the aircraft it is for, the aircraft you fly and the sets the studio ticks; with a voice id and model, how many lines of each set are rendered; no call
quadcam-cli --json gear voice import-takes FOLDER [--dry-run]   # copy earlier takes of single lines (.json beside .pcm) into the cache; they count as rendered at their voice, model and speed; reads the folder only
quadcam-cli --json gear voice voices|models|credits   # the account's voices; its models with USD per 1K characters, the estimated credits a character and the request limit; the credits left
quadcam-cli --json gear voice estimate --set quad,easter --voice Callum --model eleven_turbo_v2_5 [--lines PATH,...] [--carrier T --max-lines N --snap-ms N]   # the lines rendered and to render, then the characters (carriers included), batches and credits of the lines to render; no paid call
quadcam-cli --json gear voice sample --voices Callum,Daniel --models eleven_v4,eleven_turbo_v2_5 [--set sample] [--lines PATH,...] [--dry-run] [--confirm --digest D]   # a few hard lines per voice and model as WAVs in the sample gallery (the gear folder's voice-samples/); bills ElevenLabs, so it needs --confirm with the digest the run without it printed, and refuses when the credits fall short or the plan changed
quadcam-cli --json gear voice samples [--delete ID,... --yes | --all --yes] [--text]   # the sample gallery: each sampled voice (model, speed, accent, languages, description) with its takes, what the speech check heard and any flag, then the packs on this Mac
quadcam-cli --json gear voice flags [--text]   # every flagged take: voice, model, speed, seed, line, text, heard text, file, reason, note
quadcam-cli --json gear voice flag LINE --sample ID | --pack P --reason mispronounced|clipped|wrong_tone|noise|other [--note N] [--clear]   # flag a take (or clear its flag); a flagged studio take stays out of renders with the same seed
quadcam-cli --json gear voice retake --sample ID | --pack P [--lines PATH,...] [--dry-run] [--confirm --digest D]   # the flagged takes (or a studio pack's re-take lines named in --lines) again with another seed; bills ElevenLabs, so it needs --confirm with the digest the run without it printed
quadcam-cli --json gear voice render --set quad,extras --voice Callum --model eleven_turbo_v2_5 [--dry-run] [--confirm --digest D] [--seed N]   # the sets as carrier-sentence batches into a local pack; same confirm, digest and credit check; lines whose cut failed a check are listed under retakes, and --seed N redoes only their batches
quadcam-cli --json gear voice render-packs --voices Callum,Daniel --models eleven_turbo_v2_5 --set quad,extras [--dry-run] [--confirm --digest D] [--seed N]   # one local pack per voice and model in one run: without --confirm it prints each pair's cost, the total and one digest for the whole plan; a failure keeps the packs made before it
quadcam-cli --json gear voice library [--search WORDS] [--language en] [--accent british] [--gender G] [--age young|middle_aged|old] [--use-case U] [--category professional] [--descriptives calm,deep] [--sort trending] [--page N] [--page-size N] [--id VOICE_ID]   # ElevenLabs' Voice Library, a page at a time (or one voice by id): labels, languages, any rate of its own, and whether the account has it; free
quadcam-cli --json gear voice add VOICE_ID [--name N] [--yes]   # add a library voice to the account's My Voices; it takes a voice slot (no credits), so without --yes it only shows what would be added
quadcam-cli --json gear voice check PACK [--lines PATH,...] [--accept PATH,...] [--install]   # hear each line with the on-device speech recognizer (macOS 26 or later) and compare it with its text; in a studio pack a line heard wrong becomes a re-take; --accept keeps what was heard; --install first installs the speech assets; no provider call
quadcam-cli --json gear voice install PACK [--source INDEX]   # a pack from the index, after a hash check
quadcam-cli --json gear voice preview PACK [--source INDEX]   # an index pack's preview lines, fetched into the cache after a hash check
quadcam-cli --json gear voice publish PACK [--id ID --version N --voice-name V --license L --attribution A] [--text]   # prepare a pack rendered here for QuadCam Voices: refuses unless the speech check cleared every line; prints the review and its digest; publishes nothing
quadcam-cli gear voice publish PACK --approve --digest D   # at a terminal only: type the pack's id, then the release and the index pull request (gh), or the commands to run
quadcam-cli --json gear voice edit --radio ID --line PATH [--text T | --pack P | (neither: clear)] [--dry-run] [--confirm --digest D]   # one line on one radio; a paid --text: --dry-run prints the characters and the digest
quadcam-cli --json gear voice choose --radio ID --pack P [--drop-overrides]   # stages one card change; nothing is written to the card
quadcam-cli --json gear voice choose --radio ID,ID2 | --all-radios --pack P [--drop-overrides]   # one card change per radio (every saved EdgeTX radio with --all-radios); answers {pack, staged}
quadcam-cli --json gear voice delete PACK --yes [--takes]   # removes an installed or rendered pack from this Mac and clears the radios' choice of it (without --yes it names them); --takes also removes its raw takes from the cache
quadcam-cli --json gear voice build-pack --voice V --out DIR [--id ID --version N --lang en --license L --attribution A] [--dry-run] [--confirm] [render flags]   # maintainer tool: a zip and a voices.json entry
quadcam-cli --json gear model DEVICE [--model model01.yml] [--no-staged] [--text]   # a radio's model for the editors, from the card or the latest backup
quadcam-cli --json gear model-edit DEVICE --model model01.yml [--timer 2:name=FLT,minuteBeep=1]... [--timer-off N]... [--screen 1:'{RxBt},Tmr1/{Capa}']... [--screen-script N:NAME]... [--screen-off N]... [--logging SA2:0.5|off] [--log-sensor RxBt=off]... [--rf-alarm 50:40] [--callout lowbat:below:'{RxBt}':3.5:2:5]... [--callout armed:switch:L1:'!1x']... [--callout-off TRACK]... [--checklist-file FILE|-] [--checklist on|off] [--ops FILE]   # stages into the one "Model edits" change; writes nothing to the card
quadcam-cli --json gear radio-settings DEVICE [--no-staged] [--text]   # a radio's own settings (radio.yml) for the Settings editor: each key, its value in the file's units and the values it takes
quadcam-cli --json gear radio-settings-edit DEVICE [--set beepVolume=3]... [--select-model model00.yml] [--ops FILE]   # stages into the one "Radio settings" change; a value out of range refuses; writes nothing to the card
quadcam-cli --json gear rates FILE [FILE ...]|DEVICE [--backup ID] [--text]   # rate profiles and throttle curve
quadcam-cli --json gear sims [FILE ...|DEVICE] [--backup ID] [--rate-profile N] [--text]   # sims' rates, against the quad
quadcam-cli --json gear sims SIM --restore [--backup ID] [--digest D --yes]   # put a sim's file back from a backup (the plan without --digest)
quadcam-cli --json gear sims FILE|DEVICE [--rate-profile N] --sync --to SIM[:PROFILE][@FILE] [--to ...] [--digest D --yes]   # write the quad's rates into sim profiles
quadcam-cli --json gear firmware [--check]                    # each device's firmware against the newest release (an FC on its maker's build: the maker's newest file); --check reads the network
quadcam-cli --json gear elrs [status [--check]]               # ExpressLRS devices read so far (preview: settings set elrs_preview=true)
quadcam-cli --json gear elrs read HOST [--port P]             # the module behind a saved radio (stops its pulses) or the receiver behind a saved FC
quadcam-cli --json gear elrs set DEVICE OPTION=VALUE [...]    # stage ELRS options (packet_rate, telemetry_ratio, power, dynamic_power, switch_mode, model_match); apply with gear apply
quadcam-cli --json gear elrs flash DEVICE [--version V] [--sha256 H] [--port P]   # an ExpressLRS flash: checks, diff, digest
quadcam-cli --json gear elrs flash DEVICE [--version V] --digest D --yes          # flash with the esptool module
quadcam-cli --json gear splash IMAGE.png [--threshold N] [--invert] [--board pocket] [--out preview.png]   # a PNG as the radio's 128x64 splash; --out writes the 4x preview
quadcam-cli --json gear firmware --read [--device RADIO]       # read-only DFU trial: copy the firmware, compare the version
quadcam-cli --json gear firmware --copies [--device RADIO]     # saved firmware copies, newest first: date, version, size, SHA-256
quadcam-cli --json gear firmware --restore COPY --device RADIO --plan   # restore a saved copy: checks, diff, digest
quadcam-cli --json gear firmware --restore COPY --device RADIO --digest D --yes   # restore it to the radio in DFU mode
quadcam-cli --json gear firmware --plan --device RADIO [--version V] [--splash IMAGE.png] [--threshold N] [--invert]   # an EdgeTX flash: checks, diff, digest
quadcam-cli --json gear firmware --device RADIO [--version V] [--splash IMAGE.png] --digest D --yes      # flash the radio in DFU mode
quadcam-cli --json gear firmware --plan --device FC [--version 2026.6.0]    # a Betaflight flash (needs betaflight_flash_preview=true): checks, diff, digest
quadcam-cli --json gear firmware --device FC [--version 2026.6.0] --digest D --yes   # back up, flash over DFU, put the settings back
quadcam-cli --json gear map --radio CARD|MODEL.yml [--model model01.yml] --fc FILE [--fc FILE] [--live [--port P]] [--channels 1500,...] [--text]
quadcam-cli --json gear map --aircraft NAME | --device ID [--device ID]   # from the latest backups
quadcam-cli --json gear radio [--wait-ms 500]                    # the radio in USB Joystick mode
quadcam-cli --json gear sim calibration [RADIO] [--set FILE.json [--product NAME]]   # the sim's radio calibration
quadcam-cli --json gear sim defaults [--aircraft NAME | --radio CARD --fc FILE]     # what the sim pre-fills
quadcam-cli --json gear sim validate PROFILE --logs DIR [--poles N] [--text]      # a sim profile against decoded blackbox logs
quadcam-cli --json gear flights [--day D] [--aircraft A] [--pack L] [--place P] [--logs DIR] [set <flight> --pack L --place P | folders --add DIR | folders --remove DIR]
quadcam-cli gear report [--day D] --markdown                     # the session report
quadcam-cli --json gear report [--day D] --summarize             # with a paragraph from the on-device model
quadcam-cli --json gear report [--day D] --out FILE [--force]     # write the Markdown to FILE
quadcam-cli --json gear preflight                                # the Pack up check
quadcam-cli --json gear packs [save L --type T --charged | delete L | type save T --cells N --capacity MAH ... | type delete T | notes TEXT]
quadcam-cli --json gear crashes [--clip ID save --time S --broke TEXT --parts a,b | delete ID]
quadcam-cli --json gear backup [--device ID | --port P | --mount M] [show ID [PATH] | diff A [B] [--path P] | pin ID [--off]]
quadcam-cli --json gear backups [--device ID]
quadcam-cli --json gear storage [--prune [--dry-run]] [--export <backup|device> DIR]
quadcam-cli --json gear import-backups FOLDER [--device ID] [--dry-run]
quadcam-cli --json gear card [--device ID | --mount M] [--model model01.yml]   # an EdgeTX card: version, models, selected model, radio clock
quadcam-cli --json gear card preview [--device ID | --mount M] --edits FILE.json   # the checks and diff of card edits; writes nothing
quadcam-cli --json gear card-check [--device ID | --mount M] [--log]
quadcam-cli --json gear card-repair --check <check id> --yes
quadcam-cli --json gear card-clean [--device ID | --mount M] [--remove --yes]   # list or delete the ._ files macOS left on a card
quadcam-cli --json gear radio-cli identify|ls|play|beep|reboot|verify [--port P] [--path P] [--device ID] [--yes]   # a radio on its USB serial port (EdgeTX CLI)
quadcam-cli --json gear dfu-link [--device ID] [--serial S] [--unlink]   # link the radio in DFU mode to a saved radio
quadcam-cli --json gear stage --device ID --set NAME=VALUE [--pid-profile N | --rate-profile N] [--title T] [--note N] [--draft]   # stage; writes nothing
quadcam-cli --json gear stage --device ID --cli FILE   # raw CLI lines, no save/exit/defaults
quadcam-cli --json gear stage --device RADIO_ID --edits FILE.json   # card edits (JSON array)
quadcam-cli --json gear changes [--device ID] [--status ready] [--history]
quadcam-cli --json gear update CHANGE --status draft|ready|try|read_first [--title T] [--note N] [--order N]
quadcam-cli --json gear apply CHANGE --plan            # every check, the diff, the digest; no reboot. A Draft or Read first change refuses (draft, read_first)
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
`pull` needs no `--yes`, even when it erases; MCP `blackbox_pull` needs `confirm=true` (see MCP equivalents).
`--mode auto` (the default) uses the FC's USB disk only when the `gear_blackbox_msc` setting is on and the FC has one, else MSP; `msp` and `msc` force one path. It refuses a pull that would outlast the USB heat timer (`--force` overrides that, never an erase that could not
finish) and a port another program has open. `summary` reads the flash's size, used bytes and
ready state over one MSP read, and says whether a stored pull holds exactly that; it pulls nothing
and plays no cue. `list` shows the stored pulls newest first, each log paired with a
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
- **Values only widen.** A new value for a flag that already lists values is an addition. A value list on a flag that took any value narrows it, and fails like a removal.
- `tests/surface/cli.txt` records every command, flag, argument and listed value. A test compares it with the built tool, so a removal, a rename or a narrowing fails the build.

### MCP equivalents

Every MCP tool has a command. The command line also has `scan`, `analyze`, `clear`, `moments`, `modules manifest`, `gear fc check` and `gear voice build-pack`. An MCP `gear` action maps to a `gear` command of a similar name: `blackbox_pull` is `gear blackbox pull`, `voice_render` is `gear voice render`, and `card_mount` is `gear card-mount`. `gear --help` lists them. The exceptions:

| MCP | Command |
|---|---|
| `quadcam_status` | `status` (the card list alone: `cards`; the session alone: `show`) |
| `quadcam_load_clips` | `stage`, then `analyze` and `dates` |
| `quadcam_suggest` | `import` flags (`--name`, `--note`, `--date`, `--cut`, ...), `meta`, `cut`, `dates`; `on_device`: `names` |
| `quadcam_settings` `read`, `write` | `settings show`, `settings set` (a secret: `settings set-secret`) |
| `quadcam_gear_edit` `voice_key_set`, `voice_key_delete` | `gear voice key set`, `gear voice key delete` |
| none | `gear voice publish --approve`: only a person at a terminal approves a publish |
| `quadcam_profiles` `set_default` | `profiles default` |
| `quadcam_gear` `switch_map` | `gear map` |
| `quadcam_gear` `session_report` | `gear report` |
| `quadcam_gear` `board_notes`, `usb_timers` | `gear fc notes`, `gear fc usb` |
| `quadcam_gear` `firmware_copies`, `firmware_restore_plan`; `quadcam_gear_apply` `firmware_restore` | `gear firmware --copies`, `gear firmware --restore COPY --plan`, the same with `--digest D --yes` |
| `quadcam_gear_edit` `poll_pause` | `gear fc pause`, `gear fc resume` |
| `quadcam_gear` `*_plan` | the same command without `--yes` or `--digest` (or with `--plan`) |
| `quadcam_gear_apply` | the same command with `--digest D --yes` (MCP `confirm=true`) |
| `quadcam_library_files` `cuts`, `split_by_flight`, `export_cuts` | `library cut` (`--by-flight`, `--export`) |
| `quadcam_library_edit` `shift` | `library shift` (`--yes` is `confirm`) |

Flags that mean one thing everywhere: `--yes` is MCP `confirm`, `--plan` shows the checks and digest and writes nothing, `--device` is a saved device id, `--mount` is a mount point, `--profile` is an aircraft profile. `gear voice` keeps `--confirm`: it allows a voice provider that may charge, which is a different consent. `gear blackbox pull` takes no `--yes`, while MCP `blackbox_pull` needs `confirm=true`. When the `gear_erase_blackbox` setting is on, `pull` erases the flash after a verified pull: the setting is the consent, as `delete_clips_after_import` is for `import`, and `--keep` skips the erase for one run. A new `--yes` requirement would break schema 1 scripts, so the command keeps working without it. `gear voice delete` takes `--yes` like any other write.

## MCP server

`quadcam-cli mcp` runs an MCP server. See [MCP server](mcp.md).
