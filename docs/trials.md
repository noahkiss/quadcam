# Trying gear features on real hardware

QuadCam's tests never touch a real radio, flight controller (FC) or card. The gear features are
built and tested on simulated devices. This page is the order to try them on real ones, safest
first. Each trial reads or writes a little more than the one before.

Work through the trials in order. Stop at the first failure and [report it](#reporting-results).
A later trial assumes the earlier ones passed.

## Before you start

- Use QuadCam 0.11 or newer on an Apple Silicon Mac. **Settings > Gear** holds every setting
  named below. **Done** saves them.
- QuadCam writes only to boards and firmware versions it has proven
  ([Gear](gear.md#flight-controllers)). Today:

  | Device | Proven for writes |
  |---|---|
  | RadioMaster Pocket | EdgeTX 2.12 (a splash needs 2.12.4) |
  | BetaFPV G473 V2 FC (`BETAFPVG473_V2`) | Betaflight 2026.6.0 |
  | BetaFPV G473 FC (`BETAFPVG473`) | Betaflight 2025.12.5 |

  Any other board or version is read only. Trials 1 to 5 work on any device. Trials 6 to 10 need
  a proven device. If you have none, stop after trial 5.
- Plug USB in before the battery. Many FCs do not show up on USB when the battery is first.
- Click **Allow** if macOS asks about a new USB accessory. QuadCam cannot see that question. Until
  you click, it says "Nothing found."
- Close Betaflight Configurator and any serial terminal. QuadCam does not open a port another
  program has open.
- A small quad on USB with its battery in heats up fast. Keep the battery out unless a trial says
  to put it in. The `gear_usb_minutes` timer warns you at 20 minutes, or sooner on some boards.
- Never unplug a radio while QuadCam writes to its card. Wait for "Done, safe to unplug."
- Every write trial starts with a backup that QuadCam keeps. Backups live in the gear folder
  (**Gear > Storage**).

| Trial | Reads or writes | Settings to turn on |
|---|---|---|
| [1](#1-radio-over-usb) | Radio card (read), radio serial port (read) | none |
| [2](#2-read-the-radio-firmware) | Radio firmware (read) | none |
| [3](#3-identify-an-fc-and-back-it-up) | FC (read, one reboot) | none |
| [4](#4-pull-the-blackbox) | FC blackbox flash (read, then erase) | **Erase blackbox after download** (second pull) |
| [5](#5-expresslrs-read-then-one-option) | ExpressLRS options (read, then one write) | **Show the ELRS tools** |
| [6](#6-betaflight-flashing) | FC flash checks (read), then optionally a real flash | **Betaflight flashing (preview)** |
| [7](#7-apply-a-staged-change-to-an-fc-and-revert-it) | FC settings (write) | none |
| [8](#8-radio-card-apply-a-model-edit-and-a-voice-pack) | Radio card (write) | none |
| [9](#9-sync-rates-into-a-sim) | A sim's rate file (write) | none |
| [10](#10-flash-edgetx) | Radio firmware (write) | none |

## 1. Radio over USB

Reads the radio's SD card, then its serial port. Writes nothing to the radio.

**Needs**

- A radio on EdgeTX with a USB cable. No battery is needed.
- For the serial part: the radio's USB serial port set to **CLI** in its hardware settings.

**A. USB Storage card read**

1. Turn the radio on and plug it in. Choose USB Storage mode on the radio.
2. In QuadCam open **Gear > Connected**. Wait a few seconds. QuadCam looks every 2 seconds.
3. Open the radio. If QuadCam does not know it yet, it says **Needs attention**. Select **Save…**
   and name it.
4. On the radio's page, read **Overview**. Then open **Backups** and select **Back up now**.

**Pass**

- **Overview** shows the board and the EdgeTX version.
- The card check passes, and the backup appears in **Backups**.
- QuadCam unmounts the card and plays "Done, safe to unplug." The bar at the bottom shows
  **Safe to unplug**.
- The radio's files are unchanged. QuadCam wrote only to the gear folder.

**If it fails**

- "Nothing found": click **Allow** on the macOS accessory prompt, replug, and choose USB Storage
  mode again.
- **Needs attention** with "Unmount failed": the card stays in the list. QuadCam does not erase
  or repair a card on its own. If the card check failed, **Repair…** asks first.
- "macOS did not finish … a reboot may be needed": leave the radio plugged in and restart the Mac.
- Report: the radio model, the EdgeTX version, the message, and the Backups segment's reason.

**B. Serial CLI identify**

1. Plug the radio in with its USB serial port set to **CLI**. It shows as `<Radio> Serial Port`.
2. Run:

   ```bash
   quadcam-cli --json gear radio-cli identify
   ```

3. Optional, to list a card folder (read only): `quadcam-cli --json gear radio-cli ls --path /SOUNDS/en`.

**Pass**

- The answer holds the board and EdgeTX version the radio runs, and the saved radios of that board.
- The radio shows its board and version in the device list.

**If it fails**

- "is open in screen" (or another program's name): close that program. QuadCam refuses a busy port.
- A hint to turn the CLI on: set the radio's USB serial port to **CLI**.
- Report: the radio model and the error. QuadCam sends no command except the ones in
  [Gear](gear.md#radio-over-usb-serial).

## 2. Read the radio firmware

Saves a copy of the firmware that runs on the radio. The USB path refuses every command that could
erase, write or restart the radio. Run this before any flash.

**Needs**

- A radio, a USB cable, and a saved radio (trial 1) to compare versions. No battery.

**Steps**

1. Turn the radio off.
2. Plug the radio into the Mac. Hold no button. Wait a few seconds. (Both trims held starts the
   EdgeTX bootloader, which is another mode.)
3. Open **Gear > Firmware**. In **Read radio firmware**, pick the radio in **Radio**, or **Other
   radio** to skip the version comparison.
4. Select **Read firmware**. The bar covers two reads of the whole 1 MB flash.

CLI: `quadcam-cli --json gear firmware --read [--device RADIO]`.

**Pass**

- QuadCam reads the flash twice, finds both reads equal, saves the copy, reads the file back and
  compares it.
- The result says whether the version in the image matches the version QuadCam knows. A mismatch
  is not an error. Check the radio's About screen.
- A file pair appears in the gear folder: `firmware/<radio>/<time>-read.bin` and `.json`. Keep it.
  It is your recovery copy for trial 10.

**If it fails**

- "No radio in DFU mode": repeat steps 1 and 2.
- A blank flash, all zeros, or two reads that differ is refused with the reason. Nothing changed
  on the radio. Try another cable or port.
- Report: the radio model, the EdgeTX version, and the refusal text.

## 3. Identify an FC and back it up

Reads the FC over MSP (no reboot), then over the CLI (the FC reboots, as it does in Betaflight
Configurator).

**Needs**

- An FC on Betaflight, a USB cable, the battery **out**.
- **Back up on connect** on (the default) for the second part.

**Steps**

1. Plug the FC in over USB. Open **Gear > Connected**.
2. Run:

   ```bash
   quadcam-cli --json gear fc identify
   ```

   Add `--port /dev/cu.usbmodemX` when several FCs are plugged in.
3. Open the FC in **Gear > Connected**. If it says **Needs attention**, select **Save…**.
4. Let the on-connect backup run, or open **Backups** and select **Back up now**.

**Pass**

- `fc identify` shows the board, firmware, version and device id. It also says whether QuadCam may
  write this FC, and lists known issues. The FC does not reboot.
- The backup holds `version`, `status`, `diff all` and `dump all`. The FC reboots once and comes
  back.
- "Done, safe to unplug." plays. **Overview** shows **Last backup**.

**If it fails**

- "port busy" naming a program: close it. A background read skips quietly and tries again.
- No FC listed: unplug, plug USB in first (battery out), allow the accessory.
- "Board X is not proven.": the FC is read only. Trials 3 to 5 still work.
- The FC does not come back after the backup: unplug and replug it. Report the board, the
  Betaflight version and the step. Nothing was written to the FC.

## 4. Pull the blackbox

Copies the FC's blackbox flash to the Mac. The first pull must leave the flash alone. The second
pull may erase it.

**Needs**

- An FC with a flash chip that holds at least one log. Fly one pack first if it is empty.
- A USB cable, the battery **out** (a full 16 MB flash takes about 3.3 minutes).
- Leave **Read through USB disk mode first (not proven)** off.

**Steps: pull with erase off**

1. Open **Settings > Gear > Blackbox**. Turn **Erase blackbox after download** off. It is off by
   default. Select **Done**.
2. Open the FC and its **Blackbox** segment. Select **Pull blackbox**.
3. When it ends, open the pull in **Blackbox pulls**. Check the **Logs** table.
4. Copy the logs out and open one in your usual blackbox viewer:

   ```bash
   quadcam-cli --json gear blackbox list
   quadcam-cli --json gear blackbox export <pull> ~/Desktop/bb --split
   ```

**Pass**

- The pull appears with its size, and its **Flash** column says **Kept**.
- **Logs** lists each log with its firmware. QuadCam checked that the size equals the used bytes
  and that the image starts with a log header.
- "Done, safe to unplug." plays. The exported log opens in your viewer.

**Steps: pull with erase on**

Do this only after the exported log opened.

1. Turn **Erase blackbox after download** on in **Settings > Gear > Blackbox**. Select **Done**.
2. Select **Pull blackbox** again. (A pull of the same bytes adds no second record.)

**Pass**

- **Flash** says **Erased**. The flash reads ready with 0 bytes used.
- "Done, safe to unplug." plays after the erase, not before.

**If it fails**

- "USB heat": the timer has less time than the read needs. Unplug the battery. Or run
  `gear blackbox pull --force`. A forced pull still skips an erase it cannot finish.
- A failed read (a chunk that fails twice, a pulled plug): the job fails and nothing is erased.
- A failed erase: the pull stays stored, and the record says why. Erase by hand with **Erase flash**
  (`gear blackbox erase --yes`). It needs a stored pull of exactly what the flash holds.
- Report: the board, the Betaflight version, the flash size and bytes used, how long the pull and
  the erase took (the heat check assumes 4 seconds per MiB for the erase), and the log count.

## 5. ExpressLRS: read, then one option

Reads an ExpressLRS device through its host, then changes one option. This is a preview that
QuadCam has not tried on any real device. Do not select **Flash …** in this trial.

**Needs**

- **Settings > Gear > Preview > Show the ELRS tools** on, or **Show the ELRS tools** under
  **Firmware > ExpressLRS (preview)**. Every job refuses while it is off.
- For the radio's internal module: a saved radio with its USB serial port set to **CLI**, and no
  quad powered on that is linked to it. A read stops the radio's RF output at once.
- For an FC's receiver: a saved FC whose serial receiver is CRSF, not inverted and not half duplex.
  Keep the battery out. If the receiver does not answer on USB power, put the battery in and
  watch the USB timer.

**A. Read (options are read only)**

1. Plug in the radio. Open **Gear > Firmware** and find **ExpressLRS (preview)**.
2. Select **Read the module in <radio name>**.
3. When it ends, restart the radio. The radio stays in passthrough until you do. A second job
   needs the restart first.
4. Unplug the radio. Plug in the FC. Select **Read the receiver in <FC name>**.
5. When it ends, unplug the FC.

CLI: `quadcam-cli --json gear elrs read <saved radio or FC>`.

**Pass**

- The **ExpressLRS devices** table shows a row per device: **Transmitter** or **Receiver**, its
  target, its version and when it was read.
- **Options** lists packet rate, telemetry ratio, max power, dynamic power, switch mode and model
  match, as far as the device offers them.
- QuadCam never shows the binding phrase.

**B. Stage one option and Apply**

1. On a row select **Options**. Change one option, such as the telemetry ratio, to a value you
   can change back.
2. Select **Stage changes**. The apply sheet opens.
3. Read **Changes** and **Checks**. Select **Apply**.
4. Restart the radio, or unplug the FC. Read the device again and check the option.
5. Put the option back the same way.

**Pass**

- The sheet says the option read back as written. QuadCam kept the parameters as a backup first.
- The second read shows the new value.

**If it fails**

- A read fails on a radio: the USB serial port is not on **CLI**.
- A read fails on an FC: the serial receiver is not CRSF, or is inverted or half duplex. QuadCam
  names the check.
- A read refuses an FC it cannot identify over MSP: unplug the FC and plug it in again.
- The apply refuses because a list of values changed: the device's firmware lists them another
  way now. Read again.
- The apply refuses: an option moved since your read. Read again.
- A device stuck in passthrough: restart the radio, or unplug the FC.
- Report: the host (radio or FC) and its model, the ELRS target and version the read showed, the
  option, and the sheet's text.

## 6. Betaflight flashing

First read the checks only. Then, if you choose, flash the proven release.

**Needs**

- A proven FC (see [Before you start](#before-you-start)), saved in QuadCam, with a backup.
- **Settings > Gear > Preview > Betaflight flashing (preview)** on. It is off by default.
- A USB cable, the battery **out**.

**A. Read the checks only**

1. Plug the FC in. Open **Gear > Firmware**.
2. In the FC's row select **Flash <release>…**. The button shows only for a proven board and
   release, with the preview on.
3. Read the **Checks**. Do not select **Apply**. Select **Cancel**.

CLI: `quadcam-cli --json gear firmware --plan --device <FC id>`. QuadCam downloads the image from
Betaflight's build service for the plan. It writes nothing to the FC.

**Pass**

- Each check passes: the preview is on, one FC, the saved device, the board and release, the
  firmware image, the port is free, no DFU device is plugged in, and the USB timer outlasts the
  flash.
- The plan shows the image's SHA-256 and a digest.

**If it fails**

- A check names its reason. A board or release that is not proven gets no button.
- If the service answers in a shape QuadCam does not expect, the plan refuses. Report the text.
- Report: the board, the installed and the target release, and the failing check.

**B. A real flash (optional, do it last)**

A flash replaces the FC's firmware. Do this only on an FC you can recover.

**Needs**

- The proven release for the board, shown on the button.
- Betaflight Configurator on this Mac, to recover.
- A fresh backup (QuadCam makes one, `before_flash`, and keeps it).

**Steps**

1. Select **Flash <release>…**, check that every check passes, and select **Apply**.
2. Do not touch the FC, the cable or the battery until the sheet shows the result.

QuadCam backs up `diff all` and `dump all`, sends `bl`, waits for one new DFU device, erases and
writes the flash, reads every byte back, restarts the FC, checks it is the same FC (its MCU id),
reads the new firmware's settings, and applies the old settings as one change.

**Pass**

- The sheet shows the flash and the settings apply as verified.
- The report lists what came back, and what it skipped: a setting the new version no longer has,
  and board lines. Skipped lines stay in the `before_flash` backup.

**If it fails**

Recovery, from [Gear](gear.md#flash-a-betaflight-fc-preview):

| Where it stopped | State | What to do |
|---|---|---|
| Before the restart into the bootloader | Nothing changed | Fix the named check and retry |
| The FC never shows as a DFU device | Nothing written | Unplug USB and the battery, plug USB in, retry. The report names the backup |
| The flash fails before the erase | Nothing erased; the old firmware is whole | QuadCam asks the FC to leave DFU. If it stays there, unplug USB and the battery and plug USB in |
| A flash step after the erase or the read back fails | Half a firmware. The FC stays in its bootloader | Leave USB in. Flash an official build from Betaflight Configurator, which finds the FC in DFU mode. QuadCam cannot flash it from there |
| The FC does not start afterwards | The ROM bootloader is intact | Unplug USB and battery. Hold the FC's boot button. Plug USB in. Flash an official build from Betaflight Configurator |
| Flashed, but settings did not return | Firmware is fine | The old settings are in the `before_flash` backup. The staged change stays in **Changes** |

Report: the board, both releases, the step that failed, and the report text.

## 7. Apply a staged change to an FC and revert it

The first write to an FC's settings. It changes one harmless setting, then puts it back.

**Needs**

- A proven FC, saved, with a backup (the plan compares with it).
- A USB cable, the battery **out**.
- A setting you can change safely, such as `osd_cap_alarm`.

**Steps**

1. Open the FC and its **Changes** segment. Select **Edit setting…**.
2. Fill in **Setting** (`osd_cap_alarm`), **Value** (a number different from the current one) and
   **Applies to** (**The whole FC**). Select **Stage**.
3. Open **Gear > Bench**. In the change's status list choose **Try**.
4. Select **Review…**. Read **Changes** and **Checks**. **Apply** stays off while a check fails.
5. Select **Apply**. Wait for the result. Do not unplug.
6. Back on the **Bench**, the change says "Applied. Fly it, then decide." Select **Revert…**. The
   sheet opens on the undo. Select **Apply**.

CLI:

```bash
quadcam-cli --json gear stage --device <FC id> --set osd_cap_alarm=1500
quadcam-cli --json gear update <change> --status try
quadcam-cli --json gear apply <change> --plan
quadcam-cli --json gear apply <change> --digest <digest> --yes
quadcam-cli --json gear revert <change>
```

**Pass**

- **Checks** all pass. Apply backs the FC up (**Before apply**, kept), sends the line, saves,
  waits for the restart, reads `dump all` and finds the line as written.
- QuadCam stores the new state as a backup (**After apply**).
- After the revert the change says **Reverted**. A new backup compared with the first shows no
  difference (**Backups**, pick a backup to see its changes).

**If it fails**

- A check fails: it names the reason ("Board X is not proven.", "Port free", "USB heat").
- "The FC changed since the plan; plan again."
- The FC refuses a line: QuadCam sends `exit`, saves nothing, and the FC is as it was. The sheet
  names the line.
- The FC saved but a line did not read back: the sheet lists the lines and offers **Restore
  backup**. It stages the **Before apply** backup as a new change.
- Report: the board, the Betaflight version, the setting, and the sheet's text.

## 8. Radio card apply: a model edit and a voice pack

The first write to a radio's SD card. QuadCam backs the card up, writes one file at a time under a
temporary name, reads each back, and puts every file back if one reads wrong.

**Needs**

- A radio proven for writes (RadioMaster Pocket on EdgeTX 2.12), saved, with a backup.
- USB Storage mode and a good cable. Over the radio's USB a write is slow, about 0.3 MB/s.
- A radio that stays plugged in until the sheet shows the result.

**A. A model edit**

1. Open the radio and its **Models** segment. In the **Model** list pick a model.
2. Under **Timers**, change **Timer 1 name** to `TRIAL`. (Names hold 8 characters.) If the model
   has no timer, select **Add timer** and name it.
3. A bar shows the staged change with **Review…** and **Undo model edits**. Open **Gear > Bench**
   and choose **Try** in the change's status list.
4. Select **Review…**, read **Checks** and **Changes**, and select **Apply**.
5. Wait for "Done, safe to unplug." Check the name on the radio.
6. On the **Bench** select **Revert…**, then **Apply**.

**Pass**

- **Checks** pass: one card plugged in, the same card, the card check, nothing else writing, a
  known version, shape understood, round trip, model identity, values in range.
- The sheet reads every file back and unmounts the card. The model's other lines are unchanged.
- The revert restores the file.
- Optional: with the card back in a radio on **CLI**, run
  `quadcam-cli --json gear radio-cli verify`. It lists files the radio lacks or holds at another
  size.

**B. The voice pack**

1. Open the radio's **Voice** segment.
2. To stage one line first, use **My text…** on one line, then **Review…** and **Apply**.
3. For a whole pack you need a local one. Select **Check cost**, then **Render my voice**. With
   `tts_provider` `say` it is free and offline. A provider that may charge stops and shows the
   characters first. Or select **Refresh packs** and **Install** a pack from the index.
4. Pick the voice. Leave **Keep my overrides** on. Select **Choose voice**.
5. Select **Review…**, then **Apply**. A whole pack takes minutes over USB.
6. Optional: hear a line on the radio with
   `quadcam-cli --json gear radio-cli play --path /SOUNDS/en/<line>.wav`.

**Pass**

- The result says the files read back as written. Sounds that no pack knows are not deleted.
- The line plays on the radio.

**If it fails**

- A file reads back different, or a write fails: QuadCam puts every file back to the bytes in the
  backup. The sheet says "the card is as it was". A new file is removed.
- **Stop** finishes the current file, then puts the written files back.
- macOS stops answering, or an unmount does not finish in 60 seconds: leave the radio plugged in,
  and restart the Mac if needed. Once, pulling a radio mid-write wedged the disk service.
- The radio's firmware stops at an error: hold both horizontal trims inward while you power it
  on. The bootloader shows the SD card over USB.
- Restore: **Gear > Bench** or `quadcam-cli --json gear restore <backup> --path <file>`.
- Report: the radio model, the EdgeTX version, the files and size written, the time, and the
  sheet's text.

## 9. Sync rates into a sim

Writes an FC's rate profile into a sim's rate file. QuadCam supports four sims. It backs each file
up first and puts it back if a write fails.

| Sim | Seen loading in the game |
|---|---|
| Uncrashed | Yes |
| The Zone | No. The plan says **Unverified** |
| Liftoff | No. The plan says **Unverified** |
| Liftoff: Micro Drones | No. The plan says **Unverified**. Its file lives inside the game's app bundle |
| Velocidrone | Off. QuadCam cannot read its file yet |

Try the sims one at a time, Uncrashed first ([Gear](gear.md#sync-the-quads-rates-into-a-sim),
[Sim](sim.md)).

**Needs**

- A saved FC with a backup. QuadCam compares with the FC it saw last. **Compare with** picks
  another.
- The sim installed, **quit**, and a profile in it with the name you want to overwrite. QuadCam
  does not create sim profiles.

**Steps**

1. Open **Gear > Sims**. Each sim shows **Matches the quad** or **Out of date**.
2. Beside the profile to overwrite, select **Sync**. The sheet opens.
3. Read **Checks** (the game is not running, the file is understood, it reads back as written,
   it is writable) and **Warnings**. Select **Apply**.
4. Start the game. Open its rates screen. Compare the values with the quad's.
5. To undo, select **Restore backup** beside the sim, then **Apply**.

CLI:

```bash
quadcam-cli --json gear sims <FC id> --sync --to uncrashed:OUT
quadcam-cli --json gear sims <FC id> --sync --to uncrashed:OUT --digest <digest> --yes
quadcam-cli --json gear sims uncrashed --restore
```

**Pass**

- The result says the file read back and parsed.
- The game shows the quad's rates, and the sim says **Matches the quad**. The **Sims** item in the
  sidebar drops its **Out of date** mark when no sim differs.
- **Restore backup** puts the old numbers back, and the game shows them.

**If it fails**

- "Quit Liftoff first.": QuadCam never writes a sim that runs. Quit the game.
- A check fails because QuadCam does not understand the file: it writes nothing. Report the
  game version.
- A write fails: QuadCam puts back every file it wrote.
- The game loads the file but shows other numbers, or refuses it: select **Restore backup**.
  This is the result that decides whether the sim leaves **Unverified**. Report it.
- If macOS asks for a permission when QuadCam writes a game's file, report where.
- Report: the sim and its version, the profile, the plan's warnings, and what the game's rates
  screen showed.

## 10. Flash EdgeTX

The highest risk. A wrong or interrupted flash can leave the radio unable to start until you
flash it again. The chip's USB bootloader is in ROM, so QuadCam cannot overwrite it, and the radio
can always be put back in DFU mode.

**Needs**

- A RadioMaster Pocket, flashed with EdgeTX 2.12.4. Other boards, releases and prereleases are
  refused.
- Trials 1, 2 and 8 passed. Trial 2's copy is in `firmware/<radio>/`.
- A fresh card backup: **Back up now** on the radio's **Backups** segment. The flash plan refuses
  without one.
- Only one radio plugged in.
- A second route to recover: STM32CubeProgrammer, and the release's `<board>-<hash>.bin`.
- No splash on this flash. Try **Radio > Splash** afterwards.

**Steps**

1. Link the radio to its DFU identity once. Put the radio in DFU mode (below), then run
   `quadcam-cli --json gear dfu-link --device <radio id>`. Check the answer names your radio.
2. Open **Gear > Firmware**. In the radio's row select **Flash <version>…**. Read **Checks**: the
   board and version, the firmware image, the splash markers, the card backup, and exactly one
   radio in DFU mode.
3. Turn the radio off and plug in USB. Hold no button. The radio shows no name in DFU mode.
4. Select **Apply**. Do not touch the cable until the sheet shows the result.
5. Connect the radio in USB Storage mode and check its version in **Overview**.

CLI:

```bash
quadcam-cli --json gear firmware --plan --device <radio id>
quadcam-cli --json gear firmware --device <radio id> --digest <digest> --yes
```

QuadCam checks the chip's flash size, reads the current firmware twice, checks that it names the
radio's board and saves a copy. It then erases the sectors it needs, writes the image in 16 KB
segments (reading each back), reads the whole image back and compares every byte, then leaves
DFU mode.

**Pass**

- The sheet shows every step done. The radio restarts.
- **Overview** shows the new version.
- A verified flash links the DFU device to the radio.

**If it fails**

Recovery, from [Gear](gear.md#if-a-flash-fails-or-the-radio-will-not-start):

1. Turn the radio off. Unplug other radios.
2. Plug in USB with no button held. Wait a few seconds.
3. Open **Gear > Firmware**, select **Flash <version>…**, then **Apply**. A blank flash is allowed.
4. If QuadCam cannot, use a DFU tool on the copy in `firmware/<radio>/`. Or use STM32CubeProgrammer
   with the release's `<board>-<hash>.bin` at address `0x08000000`.
5. If the radio shows the EdgeTX bootloader (both trims held), copy a firmware file to its SD card
   from there, as the EdgeTX manual describes.

| Where it stopped | What QuadCam does |
|---|---|
| The cable or Mac drops during the read | Nothing changed. Nothing was erased |
| Two reads differ, or the saved copy does not match | No copy and no erase |
| The chip is not the expected size | Refuses before any command |
| The current firmware names another board | Refuses before the erase |
| Power or USB drops during the erase or write | The radio stays in DFU mode. Flash again |
| A segment or the final read back differs | The image is not started. Select **Apply** again |
| The image is not the board's or the version's | Refused in the plan |

Report: the radio model, the old and new version, the step that stopped, and the sheet's text.

## Reporting results

Open an issue on the QuadCam GitHub repository. Say which trial passed, which failed, and which
you skipped. For each failure include:

- The QuadCam version (`quadcam-cli --version`, or the app's About window).
- The board or radio model and its firmware version (for example "BETAFPVG473_V2, Betaflight
  2026.6.0" or "Pocket, EdgeTX 2.12.4"). For a sim, the game and its version.
- The trial number, the step, and the exact text of any message or the apply sheet.
- The job's record: the change or backup id and the sheet's result, or the command and its
  `--json` output.
- What you expected, and what happened.

Leave out serial numbers, device ids, binding phrases, API keys, file paths with your user name,
and any other personal data. Replace them with a placeholder before you paste. Do not attach a
backup or a firmware copy without checking it first.
