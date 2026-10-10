# Gear

Gear is QuadCam's second half: the FPV bench next to the clip library. This page covers what
works today. The full plan is in [Gear design](gear-design.md).

## In the app

**Gear** is a section of the sidebar, under the library groups. Its triangle opens and closes
it.

- **Connected** lists what is plugged in now. Each device also has its own row under it.
- **Devices** lists the devices you saved, plugged in or not.
- **Bench** lists the changes staged for each device. See [The Bench](#the-bench).
- **Pack up**, **Flights**, **Packs** and **Repairs** are described under
  [Flights and packs](#flights-and-packs).
- **Sims** lists the sims on this Mac against one quad's rates. The item says **Out of date**
  while a sim's rates differ from the quad's. See [Sims](#sims-page).
- **Firmware** compares each device's firmware with the newest release and flashes an EdgeTX
  radio. See [Firmware and splash](#firmware-and-splash).
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
generic one, so QuadCam does not use it as an id. It identifies the radio by its card.

**One radio, two ids.** A radio card in the built-in SD slot gets its id from the card's
hardware serial, which the radio's USB Storage mode does not show. There the id comes from the
volume UUID. When QuadCam sees a saved radio's card, it also records the other ids the card
answers to (`aliases` in `gear.json`). The same card, plugged in through the radio, is then the
same saved radio, with its name, aircraft and backups.

**DFU link.** In DFU mode the bootloader reports the chip's own serial number, which is a
stable id, but no name. QuadCam cannot match it to the storage-mode radio by itself, so the
serial is linked to a saved radio once and kept in that radio's record:

- `gear dfu-link --device RADIO` links the radio now in DFU mode to the radio you name.
  Without `--device` QuadCam takes the saved radio seen most recently and says so. A serial
  belongs to one radio; linking it to another moves it. `--unlink` removes it.
- A flash plan checks the link. A DFU device linked to another radio, or a radio linked to
  another DFU device, refuses ("The radio in DFU mode is this radio"). An unlinked DFU device
  passes with a warning that names the radio seen most recently; a verified flash then links
  it to the radio you flashed.
- Once linked, the DFU device shows in the device list under the radio's name.

## Flight controllers

QuadCam talks to Betaflight over USB in two ways:

- **MSP** (identify): board, firmware, version, the device id. No reboot.
- **The CLI** (read): `version`, `status`, `diff all`, `dump all`, `get NAME`. When the read
  ends, QuadCam leaves the CLI and the FC reboots, as it does in the Betaflight Configurator.

Every job opens the port, works, and closes it. QuadCam never keeps an FC port open between
jobs, so another app can use it, and the FC can be unplugged as soon as the job says "done".

**Sharing the port with another tool.** Two programs talking MSP and the CLI on one port garble
each other: QuadCam's MSP frames (`$M<`) land in the other tool's CLI replies. So QuadCam opens
a port only when no other program has it open:

- Before each open it asks macOS which processes have the port's device nodes open
  (`/dev/cu.*` and its `/dev/tty.*` twin; the same data `lsof` shows, read in place). If one
  does, QuadCam does not open the port. A job you start says "port busy" and names the
  program. A background read (the USB timer's battery probe, the on-connect backup) skips quietly
  and tries again later.
- QuadCam opens the port exclusively (`TIOCEXCL` and a file lock), so a tool that starts in the
  middle of a read gets "busy" instead of sharing it, and closes it again within a second.
- QuadCam's own background reads are the USB timer's probe (every 30 s), the on-connect FC
  backup and a live switch read. Each opens, reads, and closes.
- The check sees programs of your own user account. If a tool runs as another user, or you want
  no chance of a probe at all, pause the reads: **Pause reads** on the FC's Overview, `gear fc
  pause` in the CLI, or `poll_pause` in `quadcam_gear_edit`. The USB timer stops counting while
  paused, and the pause lasts until QuadCam quits. A job you start yourself still runs.

Plug USB in before the battery: many FCs do not show up on USB when the battery is first.
When several FCs are plugged in, name the port; QuadCam does not guess.

**Writes.** QuadCam writes an FC only through a staged change that you apply. It writes
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

## Blackbox

An FC with a flash chip logs its flights to it. When the flash is full, Betaflight stops logging, so
the next flights leave no log. QuadCam pulls the logs off the flash and, if you allow it, erases
the flash afterwards.

**Pull.** **Pull blackbox** on the FC's **Blackbox** segment (also `gear blackbox pull`, and the
on-connect step below) does this in one job:

1. Reads the flash summary over MSP: size, bytes used, ready.
2. Reads only the used bytes, 4 KB at a time, with the compression flag off. MSP gives about
   84 KB/s, so a full 16 MB flash takes about 3.3 minutes. A chunk that fails is asked again
   twice; a plug pulled mid-read fails the job.
3. Checks the image. Its size must equal the used bytes, it must start with a log header, and
   each log needs a firmware line. QuadCam counts the logs and reads the firmware, the craft
   name and the log date from the headers.
4. Stores the image in the gear folder's blob store and reads it back. A record in
   `<gear>/blackbox/<device>/` ties it to the FC, its aircraft and the day, and names the logs.
   A pull of the same bytes as the last one adds no second record.
5. Erases the flash only if all of the above passed and the **Erase blackbox after download**
   setting (`gear_erase_blackbox`) is on. It waits until the flash reads ready with 0 bytes used.
   A flash that grew since the read is not erased.

"Done, safe to unplug." plays after step 5 ends, not before. If the erase does not finish, the job
fails, the pull stays stored, and the record says why.

`gear_erase_blackbox` is off by default, like `delete_clips_after_import`. The CLI and the MCP tools
only respect it: `--keep` (CLI) and `keep` (MCP) skip the erase for one run, and nothing turns it on
except the setting. **Erase flash** (`gear blackbox erase --yes`, `blackbox_erase`) erases by hand.
It needs a stored pull of exactly what the flash holds (the same used size; flash only grows), and
asks first in the app.

**USB heat.** A quad on USB with its battery in heats up. When the timer (see
[Flight controllers](#flight-controllers)) shows less time left than the read needs, the pull is
refused ("USB heat") and reads nothing; unplug the battery, or force the pull. A forced pull still
never starts an erase it could not finish: the erase estimate (4 s per MiB, 20 to 120 s) plus 10 s
must fit in the time left, else the erase is skipped and the answer says so. With no battery in,
there is no timer.

**Other programs and Pause reads.** A pull refuses a port another program has open. The on-connect
step skips a port whose reads you paused; a pull you start yourself still runs.

**Steps on connect.** `blackbox` is a step in `gear_on_connect` for FCs. It is off until you tick
**Pull blackbox** for FC in Settings > Gear (or list `blackbox` in `gear_on_connect.fc`).

**Dates and flights.** A flight controller has no clock, so its logs read `0000-01-01`. QuadCam
dates a pull by the day it ran. If the FC is linked to an aircraft (Overview, or `gear devices
save --aircraft`), the Blackbox segment pairs the pull's logs with that aircraft's flights from the
radio logs **by order**, newest log with newest flight, since the previous erased pull. This is a
guess and the segment says so. Logs under 32 KB (test arms) are skipped. Pairs whose size per
second of flight differ from the others by more than 2 times are marked "size does not fit". QuadCam
does not decode the flight data yet.

**USB disk mode (not proven).** With `gear_blackbox_msc` on, a pull first tries the FC's USB mass
storage mode: it enters the CLI, checks that `help` lists `msc`, sends `msc`, copies the `.bbl` files
from the new disk in name order, unmounts it, and waits for the FC to come back. It falls back to
MSP when the FC has no `msc`. This path is built and tested on a simulated FC only. **It needs a trial
on a real FC**: the disk's file layout, the speed, and whether the FC returns to serial after the
eject are unknown. The estimate for the heat check stays the MSP one.

**Export.** `gear blackbox export <id> DIR [--split]` writes the image as `.bbl`, and with `--split`
each log as its own file. It never overwrites.

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

QuadCam writes the edits through a staged change. See
[Radio cards](#radio-cards).

**Over the radio's USB, writes are slow** (about 0.3 MB/s, a 37 MB voice pack in about 2
minutes). QuadCam writes one file at a time under a temporary name and renames it, so the
card is always whole. Cancel finishes the current file, then unmounts the card, then says
"safe to unplug". Do not pull the radio while a file is being written: once, that wedged the
Mac's disk service until a reboot. If macOS stops answering, QuadCam says a reboot may be
needed instead of waiting forever.

While a write runs over the radio's USB, the apply sheet says to keep the radio plugged in.
QuadCam never asks you to unplug it before the unmount. A mount or unmount that macOS does not
finish within its time limit (60 seconds for an unmount) ends with "macOS did not finish ...
a reboot may be needed" and leaves the radio plugged in.

If the radio's firmware stops at an error, hold both horizontal trims inward while you power
it on. The bootloader then shows the SD card over USB.

### Clean ._ files

macOS writes hidden `._*` files (AppleDouble) next to files it copies to a FAT card. The radio
does not use them. QuadCam removes the `._` file beside every file it writes, and does not
create the others. For files that are already there, **Clean ._ files** on the radio's
**Backups** segment lists them, asks, then deletes them and unmounts the card. It deletes only
files that start with the AppleDouble header; a file you named `._notes` stays. On the command
line: `gear card-clean --device ID` lists, `--remove --yes` deletes.

### Radio over USB Serial

With the radio's USB serial port set to **CLI** (the hardware settings, `serialPort: VCP`),
the radio shows as a serial port (`<Radio> Serial Port`, 115200 baud, prompt `>`). It is a
radio, not a flight controller. The CLI cannot move file contents, so QuadCam uses it for small
jobs only. `gear radio-cli ACTION` runs one:

| Action | What it does |
|---|---|
| `identify` | `ver`: the board and EdgeTX version the radio runs, and the saved radios of that board |
| `ls --path /SOUNDS/en` | lists a card folder |
| `play --path /SOUNDS/en/hello.wav` | plays a sound file on the radio's speaker. A voice line's card path works as it is, so after a voice pack is applied you can hear a line on the radio itself |
| `beep` | the radio beeps |
| `reboot --yes` | restarts the radio |
| `verify [--device ID]` | `ls` each folder of the saved radio's latest backup, and lists the files the radio lacks or holds at another size. Use it after a card apply, with the card back in the radio. `LOGS/` is skipped |

QuadCam sends no other command. A path is an absolute card path of plain characters. QuadCam
refuses a port another program has open ("is open in screen"), as it does for a flight
controller, and a radio whose CLI is off gets a hint to turn it on. After `identify`, the
serial radio shows its board and version in the device list.

## Radio model editors

A saved radio's page has a **Models** and a **Checklists** segment. They read the model from the
mounted card, or from the radio's latest backup when it is not plugged in, and show the
radio's staged model edits on top. Every control stages its change at once. All edits join
one change per radio, "Model edits". A later edit of a setting replaces the earlier one,
and an edit that puts the model back as the radio has it drops out. Nothing reaches the card
until the apply sheet writes it. **Review…** opens the sheet; **Undo model edits** discards
the change.

The **Model** pop-up picks the model file. The first load shows the radio's selected model.

| Section | What it edits |
|---|---|
| Timers | Name (8 characters), mode, switch, countdown beep, persistent, beep every minute, count up. Add a timer (3 at most) or remove one. A timer's stored time is never set |
| Telemetry screens | A value screen holds up to 4 lines of up to 3 sources: a sensor label in braces, `{RxBt}`, or `Tmr1`. A script screen names a script (6 characters). Screens run in order: a screen cannot follow a gap |
| Logging | Whether the radio writes a log, on which switch, and how often (0.1 to 25.5 s). Which telemetry sensors the log records |
| Alarms | The RSSI warning and critical levels. Critical cannot be over warning |
| Callouts | A sound that plays while a switch is on, or while a sensor is below or above a value for a delay. Repeat: once, once but not at power-on, or every few seconds |
| Checklist | The power-on checklist, in the **Checklists** segment |

**A callout belongs to its sound.** The sound's name (8 characters at most, a file in
`SOUNDS/<language>/` on the card) identifies the callout. Staging a callout with a sound that
already has one replaces it, and the logical switch QuadCam made for it changes with it. A
logical switch that anything else uses stays as it is. A battery callout uses the first free
logical switch. QuadCam can edit only the callouts whose condition it writes; any other
callout shows in the list and can be removed.

A sensor's value is typed as the sensor shows it, `3.5` for 3.5 V, and stored in the
sensor's precision. A sensor must exist in the model; if it does not, discover sensors on
the radio first.

**Checklist.** Each item is a line; a tick box starts the line with `=`. A line holds 20
characters on the RadioMaster Pocket (the `=` counts), and a checklist holds 99 lines.
QuadCam writes `MODELS/<model name>.txt` and turns on `displayChecklist` and
`checklistInteractive`. Staging a checklist turns it on; the checkbox turns it on or off.

**Not yet checked on a real radio:** the logging function (`LOGS`, its period in 0.1 s), the
value screens' limit of 4 lines and 3 sources, the list of timer modes the pop-up offers
and the 99-line checklist limit (QuadCam's own cap). Each follows the shape of the
functions and files a real card does hold. Check each on a real card before relying on it.

## Radio voice

A saved radio's **Voice** segment chooses the voice its callouts and system sounds use. The
radio plays WAV files from `SOUNDS/<language>/` on its card. QuadCam holds a list of lines, the
sound a radio plays and the text a voice reads, and puts a voice's takes of them on the card.

- **Lines.** Each line has a card path, its text and a group (callouts, numbers, system,
  units, extras). A few words read badly, so QuadCam spells them out before a voice speaks them:
  `GPS` is spoken `G.P.S.`, `TX` `T.X.`, `dBm` `D.B.M.`. The table shows the spoken text
  beside the text. Your own lines live on your Mac, never in a pack.
- **Packs.** A voice pack is a zip of WAVs for one voice. **Refresh packs** reads the pack
  index (the `voice_index` setting: an address or a file) and **Install** unpacks a pack into the
  gear folder after a hash check. No pack ships with QuadCam yet.
- **Render my voice.** QuadCam can also speak every line itself, with the provider in the
  settings, and keep the takes as a local pack: `tts_provider` is `say` (macOS, free, offline)
  or `openai` (any server with an OpenAI-compatible `/v1/audio/speech`, such as a Kokoro
  server on this Mac: set `tts_base_url`, `tts_model` and `tts_voice`). A key goes in
  `QUADCAM_TTS_KEY` or the `tts_key` setting and is never shown or logged. **Check cost**
  reports the lines, how many the cache already holds, and the characters a provider would
  speak. A provider that is not on this Mac may charge: **Render my voice** then stops and
  shows the characters, and **Render and pay** goes ahead.
- **The cache.** A take is kept by provider, voice, model, speed, spoken text and seed. A
  render with other trim or tempo settings finds the take and calls no provider.
- **The shape of a sound.** Each WAV is 32 kHz, 16-bit, mono. QuadCam trims the silence
  (speech is every 5 ms window within 55 dB of the loudest), adds 20 ms before and 150 ms
  after, and fades the speech in over 5 ms and out over 30 ms. A tempo other than 1 runs
  first, in ffmpeg.
- **Choose voice.** Pick an installed voice and **Choose voice** stages one change that
  puts every sound of that pack on the card. **Keep my overrides** (on by default) keeps the
  lines you changed one by one. Sounds on the card that no pack knows are never deleted.
  **Review…** opens the apply sheet: backup first, write, read back, compare. Over the radio's
  USB a whole pack takes minutes.
- **One line.** **Use another voice…** takes another installed voice's take for that line
  on this radio. **My text…** speaks your own text with your provider for that line only.
  **Reset** clears it. The play buttons play a take. A line you add this way that QuadCam does
  not list becomes one of your own lines.

### Voice studio

The **Voice studio** at the top of the Voice segment renders whole line sets with ElevenLabs,
inside carrier sentences, and compares voices before it spends credits.

- **Key.** Paste the ElevenLabs API key and **Save key**. It goes to the macOS Keychain
  (service `app.quadcam`) and nowhere else: not `settings.json`, not a log, not an answer.
  The studio shows only the last four characters. **Remove key** deletes it. From the command
  line, `quadcam-cli gear voice key set` reads the key from stdin. `QUADCAM_TTS_KEY` in the
  environment takes the place of the Keychain for one run.
- **Account.** The studio lists the account's voices and models and the credits left. Each
  model shows its price in USD per 1,000 characters and the credits a character costs. QuadCam
  holds one rate table (the pricing page of 2026-10-09). A promo rate, such as the one on v4
  and v4 turbo until 2026-10-12, shows with its last day. The credit figure is an estimate:
  one credit is one character at the $0.08 rate, so a $0.04 model costs half a credit. After
  a paid call, QuadCam records the character count ElevenLabs reports and uses that next time.
- **Line sets.** A set is a list of lines to render. Sets combine, and a line in two sets is
  rendered once. Every line has a card path, its text, a tone and the sets that hold it.
  Four tones batch separately, so a calm word never picks up a warning's voice: `calm`,
  `alert` (warnings), `number` (numbers and units) and `fun` (easter eggs).

  | Set | Lines | Holds |
  |---|---|---|
  | `edgetx` | 747 | Every English prompt EdgeTX 2.x plays: the radio's own 147 prompts and number words, 65 units, 356 model prompts, and 179 telemetry-script prompts |
  | `quad` | 396 | The radio's prompts and units, the EdgeTX prompts a quad plays (modes, rates, VTX, OSD, turtle, rescue, navigation), and QuadCam's quad callouts |
  | `heli` | 325 | The same base, the helicopter prompts (idle up, governor, autorotation, engine, glow), and QuadCam's callouts |
  | `plane` | 368 | The same base, the plane prompts (gear, flaps, spoilers, brakes, reverse thrust, drag brake), and QuadCam's callouts |
  | `glider` | 328 | The same base, the glider prompts (thermal, launch, crow, butterfly, snapflap), and QuadCam's callouts |
  | `car` | 308 | The same base and the car prompts (drive modes, steering, gears, drift, stability control) |
  | `scripts` | 179 | The Betaflight, INAV and Yaapu telemetry-script prompts |
  | `extras` | 164 | More FPV callouts: arming states, battery stages (ninety to ten percent), link stages, rate profiles (freestyle, race, cinematic), VTX power, beeper, race timing, GPS rescue, checks |
  | `easter` | 20 | Short fun lines in original wording. Off unless you pick the set |
  | `sample` | 12 | Hard lines for comparing voices |
  | `custom`, `quadcam` | yours, 45 | Your own lines, and the lines a pack holds today |

  The number and unit prompts (`0000.wav` to `0112.wav`, `volt0.wav` and so on) and the
  sounds the radio plays by itself go in `SOUNDS/en/SYSTEM/`, where EdgeTX looks for them; a
  model's own tracks go in `SOUNDS/en/`; a script's prompts go in
  `SOUNDS/en/SCRIPTS/<script>/`. A voice reads some lines differently from the written
  text (for example `P term` for the Betaflight script's `P`), and the spelling rules still
  apply. The set files are `src-tauri/resources/voice/sets/*.csv`; the `kinds` column of
  `edgetx.csv` and `scripts.csv` names the aircraft that play a line.
- **Cost.** The line under the pickers shows the characters (carriers included), the batches,
  the price in USD and the credits (estimated), and the credits left. Batches the cache already
  holds cost nothing. A render the credits do not cover cannot start. A batch stays under the
  model's per-request limit (v3 5,000 characters, multilingual v2 10,000, flash and turbo
  40,000).
- **Sample.** Tick voices and models and press **Sample**. QuadCam renders the sample lines
  in each pair, shows the cost, and waits for **Sample and pay**. Each play button in the grid
  plays one line in one voice and model.
- **Render pack.** Needs one voice, one model and at least one set. After **Render and pay**
  the lines go into a local pack named for the voice and model; a second set adds to the
  same pack. **Choose voice** puts it on a radio like any other pack.
- **How a line is cut.** A bare word sounds wrong from a voice, so each line is spoken in a
  carrier ("The word is six.") and cut out. Lines are grouped by tone (calm, alert, number,
  fun), up to 30 sentences in a batch. ElevenLabs returns the time of every character; QuadCam
  cuts from the start of the line's first character to the end of its last, moving each edge
  to the quietest point within 40 ms. The cut then gets the usual trim, fades and tempo.
  A cut that is silent, under 120 ms, or long for its text is reported. The raw audio and
  timestamps of each batch are kept by provider, voice, model, speed, text and seed, so a new
  trim or tempo setting re-cuts for free. `--carrier "I said {line}."` changes the carrier.

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
`backup`, `import`, `apply_ready` and `blackbox` (FCs only; see [Blackbox](#blackbox)). Only `backup` is on by default, and only while
`gear_auto_backup` is on. `backup` runs two steps: **Card check** (a card QuadCam knows) and
**Backup** (a radio card or an FC). `import` is accepted in the setting and does nothing yet. `apply_ready` is described
under [Apply on connect](#apply-on-connect).

A card is unmounted (`diskutil unmountDisk`) at the end of every job: the on-connect steps, a
backup, a card check, a repair, an apply. "Done, safe to unplug" plays only after the unmount worked.
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
  stay. The backup read back after an apply is thinned like a plug-in backup. Of the rest, QuadCam keeps the newest `gear_keep_recent` (10), then one a week for
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

## Staged changes and apply

A change to a flight controller goes through five steps: **stage** it, **review** it, **check**
it, **confirm** it, then **apply** it. Staging writes nothing to the FC. Only the apply
does, and only after the plan's checks pass.

**Stage a setting.** On an FC's page open **Changes**, then **Edit setting…**. Give a setting
name (`osd_cap_alarm`), its value, and, for a profile setting, the PID or rate profile. QuadCam
refuses a name that the FC's latest backup does not hold, and any line it never sends
(`save`, `exit`, `defaults`, `batch`, `bl`, `dfu`: QuadCam saves and exits itself). The change
waits in the gear folder (`changes/`) until you apply or discard it. **Discard** keeps it in
the history.

**Review.** A device with staged changes shows a bar on its Overview: "1 change ready" and
**Review…**. Nothing applies on its own. **Review…** opens the apply sheet:

- **Changes** shows the lines that would be sent, with the old value removed and the new one
  added.
- **Checks** shows each guard with a pass mark or the reason it fails. **Apply** stays off while
  one fails. Return does not press it.
- **Apply** backs the FC up (QuadCam keeps this backup), sends the lines, saves, waits for the
  FC to restart, reads `dump all` and checks that every line reads back as written. A `set`
  equal to its default is checked too.

The checks:

| Check | Fails when |
|---|---|
| One FC plugged in | No FC is plugged in, or several are and QuadCam cannot tell which one the change is for |
| Same FC as planned | The FC on the port is not the one the change was staged for |
| Known board and version | QuadCam has not proven this board and Betaflight version for writing |
| Backup to compare with | The FC has no backup yet; the plan compares with the latest one |
| Lines understood | A line is one QuadCam never sends, or has no name or value |
| Settings exist | A name is not in the FC's `dump all`, or is a profile setting with no profile |
| Port free | Another program (a configurator, a terminal) has the port open |
| USB heat | The FC has run on USB with its battery in past its limit ([USB timer](#flight-controllers)) |

Two more guards run when you press **Apply**, because only then does QuadCam read the FC: the
backup must succeed, and the FC must still hold what the plan compared with (otherwise "The FC
changed since the plan; plan again."). Each `set` is also checked with the FC's own `get`: a
value outside the allowed range or list is refused before any line is sent.

**If it fails.** A line the FC refuses stops the apply: QuadCam sends `exit`, nothing is saved,
and the FC is as it was. The sheet names the line. If the FC saves but a line does not read back
as written, the sheet lists those lines and offers **Restore backup**, which stages the backup
taken before the write as a new change and opens it in the same sheet. A restore sets back
every setting, mode, adjustment and feature that differs; it leaves resources, serial ports
and timers alone.

After an apply QuadCam stores the FC's new state as a backup (**After apply**), so the next
plan compares with it. A profile selection changes the FC's active profile once saved, so
QuadCam selects the profile it needs, then selects the one the FC had.

### Radio cards

A radio's changes use the same five steps. Stage them from the command line or an agent
(an edits file, see [Command line and agents](#command-line-and-agents)); a radio's **Changes**
segment lists them and **Review…** opens the same sheet. The checks:

| Check | Fails when |
|---|---|
| One card plugged in | No radio card is mounted or still plugged in |
| Same card as planned | The card is not the one the change was staged for (its id) |
| Card check | The card's last file-system check failed. Repair it first |
| Nothing else writing | A backup or another job is running on the card |
| Known version | QuadCam has not proven this board and EdgeTX version for writing |
| Shape understood, Round trip, Model identity, Values in range | A file is not one QuadCam rewrites unchanged, or an edit names another model than the file holds ("wrong card?"), or a value is out of range |
| Selected model kept | The edits would change the radio's selected model without asking |
| Something to write | The card already holds what the edit asks for |
| Read first | The change is marked Read first |

**Apply** backs the card up first (a full snapshot, **Before apply**, which QuadCam always
keeps), then writes one file at a time under a temporary name, reads each back and compares it,
then reads every file again. If a file reads back different, or a write fails, QuadCam puts
every file back to the bytes in the backup, and the sheet says the card is as it was. A new file
is removed. Over the radio's USB a write is slow and **Stop** finishes the current file, then
puts the written files back.

**Mount, work, unmount.** QuadCam keeps a card unmounted between jobs, for every kind of
card: a radio's, a DVR card and a goggles card. Any job that needs a card mounts it when it is
unmounted but still in, does its work, and unmounts it again:

| Job | Ends with |
|---|---|
| Backup, card check, repair, apply, restore, voice pack, `._` clean-up | The unmount, and "safe to unplug" only after it worked |
| A plan, a card view, a preview, a list of `._` files, a format plan, a card prep plan | A quiet unmount, with no cue |
| Importing clips (the card is mounted for the stage) | The unmount when the import is done, after "Delete clips after import". Finish says "Unmounted. Safe to remove." |
| Format and card prep | The erase unmounts the card. A refused erase unmounts it too |

A card that is mounted already stays as it is for a plan, a view or a preview. A DJI device
over USB (an air unit) is never unmounted this way. If an unmount fails, the sheet says so and
the cue is "failed". Lists such as Connected and the Pack up check never mount a card: they read
what is mounted, or the latest backup.

**Mount** (on a radio's, a DVR card's or a goggles card's page, when the card is unmounted but
still in) mounts the card so you can browse it in Finder. QuadCam unmounts it again when you
press **Done**, or after 10 minutes. **Import clips…** on a DVR or goggles card's page mounts it
and starts an import. **Prepare card…** opens [card prep](format-safety.md#card-prep).

A restore of a radio backup names the files to put back (`gear restore <backup> --path
RADIO/radio.yml`). It makes those files read as in the backup, and removes a file the backup
does not hold.

### The Bench

**Bench** (in the sidebar, with a count) lists every device that has a waiting change, in
the order the next session applies them.

- Each device shows whether it is plugged in, unmounted but still in, or away. A device that is
  away shows what to do: "Plug in the radio in USB Storage mode to apply 3 changes." **Next
  session** names the first Ready or Try change.
- Each change has a status you set from its list:

  | Status | Means |
  |---|---|
  | Draft | Being edited. The plug-in bar and **Review** skip it |
  | Ready | Agreed. Apply it when the device is in |
  | Try | Apply, fly, then keep or revert |
  | Read first | Read the real value on the device before changing anything. It does not apply until you mark it Ready |
  | Applied | A Try change that applied and verified. It waits for **Keep** or **Revert…** |
  | Verified, Failed, Reverted, Discarded | After the apply sheet, or Discard |

- **Keep** makes an applied Try change Verified. **Revert…** stages the undo and opens it in the
  sheet. For an FC the undo sets back only the lines that change set, to their values from just
  before its apply; settings a later change set stay as they are. When a later applied change
  set the same line, the revert's note says so, because the revert undoes that value too. For a
  card it restores the files the apply wrote. The change becomes Reverted when the undo
  verifies.
- **History** lists changes that are done.
- **Copy as Markdown** puts the queue on the clipboard.

### Copy settings between quads

**Copy settings…** (on the Bench, and on an FC's **Changes**) copies settings from one FC's
latest backup to another FC. Pick the quad to copy from and the one that gets the settings, then
the parts (rates, PID profiles, OSD, modes, adjustments, VTX, features and beeper) and any
settings by name. QuadCam shows the checks and the diff. **Stage** queues one change on the
target. It writes nothing until you apply it.

| Check | Fails when |
|---|---|
| Same firmware | The two quads do not run the same firmware |
| Same release | The year and month of the versions differ (a patch difference is fine) |
| Something picked | No part and no setting is picked |
| Something differs | The target already holds everything picked |

QuadCam leaves out, and lists, what it should not copy:

- values each quad has of its own: the craft name, accelerometer trims, battery and current
  calibration;
- with two different boards, settings tied to the board's chips and buses (`gyro_*`, `acc_*`,
  `baro_*`, `mag_*`, `serial*`, SPI and I2C settings);
- a setting or line the target's backup does not hold.

### Apply on connect

**Settings > Gear > Steps on connect** can list `apply_ready` for a kind of device. It is off
by default. When the device is plugged in, QuadCam plans each Ready change of that device, and
a plan whose checks all pass opens the apply sheet. Nothing is written until you click
**Apply**. With no window to click in, the step does nothing.

### OSD element moves

An agent or the command line can move one OSD element: the edit `{"kind": "osd_element",
"element": "vbat", "x": 20, "y": 9, "profiles": [1, 3]}` becomes
`set osd_vbat_pos = <value>`, keeping the element's other bits. The OSD segment's editor stages
the same edit (see "Edit the OSD" below).

## Card check

Pulling a card while it is mounted can leave its file system damaged. Before QuadCam backs up
a card it knows (on connect), it runs `diskutil verifyVolume` on it: read-only, about 30
seconds over a radio's USB. The card unmounts and mounts again while it is checked. **Stop**
ends a check. The result shows on the device's **Backups** segment, in the import sheet's header
for a card QuadCam knows (an import never runs a check), and a failed check marks
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

### Edit the OSD

On a saved FC with a backup, the OSD segment is the editor. Opening a file turns it back into a
viewer. Nothing reaches the FC until you apply it from the apply sheet.

- **Move:** drag an element on the grid, or focus it (Tab) and press the arrow keys. Shift moves
  five cells. A drag stays on the grid; the keys and the **X** and **Y** boxes in the list reach
  any cell the position value holds (x 0-63, y 0-31), so you can see an off-screen warning.
- **Turn on or off:** tick the element in the list for the profile you are looking at. The list
  shows every element, the ones on in this profile first.
- **One position for all profiles:** moving an element moves it in every profile that shows it.
- **Copy a layout between profiles:** pick the two profiles and press **Copy profile**. The
  target then shows exactly the elements the source shows.
- **Copy from another quad:** **Copy from another quad…** opens Copy settings with OSD ticked.
  It copies every `osd_*` setting from that quad's latest backup, not only the positions.
- **Live check:** the grid, the problem list and the profile counts show the layout with your
  edits on top, so an overlap or an element off screen shows as you edit. A diff-only backup
  cannot be edited for elements it leaves out.
- **Staged as one change:** all edits go into one change named "OSD layout" on the FC's
  Changes segment. A later edit of an element replaces its earlier one, an element put back where
  the FC holds it drops out, and the change disappears when nothing differs. **Review…** opens
  the apply sheet, which shows the `set osd_<element>_pos` lines. **Undo OSD edits** discards the
  change.
- **Element labels:** each element draws as plain sample text, not Betaflight's font.

## Rates

The Rates segment shows how an FC's rate profiles turn stick movement into rotation, and how
each sim on this Mac compares. You can edit a profile and write the quad's rates into a sim;
both go through the apply sheet. Open a flight controller's page under **Gear > Devices** and
choose **Rates**. A saved FC with a backup shows its latest backup; **Open dump…** reads a
`dump all` or `diff all` file instead (a file is read only).

- **Profiles:** one button per rate profile, with its name (FREE, RACE, CINE). The profile the
  FC uses has an "In use" mark. The line under the buttons names the rates type: Betaflight,
  Actual, Quick, Raceflight or KISS.
- **Curves:** roll, pitch and yaw, in degrees per second against stick position from centre to
  full. The table lists RC rate, super rate, expo, the rate limit, the centre rate (degrees per
  second per full stick at the centre) and the maximum rate. The throttle chart shows mid, expo,
  hover and the throttle limit.
- **Compare with:** draws a second, dashed curve from another profile of the quad, the profile
  in use of another saved quad, or a sim's profile. The table gains a column with its maximum.
- **A diff alone:** a diff lists only values that differ from the defaults. QuadCam fills in
  Betaflight's defaults and says so. Use a dump for exact curves.
- **Sims:** QuadCam reads the rate files of Liftoff, Liftoff: Micro Drones, Uncrashed and The
  Zone, and lists each profile's maximum rates. Each sim says "Matches the quad" or "Out of date"
  against the selected profile, and shows the largest gap in degrees per second. A sim
  takes Betaflight rates, so a quad on Actual or Quick is fitted to the Betaflight model first.
  The fit stays within 6 % of the maximum rate for Actual profiles with a centre of 40 to 200
  degrees per second, a maximum of 300 to 1000 and expo up to 60. Outside that range the error
  grows, up to about 12 %. The fit error shows next to each profile it affects. Only Uncrashed has a throttle
  curve. Velocidrone is off: QuadCam needs a sample Velocidrone save to read its rate file, so
  the adapter ships disabled until one exists. A sim marked "Running" may rewrite its
  file when it quits.
- **Edit profile:** on a saved FC, **Edit profile** opens the profile's rates type, the three
  numbers of each axis (named for the type: RC rate, super rate and expo for Betaflight; center
  rate, max rate and expo for Actual and Quick), the rate limits, the throttle mid, expo and
  limit, and the name. The curves redraw as you type, with the profile as it is now dashed.
  **Stage** queues only the values you changed as one change; **Review and apply…** opens the
  apply sheet. Nothing is written until you click Apply there. **Convert to** fits the profile
  onto another rate model with the same fit the sims use (whole numbers within the model's
  range) and shows the largest gap from the old curve per axis; stage it like any edit.
  Changing **Rates type** alone keeps the numbers and changes what they mean.

### Sims page

**Gear > Sims** lists every sim on this Mac against one quad's profile in use. The quad is the
saved flight controller that QuadCam saw last and has a backup; **Compare with** picks another
when there are several. Each sim shows **Matches the quad** or **Out of date**, its files and
profiles with **Sync**, and **Restore backup** for a sim QuadCam backed up (the same sheets as
in the Rates segment). Velocidrone shows **Off** with the reason.

The sidebar item says **Out of date** while at least one enabled sim differs from that quad.
It clears after a sync or when the quad's rates change. `gear status` carries the count as
`sims_out_of_date` and the quad as `sims_quad`.

### Sync the quad's rates into a sim

**Sync** beside a sim profile opens the apply sheet on a plan to write the selected rate
profile of the quad into that sim profile. It overwrites the profile you pick; QuadCam does not
create sim profiles.

- **Sims:** Liftoff, Liftoff: Micro Drones (its file lives inside the game's app bundle),
  Uncrashed (one file per profile; the file name is the profile name) and The Zone.
- **The plan** lists the checks, the values that change per file (old line out, new line in),
  warnings and a digest. Checks: the game is not running, the file is understood, the file
  rewrites unchanged byte for byte, and it is writable. **A sim that runs is never written**
  ("Quit Liftoff first."); the Sync button is off while it runs, and the check runs again
  right before the write.
- **Warnings:** a quad on Actual or Quick is fitted to Betaflight first, with the largest gap;
  a sim without a throttle curve (every sim but Uncrashed) gets rates only; and an
  **Unverified** warning for every sim whose written file QuadCam has not yet seen the game
  load (see below). Check the game's rates screen after the first sync of each.
- **The write:** QuadCam backs up each file first (the gear folder, as device `sim-<id>`, always
  kept), writes it through a temporary file and a rename with the file's own permissions, reads it
  back and parses it. If one file fails, every file already written is put back. Only the values
  that differ change; the rest of the file stays as it was.
- **Throttle:** Uncrashed gets the quad's throttle mid and expo. A sim has no hover value, so
  the quad's `thr_hover` is not counted as a difference.

**What was checked against a real install.** The file shapes below were read from the files of
real installs of each game (reading only) and the adapters read them. Only Uncrashed's write
has also been seen loading in the game (by a script that writes the same bytes); for Liftoff,
Micro Drones and The Zone the plan says "Unverified". Deadband, input expo and channel maps
are not synced: they are outside what the sims' rate adapters cover.

| Sim | Shape |
|---|---|
| Liftoff, Micro Drones | `<rateProfiles><name>…</name><rates xsi:type="BetaFlight"><Roll><Rate>127</Rate><Expo>40</Expo><SuperExpo>72</SuperExpo></Roll>…` whole numbers, as the CLI stores them. Micro Drones wraps profiles in `<FlightRatesProfile>` |
| Uncrashed | Unreal GVAS, 12 little-endian floats after `FloatProperty`: super rate, RC rate, expo per axis, rates type (0 Betaflight), throttle mid, throttle expo, as fractions |
| The Zone | `[rate_profile_N]` with one `rates={ "roll": Vector3(rc, super, expo), …, "type": "betaflight" }` dictionary, as fractions |

### Restore a sim's backup

A sim that QuadCam backed up shows when, with **Restore backup** in the Rates segment's Sims
list. It opens the same apply sheet on a plan to put the file back as it was before a sync.

- **The backup:** the newest one that differs from the file now. A second restore therefore
  undoes the first. `gear sims --restore SIM --backup ID` picks one from `gear backups --device
  sim-<id>`.
- **The plan** lists the checks (a backup found and readable, the game is not running, the
  file is there and writable), the rates that change per profile, and a warning that changes
  made in the game or by a later sync are lost. A digest covers the backup and the file as it is
  now.
- **The write:** QuadCam backs up the file as it is now (kept), writes the backup's bytes
  through a temporary file and a rename, and reads them back byte for byte. A failure puts the
  file back. A sim that runs is never written.

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
  throttle low). Mix switches, `ADD`, `MUL` and `REPL` lines, inputs and limits all count.
- **Combinations:** some effects need two controls at once, such as a mode on a logical switch
  `SA down AND SB down`. For each position QuadCam also tries the other controls that feed the
  same channel, logical switch, special function or timer, up to 4 controls in a group. A
  position that does more with another control moved shows an extra line under it
  (`down with SB down`) with only what that adds. A control with only such effects is not
  reported as doing nothing, and the live match counts these lines. A group of more than 4
  controls is not tried, and a note says so.
- **Not mapped:** a logical switch on telemetry, a timer or sticky state cannot be worked out.
  The **Not mapped** list shows each with its condition (`RxBt > 3.5`), the controls it reads
  and what uses it. It lights no position. A control that only feeds one is not reported as
  doing nothing.
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
- **Channels:** CH1 to CH8 in µs, and the 24 buttons. In the radio's Classic joystick mode,
  CH9 to CH32 are buttons 1 to 24 (EdgeTX manual, USB Joystick). In Advanced mode the model
  sets which channels are buttons, so the numbering can differ.
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
share. **Save…** writes it to a Markdown file you pick.

Under the flights, **Range by place** charts the worst link quality (LQ) of each flight at
each place, oldest first. **Values** under a chart lists LQ and RSSI per flight.

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
| Radio model | The selected model belongs to an aircraft. A radio that is plugged in is read now; one that is not is read from its latest backup, and the row says "from backup" and its age |
| Card space | Each goggles or DVR card plugged in has 2 GB and 10 % free. A radio card that is not plugged in uses the space seen when its latest backup was taken, marked "from backup" |
| Backups | Each saved radio and FC was backed up in the last 14 days |
| Cards out | No card is still in the Mac |

A row QuadCam cannot judge (nothing plugged in, no backups kept yet) reads as unknown. Each radio
backup records the card's free space for this check.

**Repairs** lists crashes by aircraft. Log a crash from a clip: open the clip, choose the
**Flight** tab, then **Log crash…**. The time is the playhead; add what broke and the parts
used. The clip's Flight tab lists its crashes.

Packs, pack types, the pack set on each flight, added log folders and crashes are yours, in
`gear.json`. No file is written to a clip.

## Firmware and splash

> **Warning.** Flashing writes a radio's firmware. A wrong or interrupted flash can leave the
> radio unable to start until you flash it again. The chip's own USB bootloader is in ROM, so
> QuadCam cannot overwrite it, and a radio can be recovered over USB (see
> [Recovery](#if-a-flash-fails-or-the-radio-will-not-start)). **QuadCam's flash has not been
> tried on a real radio yet.** Read the firmware first (below), and keep the copies QuadCam
> makes.

### Check

**Gear > Firmware** lists each saved device with its installed version and the newest release:

| Product | Newest release comes from |
|---|---|
| EdgeTX | The EdgeTX releases on GitHub |
| Betaflight | The Betaflight releases on GitHub |
| ExpressLRS | The ExpressLRS release index |

- **Check for updates** reads the network. Without it, the page shows the last answer, or
  "Not checked yet." The setting `firmwareCheck` (**Settings > Gear**) is `manual` by default.
  With `daily`, the page checks when the last answer is a day old.
- A row says **Up to date**, **Update available** or **Unknown** (the device reports no
  version, or no check has run). A source that fails shows a notice; the others still show.
- The sidebar shows how many devices have an update.
- QuadCam checks Betaflight versions. It does not flash them. For ExpressLRS, see [ExpressLRS (preview)](#expresslrs-preview).
- **Flash 2.12.4…** appears for an EdgeTX radio that QuadCam has proven (see below).

### Read the firmware (the first trial)

**Firmware > Read firmware** reads the firmware that runs on a radio and saves a copy. It
cannot erase, write or restart the radio: the USB path it uses refuses every command except
status polls, aborts, the address pointer and uploads. Run it before any flash.

1. Turn the radio off.
2. Plug the radio into this Mac with the USB cable. Do not hold any buttons. (Holding both
   trim buttons starts the EdgeTX bootloader, which is a different mode.) Wait a few seconds.
3. Pick the radio in **Radio**, or **Other radio** to skip the version comparison.
4. Select **Read firmware**. The progress bar covers two reads of the whole 1 MB flash.

QuadCam reads the flash twice and keeps the copy only when both reads are equal. It saves the
copy, reads the saved file back and compares it, then compares the version the image names
(`edgetx-pocket-2.12.4 ...`) with the version it knows for the radio. The result says whether
they match. A mismatch is not an error; check the radio's About screen.

- The copy is a file pair under the gear folder, `firmware/<radio>/<time>-read.bin` and
  `.json`. It is not a card backup and does not appear in **Backups**.
- A flash that reads as blank, as all zeros (a protected chip) or differently on the two reads
  is refused with the reason.
- With no radio in DFU mode, the read says so and tells you the steps above.
- Command line: `quadcam-cli --json gear firmware --read [--device RADIO]`. MCP:
  `quadcam_gear` `firmware_read`.

### Flash an EdgeTX radio

QuadCam flashes only a board and an EdgeTX version listed in `compat.rs`. Today that is the
RadioMaster Pocket on EdgeTX 2.12. A splash needs a version it has checked: 2.12.4.
Anything else is refused with the reason.

1. Back up the radio's card (**Backups**). The flash plan refuses without a backup.
2. Open **Firmware > Flash…**, or make a splash first (below). The sheet shows the checks:
   the board and version, the firmware image, the splash markers, the card backup and exactly
   one radio in DFU mode.
3. Put the radio in DFU mode: turn it off and plug in the USB cable, without holding any
   button. The radio shows no name in DFU mode, so check that it is the radio you picked. Once
   the DFU device is linked to a saved radio (see [What QuadCam finds](#what-quadcam-finds)),
   the plan refuses a different radio.
4. Select **Apply**. QuadCam:
   - checks the chip has the flash size the board expects (1 MB for the Pocket);
   - reads the firmware the radio runs now twice, saves it as a copy and reads the saved file
     back. Nothing is erased until both reads are equal and the file matches. A blank flash
     (an earlier flash that stopped) has nothing to copy and may be flashed;
   - erases the sectors it needs and writes the image in 16 KB segments, reading each segment
     back before it writes the next;
   - reads the whole image back and compares every byte;
   - leaves DFU mode, and the radio restarts.
5. Connect the radio in USB Storage mode to see its version.

If a segment or the final read back differs, QuadCam does not start the image. The radio
stays in DFU mode: select Apply again, or unplug it, turn it off and plug it in again.

Where the image comes from:

- QuadCam downloads the release's firmware zip (`edgetx-firmware-vX.Y.Z.zip`) from EdgeTX on
  GitHub when you plan a flash. It bundles and redistributes no firmware. The download is kept
  in `~/Library/Caches/app.quadcam/firmware/edgetx/<version>/`.
- QuadCam checks the zip against the SHA-256 that the release lists. When the release lists
  none, QuadCam records the hash at the first download and shows it in the plan.
- The board's image is the one `.bin` in the zip named `<board>-<commit hash>.bin`
  (`pocket-def35ad.bin` in 2.12.4). None, or two, refuses.
- The image must be a full image: the bootloader's vector table first and the firmware's at
  `0x8000`, 400 to 1000 KB. A firmware-only file is refused, because writing it at the start
  of the flash would replace the bootloader.
- The image must name its own board and version (`edgetx-pocket-2.12.4 (...)`). An image of
  another release or board is refused.

### If a flash fails or the radio will not start

The STM32 chip has a bootloader in ROM that no flash can erase. A radio whose firmware is
missing or broken can still be put in DFU mode, and QuadCam can flash it again.

1. Turn the radio off. Unplug other radios.
2. Plug in the USB cable, with no button held. Wait a few seconds.
3. Open **Firmware > Flash…** and select **Apply**. A blank flash is allowed.
4. If QuadCam cannot do it, use a DFU tool on the copy in `firmware/<radio>/`. Another
   option is STM32CubeProgrammer with the release's `<board>-<hash>.bin` at address
   `0x08000000`.
5. If the radio shows the EdgeTX bootloader (it started with both trims held), you can copy a
   firmware file to its SD card from there, as the EdgeTX manual describes.

Where a failure can leave the radio, and what QuadCam does:

| Point | What can go wrong | What QuadCam does |
|---|---|---|
| The cable or the Mac drops during the read | Nothing on the radio changes | The read fails; nothing was erased |
| Two reads differ | A bad cable or port | No copy, no erase |
| The saved copy does not match | A failing disk | No erase |
| The chip is not the expected size | The wrong radio in DFU mode | Refuses before any command |
| The device moves another block size than 2,048 bytes | Blocks land at the wrong address | Refuses before the erase |
| Power or USB drops during the erase or the write | The radio has no valid firmware | The radio stays in DFU mode; flash again |
| A block does not program | A corrupt image | Caught by the segment read back; the image is not started |
| The final read back differs | A corrupt image | The image is not started |
| The image is not the board's or the version's | A wrong file | Refused in the plan |

### ExpressLRS (preview)

A preview for 1.1, off by default. Select **Show the ELRS tools** under **Firmware > ExpressLRS
(preview)**, or turn on **Settings > Gear > Show the ELRS tools**
(`quadcam-cli settings set elrs_preview=true`). **QuadCam has not tried any of this on a real
radio, FC or ExpressLRS device yet.** Every job refuses while the setting is off.

**Read.** An ExpressLRS device sits behind a saved radio (its internal module) or a saved FC
(its receiver). **Read the module in …** or **Read the receiver in …** hands the host's USB
port to the device and asks it over CRSF for its name, version, target and parameters:

| Host | What QuadCam does |
|---|---|
| Radio | The radio's USB serial port must be set to CLI (as for the radio CLI). QuadCam stops the pulses and starts `serialpassthrough rfmod 0 400000` |
| FC | QuadCam checks that the serial receiver is CRSF, not inverted and not half duplex, finds the UART with the serial receiver and starts `serialpassthrough <uart> 420000` |

The radio or FC **stays in passthrough** until you restart the radio or unplug the FC. A second
job needs that restart first. The version comes from a text the device lists in its parameters
(`ELRS 4.1.0 …`), else from its device info; the report says which. A device it cannot read a
version from is saved without one.

The read saves the device (a transmitter or receiver, named by its host and target) and its
options in the gear folder. It cannot read the binding phrase, and QuadCam never stores one in a
change.

**Options.** For a device that was read, **Options** lists packet rate, telemetry ratio, max power,
dynamic power, switch mode and model match, as far as the device offers them. **Stage changes**
queues one change; **Apply** opens the apply sheet like any change. The apply reads the device
again and refuses if an option moved since your read, keeps the parameters as a backup, writes each
option over CRSF, reads them back and compares. From a terminal:
`quadcam-cli gear elrs set <device> packet_rate=250Hz telemetry_ratio=1:16`, then
`gear apply`. A choice matches the device's text or the text before its `(`.

**Flash.** **Flash 4.1.0…** (the newest release the last check found) shows the plan:

- QuadCam downloads the official release (`firmware.zip` from the ExpressLRS artifactory, on your
  action) and unpacks it in the cache. ExpressLRS publishes no checksum, so the plan shows the
  SHA-256 QuadCam recorded; pass `--sha256` (CLI) or `sha256` (MCP) to check against a digest you
  trust, and a mismatch deletes the download.
- The device's target is found in the release by the name the device reports. QuadCam flashes only
  the unified ESP8285 receiver and ESP32 transmitter-module targets, on ExpressLRS 3 and newer, and
  refuses any other target, a name that matches none or several, and a version the target does not
  support.
- QuadCam configures the image: the device name, your binding phrase as the UID, the hardware
  layout and the WiFi delay (`elrs_wifi_interval`, 60 s by default). It reads the image back and
  checks that no other byte changed. **The plan shows only a fingerprint of the UID, never the
  phrase.** Set the phrase in the ELRS section (write-only) or with
  `settings set elrs_binding_phrase=…`; reads show `(set)`. The plan refuses without a phrase.
- The region (`elrs_region`, `FCC` or `LBT`) picks the image folder.
- The **esptool** module must be installed ([Modules](modules.md)). Nothing downloads except the
  release, on your action.
- Mixed majors (4.x on one end, 3.x on the other) do not link; the plan warns, and the page warns
  about a read pair. Flash the receiver first.

**Apply** puts the device in its bootloader (a radio: the module's boot pin held while it powers
up; an FC: the receiver restarts into its bootloader on a CRSF request, and its name must match),
reads the chip's current flash with esptool and keeps it as a backup (`firmware.bin`), writes with
`esptool write-flash` and reports success only when esptool prints that it verified the data.
Afterwards restart the radio or power-cycle the quad, then read the device again. ExpressLRS 4
wipes a receiver's Options page, so note your settings first. A flash that fails leaves the device
in its bootloader: power-cycle it and flash again.

Stock esptool does not have the `--passthrough` flag ExpressLRS's own tools add. QuadCam uses
`--before no-reset` after it has put the device in its bootloader. Whether that works through a
passthrough is one of the things a real-device trial has to show.

### Splash

**Radio > Splash** makes the start-up picture.

1. **Choose image…** picks a PNG. QuadCam scales it to fit 128 × 64, centres it on white and
   cuts it to two tones.
2. **Threshold** sets how dark a pixel must be to count as dark. **Invert** swaps the tones.
   The preview shows the radio's pixels at 4 times the size.
3. **Make firmware…** opens the flash sheet for the radio's installed version, with the
   picture put into the board's image.

QuadCam finds the splash in the image by its markers: `SPS`, a zero byte, the width 128 and the
height 64, then 1,024 picture bytes, then `SPE`. It refuses an image where a marker is missing,
appears twice or is not where the layout puts it. After the patch it decodes the picture and
compares it with the preview. Colour radios are refused: "This radio's splash format is not
supported yet."

The layout was compared with the official EdgeTX 2.12.4 Pocket image: one `SPS` marker,
`0x80 0x40`, 1,024 bytes of 8-row vertical bytes with the top row in bit 0 (a set bit is a drawn
pixel), then `SPE`. The bytes decode to the EdgeTX logo. The tests use a synthetic image with
the same layout. If another release differs, the plan refuses.

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
quadcam-cli --json gear fc pause|resume [--port ...]                 # pause the running app's FC reads
quadcam-cli --json gear fc usb                                   # USB timers
quadcam-cli --json gear blackbox pull [--port P] [--keep] [--mode auto|msp|msc] [--force]   # read, verify, store; erase if the setting allows
quadcam-cli --json gear blackbox list [--device ID]              # stored pulls with their guessed flights
quadcam-cli --json gear blackbox export <pull> DIR [--split]     # .bbl files
quadcam-cli --json gear blackbox erase [--port P] --yes      # erase the flash; needs a stored pull of it
quadcam-cli gear osd quad.dump_all.txt --text        # each OSD profile drawn, and the check
quadcam-cli --json gear osd quad.dump_all.txt apply.cli --grid PAL
quadcam-cli --json gear osd <fc> --staged                    # the layout with its staged OSD edits on top
quadcam-cli --json gear osd-edit <fc> --move vbat=12,3 --profiles vbat=1,3   # stage a move and a toggle
quadcam-cli --json gear osd-edit <fc> --copy 1:2             # profile 2 shows what profile 1 shows
quadcam-cli gear rates quad.dump_all.txt --text      # every rate profile: names, maximum and centre rates
quadcam-cli --json gear sims quad.dump_all.txt [--rate-profile N]   # the sims' rates against the quad
quadcam-cli --json gear sims quad.dump_all.txt --sync --to uncrashed:OUT --to liftoff:Freestyle   # the plan
quadcam-cli --json gear sims quad.dump_all.txt --sync --to uncrashed:OUT --digest D --yes        # write
quadcam-cli --json gear sims uncrashed --restore [--backup ID]                              # the plan to put a sim's file back
quadcam-cli --json gear sims uncrashed --restore [--backup ID] --digest D --yes                   # restore
quadcam-cli --json gear firmware [--check]           # installed against newest; --check reads the network
quadcam-cli --json gear elrs [status [--check]]      # ELRS devices read so far (a preview: elrs_preview)
quadcam-cli --json gear elrs read HOST [--port P]    # the module behind a radio or the receiver behind an FC
quadcam-cli --json gear elrs set DEVICE packet_rate=250Hz   # stage options; then gear apply
quadcam-cli --json gear elrs flash DEVICE [--version V] [--sha256 H]   # the flash plan
quadcam-cli --json gear elrs flash DEVICE --digest D --yes            # flash with the esptool module
quadcam-cli --json gear splash logo.png [--threshold 128] [--invert] [--board pocket] [--out preview.png]
quadcam-cli --json gear firmware --read [--device <radio>]   # read-only DFU trial: copy the firmware, compare the version
quadcam-cli --json gear firmware --plan --device <radio> [--version 2.12.4] [--splash logo.png]   # checks, diff, digest
quadcam-cli --json gear firmware --device <radio> [--splash logo.png] --digest D --yes   # flash (radio in DFU mode)
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
quadcam-cli --json gear report [--day 2026-10-04] --out report.md [--force]   # save it as a file
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
quadcam-cli --json gear card-clean [--device ID | --mount M] [--remove --yes]   # list or delete the ._ files macOS left
quadcam-cli --json gear radio-cli identify|ls|play|beep|reboot|verify [--port P] [--path P] [--device ID] [--yes]   # a radio on USB Serial
quadcam-cli --json gear dfu-link [--device ID] [--serial S] [--unlink]   # link the radio in DFU mode to a saved radio
quadcam-cli --json gear stop <handle>                # stop a backup or card check
quadcam-cli --json gear stage --device <id> --set osd_cap_alarm=1500   # stage; writes nothing
quadcam-cli --json gear stage --device <id> --cli lines.cli [--title T] # raw CLI lines
quadcam-cli --json gear changes [--device ID] [--history]
quadcam-cli --json gear apply <change> --plan        # the checks, the diff and the digest
quadcam-cli --json gear apply <change> --digest D --yes
quadcam-cli --json gear restore <backup> [--path P]...   # stage a backup back (a card: name the files)
quadcam-cli --json gear stage --device <radio id> --edits edits.json   # stage card edits
quadcam-cli --json gear update <change> --status try|ready|read_first|draft [--title T] [--order N]
quadcam-cli --json gear keep <change>                # an applied Try change becomes Verified
quadcam-cli --json gear revert <change>              # stage a restore of the backup its apply took
quadcam-cli --json gear copy --from <fc or backup> --to <fc> --part rates --part osd [--setting NAME]... --plan
quadcam-cli --json gear copy --from <fc> --to <fc> --part rates   # stage the copy
quadcam-cli --json gear card-mount <radio id> [--minutes 10]      # mount a card to browse it
quadcam-cli --json gear card-unmount <radio id>
quadcam-cli --json gear discard <change>
```

An edits file is a list. Each edit is a model (`{"kind": "model", "file": "model01.yml",
"name": "<its name>", "ops": [...]}`), the radio (`{"kind": "radio", "ops": [...]}`), a
checklist (`{"kind": "checklist", "model": "model01.yml", "text": "=Props tight"}`), a model
copy or a model delete. Model ops: `rename`, `set_model_id`, `set_flags`, `set_checklist`,
`set_mixes`, `set_logical_switch`, `special_functions`, `move_special_function`,
`set_timer`, `remove_timer`, `swap_timers`, `set_switch_warnings`, `set_screen`, and the editors' ops `set_screen_values`, `set_logging`,
`set_sensor_logs`, `set_rf_alarms`, `set_callout` and `remove_callout`. Radio ops:
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
| `QUADCAM_FLASH` | Firmware flashes go to a fake DFU device | The real USB path |
| `QUADCAM_FETCH` | Downloads reach only this Mac | Downloads reach the internet |

Card unmounts (`diskutil unmountDisk`) follow `QUADCAM_SERIAL`, and so does the list of DFU
devices. The firmware tests serve releases from memory and flash a fake DFU device. The ExpressLRS tests (`tests/gear_elrs.rs`) use a simulated radio, FC and ELRS device on fake ports and `esptool` as a recorder; ExpressLRS and esptool are never downloaded or run for real. Tests use the synthetic card
(`gear::edgetx::synth`) in a temporary folder.

**Blackbox: tested on a simulated FC only.** The tests drive `FakeFc` with a flash image. These need a
trial on a real FC before you trust them:

- The MSP flash messages on a real board: the summary flags, the 4 KB read, and that a reply is never
  compressed when the flag is off.
- How long a real erase takes, against the 4 s per MiB estimate behind the heat check.
- USB disk mode (`gear_blackbox_msc`): the disk's file names and layout, its speed, and whether the FC
  returns to serial after the disk is released.
- Pairing logs with flights by order, on days with test arms and missed flights.
