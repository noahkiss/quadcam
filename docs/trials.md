# Trying gear features on real hardware

QuadCam's tests never touch a real radio, flight controller (FC) or card. The gear features are
built and tested on simulated devices. This page is the order to try them on real ones, safest
first. Each trial reads or writes a little more than the one before.

Work through the trials in order. Stop at the first failure and [report it](#reporting-results).
A later trial assumes the earlier ones passed. Trials 11 to 16 need at most trials 1 and 3, so you
can run them before trials 6 to 10. Trial 17, the 1.0 gate, comes last.

## Before you start

- Use QuadCam 0.11 or newer on an Apple Silicon Mac. **Settings > Gear** holds every setting
  named below. **Done** saves them.
- QuadCam writes only to boards and firmware versions it has proven
  ([Gear](gear.md#flight-controllers)). Today:

  | Device | Proven for writes |
  |---|---|
  | RadioMaster Pocket | EdgeTX 2.12 (a splash needs 2.12.4) |
  | RadioMaster GX12 | EdgeTX 2.12.5 |
  | BetaFPV G473 V2 FC (`BETAFPVG473_V2`) | Betaflight 2026.6.0 |
  | BetaFPV G473 FC (`BETAFPVG473`) | Betaflight 2025.12.5 |

  Any other board or version is read only. Trials 1 to 5 and 11 to 16 work on any device.
  Trials 6 to 10 and 17 need a proven device. If you have none, skip them.
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
| [5](#5-expresslrs-read-each-module-then-one-option-then-a-flash) | ExpressLRS: each radio's module and each receiver (read), one option (write), then a flash | **Show the ELRS tools** |
| [6](#6-betaflight-flashing) | FC flash checks with the maker's build (read), then optionally a real flash | **Betaflight flashing (preview)** |
| [7](#7-apply-a-staged-change-to-an-fc-and-revert-it) | FC settings (write) | none |
| [8](#8-radio-card-apply-a-model-edit-and-a-voice-pack) | Radio card (write) | none |
| [9](#9-sync-rates-into-a-sim) | A sim's rate file (write) | none |
| [10](#10-flash-edgetx) | Radio firmware (write) | none |
| [11](#11-card-prep-a-spare-dvr-card-and-a-goggles-card) | A DVR card and a goggles card (erase) | none |
| [12](#12-card-check-and-repair) | A card's file system (read, repair only after a failed check) | none |
| [13](#13-import-with-the-ffmpeg-module) | The ffmpeg module (install), clips (import) | none |
| [14](#14-switch-map-position-by-position) | Radio and FC backups, live channels (read) | none |
| [15](#15-osd-render-against-the-goggles) | An FC backup (read) | none |
| [16](#16-cues) | Nothing on a device | **Cues** settings, changed during the trial |
| [17](#17-one-full-bench-session-in-quadcam-only) | Everything: the 1.0 gate | none |

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
4. On the radio's page, read **Overview**. Then open **Backups** and select **Back Up Now**.

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
4. Select **Read Firmware**. The bar covers two reads of the whole 1 MB flash.

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
4. Let the on-connect backup run, or open **Backups** and select **Back Up Now**.

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
2. Open the FC and its **Blackbox** segment. Select **Pull Blackbox**.
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
2. Select **Pull Blackbox** again. (A pull of the same bytes adds no second record.)

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

## 5. ExpressLRS: read each module, then one option, then a flash

Reads the ExpressLRS device behind each host, changes one option, and last flashes. This is a
preview that QuadCam has not tried on a real device. Run the parts in order: A on both radios,
then B, then trial 6 part A, and only then C. C is a real write.

What the tests prove on simulated devices, and what only this trial can show:

| Proven on fakes | Only a real device shows |
|---|---|
| Frames start with a byte ExpressLRS 4.1 takes (`0xC8`, or the device's own address) | That the module answers through EdgeTX's `serialpassthrough rfmod 0 400000` at all |
| A module answers only right after it takes a frame: QuadCam sends a heartbeat every 20 ms while it waits | That the heartbeats keep the module at 400000 baud |
| A module misses frames for its first second after power-up: QuadCam pings for up to 8 s | How long the GX12's LR1121 module takes to answer |
| The target per radio: Pocket `radiomaster.tx_2400.pocket`, GX12 `radiomaster.tx_dual.gx12` | That esptool reaches the module through the passthrough at 460800 |
| Both quads' receivers: `betafpv.rx_2400.aio` (name `BFPV AIO 2G4RX`) | That esptool reaches the receiver through Betaflight's passthrough at 420000 |

**Needs**

- **Settings > Gear > Preview > Show the ELRS tools** on, or **Show the ELRS tools** under
  **Firmware > ExpressLRS (preview)**. Every job refuses while it is off.
- For a radio's internal module: the radio saved, its USB serial port set to **CLI**, and no quad
  powered on that is linked to it. A read stops the radio's RF output at once.
- For an FC's receiver: a saved FC whose serial receiver is CRSF, not inverted and not half duplex.
  Keep the battery out. If the receiver does not answer on USB power, put the battery in and
  watch the USB timer.

**A. Read each radio's internal module (read only)**

Do this on the Pocket, then on the GX12.

1. Turn the radio on, plug in USB and choose **USB Serial (VCP)**. Open **Gear > Firmware** and
   find **ExpressLRS (preview)**.
2. Select **Read the Module in <radio name>**. It can take up to 10 seconds: the module restarts
   and QuadCam pings until it answers.
3. When it ends, restart the radio. The radio stays in passthrough until you do. A second job
   needs the restart first.

CLI: `quadcam-cli --json gear elrs read <saved radio> --yes`.

**Pass**

- The **ExpressLRS devices** table shows a **Transmitter** row for each radio: target `RM Pocket`
  or `RM GX12 X-Band`, version 4.1.0, and the time of the read.
- **Options** lists packet rate, telemetry ratio, max power, dynamic power, switch mode and model
  match, as far as the module offers them.
- QuadCam never shows the binding phrase.

**B. Read each receiver, then stage one option and Apply**

1. Plug in the Air65 II FC (battery out). Select **Read the Receiver in <FC name>**. Unplug it.
   Do the same for the Meteor75.
2. Pass: a **Receiver** row for each, target `BFPV AIO 2G4RX`, version 4.1.0.
3. On a radio's row select **Options**. Change one option, such as the telemetry ratio, to a
   value you can change back.
4. Select **Stage Changes**. The apply sheet opens. Read **Changes** and **Checks**. Select
   **Apply**.
5. Restart the radio. Read the module again and check the option.
6. Put the option back the same way.

**Pass**

- The sheet says the option read back as written. QuadCam kept the parameters as a backup first.
- The second read shows the new value.

**C. Flash (a real write; after trial 6 part A)**

ExpressLRS majors must match: flash the receivers before the radios. Each plan must show
**Firmware image** with the right target before you select **Apply**.

1. On a receiver's row select **Flash 4.1.0…**. Check: target `BETAFPV 2.4GHz AIO RX
   (Unified_ESP8285_2400_RX on esp8285)`, the UID fingerprint, region FCC. Select **Apply**.
2. Unplug the FC, power-cycle the quad, and read the receiver again.
3. On each radio's row select **Flash 4.1.0…**. Check the target: Pocket `RadioMaster Pocket
   Internal 2.4GHz TX`, GX12 `RadioMaster GX12 2.4/900 TX (Unified_ESP32_LR1121_TX on esp32)`.
   Select **Apply**.
4. **After a radio's module flash, expect the radio to look frozen for a minute or more**: the
   screen stays on, sticks, pots and the power button do nothing, and unplugging USB changes
   nothing. Wait. It recovers by itself (the GX12 sounded its inactivity alarm first), and then
   the power button works. Do not pull the battery. The plan and the report both say this.
5. Restart the radio and read the module again.

**Pass**

- Each report says **Verified** (esptool's "Hash of data verified") and names a `before_flash`
  backup of the chip's old firmware.
- The read afterwards shows 4.1.0 and the same target. The quad links to the radio.

**If it fails**

- A radio read finds no module: the USB serial port is not on **CLI**, or the module did not
  answer through the passthrough. Report how long the read ran.
- A read fails on an FC: the serial receiver is not CRSF, or is inverted or half duplex. QuadCam
  names the check.
- A read refuses an FC it cannot identify over MSP: unplug the FC and plug it in again.
- The apply refuses because a list of values changed, or an option moved since your read. Read
  again.
- A device stuck in passthrough: restart the radio, or unplug the FC.
- A flash that fails leaves the device in its bootloader. Power-cycle it and flash again, or use
  the ExpressLRS Configurator with the same target.
- Report: the host and its model, the ELRS target and version the read showed, the step, and the
  sheet's text.

## 6. Betaflight flashing

First read the checks only, on both quads. Then, if you choose, flash.

**Needs**

- A proven FC (see [Before you start](#before-you-start)), saved in QuadCam with its aircraft
  linked (the aircraft picks the maker's files), with a backup.
- **Settings > Gear > Preview > Betaflight flashing (preview)** on. It is off by default.
- A firmware check run since the FC was saved (**Firmware > Check for Updates**). It reads BetaFPV's
  files for each quad.
- A USB cable, the battery **out**.

Both quads run BetaFPV's own builds, so QuadCam plans BetaFPV's `.hex` for each quad, the one
paired with the newest `.txt` its firmware check read. Keep each FC's **Firmware source** on
**Automatic**: only a hand-set **Official Betaflight** plans an official build, with a warning.

**A. Read the checks only (no flash)**

Do this on the Air65 II, then on the Meteor75.

1. Plug the FC in.
2. Run `quadcam-cli --json gear firmware --plan --device <FC id>`. The Firmware page shows no
   **Flash…** button for an FC in its maker's channel yet. QuadCam downloads the `.hex` for the
   plan. It writes nothing to the FC.
3. Read the checks and the **Firmware image** lines.

**Pass**

- **Firmware source** passes, and **Firmware image** names BetaFPV's file for the quad: Air65 II
  `A65 II F_0702_Freestyle_GF 1219S_2026_6_0 20260515.hex` (BMI270), Meteor75 the
  `.hex` paired with its `20260625` `.txt` (build 2025.12.5-alpha, eb2bb5a33). Never an official `BETAFPVG473_V2` build.
- Every other check passes: the preview, one FC, the saved device, the board and release, the
  port is free, no DFU device is plugged in, and the USB timer outlasts the flash.
- The plan shows the image's SHA-256 and a digest. If the FC already runs that build, a warning
  says the flash writes the same build again.

**If it fails**

- **Firmware source** refuses with "has not read BetaFPV's files": run a firmware check first.
- **Firmware image** names an official build: the FC's **Firmware source** is set to **Official
  Betaflight**, or QuadCam sees no BetaFPV quad for this FC. Set it to **Automatic** and check
  its linked aircraft.
- Report: the board, the installed build, the planned file, and the failing check.

**B. A real flash (optional, do it last)**

A flash replaces the FC's firmware. Do this only on an FC you can recover.

**Needs**

- Trial 6 part A passed on that FC.
- Betaflight Configurator on this Mac, and BetaFPV's `.hex` for the quad saved locally, to
  recover.
- A fresh backup (QuadCam makes one, `before_flash`, and keeps it).

**Steps**

1. Run the plan again and copy its digest. Run
   `quadcam-cli --json gear firmware --device <FC id> --digest <D> --yes`, then confirm in the app.
2. Do not touch the FC, the cable or the battery until the result shows.

QuadCam backs up `diff all` and `dump all`, sends `bl`, waits for one new DFU device, reads the
old flash twice and keeps it, erases and writes the flash, reads every byte back, restarts the
FC, checks it is the same FC (its MCU id), reads the new firmware's settings, and applies the old
settings as one change.

**Pass**

- The report shows the flash and the settings apply as verified.
- The report lists what came back, and what it skipped: a setting the new version no longer has,
  and board lines. Skipped lines stay in the `before_flash` backup.
- The quad arms on the bench (props off) and the OSD looks as before.

**If it fails**

Recovery, from [Gear](gear.md#flash-a-betaflight-fc-preview):

| Where it stopped | State | What to do |
|---|---|---|
| Before the restart into the bootloader | Nothing changed | Fix the named check and retry |
| The FC never shows as a DFU device | Nothing written | Unplug USB and the battery, plug USB in, retry. The report names the backup |
| The flash fails before the erase | Nothing erased; the old firmware is whole | QuadCam asks the FC to leave DFU. If it stays there, unplug USB and the battery and plug USB in |
| A flash step after the erase or the read back fails | Half a firmware. The FC stays in its bootloader | Leave USB in. Flash BetaFPV's `.hex` for the quad from Betaflight Configurator (Load Firmware [Local]), which finds the FC in DFU mode. QuadCam cannot flash it from there |
| The FC does not start afterwards | The ROM bootloader is intact | Unplug USB and battery. Hold the FC's boot button. Plug USB in. Flash BetaFPV's `.hex` from Betaflight Configurator |
| Flashed, but settings did not return | Firmware is fine | The old settings are in the `before_flash` backup. The staged change stays in **Changes** |

Report: the board, both builds, the step that failed, and the report text.

## 7. Apply a staged change to an FC and revert it

The first write to an FC's settings. It changes one harmless setting, then puts it back.

**Needs**

- A proven FC, saved, with a backup (the plan compares with it).
- A USB cable, the battery **out**.
- A setting you can change safely, such as `osd_cap_alarm`.

**Steps**

1. Open the FC and its **Changes** segment. Select **Edit Setting…**.
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
   has no timer, select **Add Timer** and name it.
3. A bar shows the staged change with **Review…** and **Undo Model Edits**. Open **Gear > Bench**
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
2. To stage one line first, use **My Text…** on one line, then **Review…** and **Apply**.
3. For a whole pack you need a local one. Select **Check Cost**, then **Render My Voice**. With
   `tts_provider` `say` it is free and offline. A provider that may charge stops and shows the
   characters first. Or select **Refresh Packs** and **Install** a pack from the index.
4. Pick the voice. Leave **Keep my overrides** on. Select **Choose Voice**.
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
5. To undo, select **Restore Backup** beside the sim, then **Apply**.

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
- **Restore Backup** puts the old numbers back, and the game shows them.

**If it fails**

- "Quit Liftoff first.": QuadCam never writes a sim that runs. Quit the game.
- A check fails because QuadCam does not understand the file: it writes nothing. Report the
  game version.
- A write fails: QuadCam puts back every file it wrote.
- The game loads the file but shows other numbers, or refuses it: select **Restore Backup**.
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
- A fresh card backup: **Back Up Now** on the radio's **Backups** segment. The flash plan refuses
  without one.
- Only one radio plugged in.
- A second route to recover: STM32CubeProgrammer, and the release's `<board>-<hash>.bin`.
- No splash on this flash. Try **Radio > Splash** afterwards.

**Steps**

1. Link the radio to its DFU identity once. Put the radio in DFU mode (below), then select
   **Link to <radio>** under **DFU device** in the flash sheet, or run
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

## 11. Card prep: a spare DVR card and a goggles card

Erases a card that has no import session behind it. QuadCam refuses while a clip on the card is
not in the library. See [Format safety](format-safety.md#card-prep).

**Needs**

- A spare analog DVR card and a removable DJI goggles card, in a card reader or the Mac's SD
  slot. Use cards whose clips you have imported, or cards with nothing you want to keep.
- For the refusal check: one clip on the DVR card that is not in the library. Record a short clip
  on the DVR, or keep one you have not imported yet.

**A. The refusal**

1. Put the DVR card in. Open it under **Gear > Connected**.
2. Select **Prepare Card…**.

**Pass**

- QuadCam erases nothing. A message says "Refused: N of M clips on the card are not in the
  library (…). Import them first." and names up to five clips.

**B. The DVR card**

1. Import the missing clip: select the card under **Import from**, then **Add to Library**, and
   finish the import.
2. Open the card's page in **Gear** again. If the card is unmounted but still in, it shows
   **Mount**; **Prepare Card…** works either way.
3. Select **Prepare Card…**. Read the dialog. Select **Erase**.
4. Put the card back in the DVR. Record a short clip.

**Pass**

- The dialog says "Erase /dev/diskN (<volume>, <size>) as FAT32 …? This cannot be undone." It
  adds size advice for a card over 32 GB. The Return key does not erase.
- "Card prepared. Safe to remove." shows, and the card unmounts.
- The DVR records to the card, and QuadCam lists the new clip under **Import from**.

**C. The goggles card**

1. Put the goggles card in. Check that every clip on it is in the library (step A refuses if
   not).
2. On its page in **Gear**, select **Prepare Card…**, then **Erase**.
3. Put the card back in the goggles. Record a short clip.

**Pass**

- The dialog names exFAT. The card unmounts after the erase.
- The goggles read the card and record to it.
- Optional: plug a DJI air unit or the goggles in over USB (storage mode). Its page offers no
  erase: **Prepare Card…** refuses with "Refused: QuadCam never formats a DJI device over USB
  …".

**If it fails**

- "Card … is not plugged in": the card left the disk list. Take it out, put it back, and try
  again.
- The volume did not mount: take the card out and put it back.
- The DVR or the goggles do not read the card: format it in the device, and report it.
- Report: the card's size and brand, the device (DVR or goggles model), the dialog's text, and
  the message.

## 12. Card check and repair

Checks a card's file system. A repair runs only after a failed check, and asks first.

**Needs**

- A DVR card, a goggles card or a radio in USB Storage mode, saved in QuadCam.
- **Settings > Gear > Back up on connect** on (the default), for the check on connect.

**Steps**

1. Plug the card in. QuadCam checks a card it knows before its on-connect steps. Wait for "done,
   safe to unplug."
2. Open the device's **Backups** segment. Read the check's result in **Overview** (the last card
   check).
3. If the card is unmounted, select **Mount**. Then select **Check Card**. **Stop** ends a check.
4. Try the repair refusal on the healthy card:

   ```bash
   quadcam-cli --json gear card-check --device <id>
   quadcam-cli --json gear card-repair --check <check id> --yes
   ```

5. Only if a check ever fails: select **Repair…** in the banner, then **Repair**.

**Pass**

- A check takes about 30 seconds over a radio's USB, less in a card reader. The result says "The
  file system is OK."
- The card unmounts after the check, and the cue plays.
- `card-repair` on a card whose last check passed refuses: "Refused: the card's latest check
  passed; there is nothing to repair."
- After a failed check: the device shows **Needs attention**, and a banner says "Card check
  failed: …" with **Repair…**. The repair backs up the card when it can read it, repairs it,
  checks it again, and says "Repaired. The card checks out."

**If it fails**

- A check that never ends: select **Stop**, and report how long it ran.
- "The repair could not fix every error.": copy the files off the card, then format it in the
  device.
- Report: the card's kind and size, the summary text, and the check's line in
  `health/<device>.jsonl` in the gear folder (**Gear > Storage**).

## 13. Import with the ffmpeg module

Imports clips with the ffmpeg module, then with Homebrew's ffmpeg, then with none.

**Needs**

- A DVR card with a few short clips.
- For part B: Homebrew's `ffmpeg` installed. Skip part B without it.

**A. The module**

1. If the banner says **ffmpeg not found**, select **Install ffmpeg…**. Otherwise open
   **Settings > Modules** and select **Install** in the ffmpeg and ffprobe row.
2. Read the dialog: version, size, license, source and address. Select **Download**.
3. In **Settings > Modules**, set **Use ffmpeg from** to **The QuadCam module, else Homebrew**.
   Read the **In use:** line.
4. Import the card: select it under **Import from**, then **Add to Library**.
5. Play one imported clip.

**Pass**

- Nothing downloads before **Download**. The module installs with no restart.
- **In use:** names the module's folder (`…/app.quadcam/modules/ffmpeg/…`).
- Every clip verifies, and the clip plays in QuickTime Player.

**B. Homebrew**

1. Set **Use ffmpeg from** to **Homebrew**. Read **In use:**.
2. Import the same clips again into a test folder, or import a second card.

**Pass**

- **In use:** names Homebrew's ffmpeg (`/opt/homebrew/bin/ffmpeg`).
- The clips verify as in part A.

**C. No ffmpeg**

1. Set **Use ffmpeg from** back to **The QuadCam module, else Homebrew**.
2. Only without Homebrew's ffmpeg: select **Remove** on the module, then **Install ffmpeg…** in
   the banner again.

**Pass**

- With no ffmpeg, the banner says "ffmpeg not found. Import is blocked until QuadCam has ffmpeg."
  Import stays off.
- After the reinstall, import works again with no restart.

**If it fails**

- "The download does not match the expected checksum.": the download was cut or changed. Try
  again, and report it if it repeats.
- "ffmpeg was changed after install; reinstall it.": select **Reinstall**.
- Report: the step, the **In use:** line, the message, and the import sheet's text for a clip
  that did not verify.

## 14. Switch map, position by position

Reads the switch map off a radio's and an FC's backups, and checks it against the real radio and
quad. Writes nothing.

**Needs**

- An aircraft profile that names the radio and the FC, both saved, each with a backup (trials 1
  and 3).
- For the FC part of **Live**: the quad bound to the radio. Props **off**, battery in, and the
  USB timer on.

**Steps**

1. Open the FC's page and choose **Switches**. Read the tiles: **Controls**, **Modes**,
   **Conflicts** and **Not mapped**.
2. Put the radio in USB Joystick mode and plug it in. Under **Live** choose **Radio**.
3. Move each switch through each position, one at a time. Compare the marked position and its
   modes with what you set up on the radio and in Betaflight.
4. Plug in the FC, put the battery in (props off), and choose **FC** under **Live**. Move each
   switch again.
5. Read **Conflicts** and **Not mapped**.

**Pass**

- Each switch shows its type (2-position, 3-position or toggle) and each position's channel
  value in µs, its Betaflight modes, its adjustments, and the radio's own effects (logical
  switches, special functions, timers).
- With **Radio** and with **FC**, the marked position follows the switch within a second. The
  modes shown on are the ones the quad turns on (check arm and the flight modes in Betaflight
  Configurator's Modes tab if in doubt).
- A combination such as "down with SB down" shows only when two switches together do more.
- **Conflicts** lists only real problems: two modes on one range, a switch that does nothing, a
  mode no switch reaches, a sound file the card lacks.

**If it fails**

- "No radio in USB Joystick mode.": choose **USB Joystick** on the radio.
- **FC** is not offered: the FC is not plugged in, or another program holds its port.
- A position shows the wrong mode: note the switch, the position, what QuadCam shows and what
  the quad does.
- Report: the radio and FC models and versions, the switch and position, the model's mix and
  logical switch lines for that switch (from the backup's `MODELS/` file), and the FC's `aux`
  and `adjrange` lines.

## 15. OSD render against the goggles

Checks that QuadCam draws the OSD where the goggles show it, for each OSD profile. Writes
nothing.

**Needs**

- A saved FC with a backup (trial 3), and goggles that show its OSD.
- Props **off**, the battery in to power the video, and the USB timer on.

**Steps**

1. Open the FC's page and choose **OSD**. The line at the top says "From backup · <date>".
2. Leave **Grid** on **Auto**, or pick the system the goggles use (**NTSC**, **PAL** or **HD**).
3. Power the quad and put the goggles on. Switch the quad to OSD profile 1.
4. Compare each element's place with the screen in **OSD profile** 1.
5. Repeat for profiles 2 and 3, if the quad uses them.

**Pass**

- Each element sits on the same row and column as in the goggles. Elements draw as sample
  text, not Betaflight's font, and widths are estimates: an element one or two cells wider or
  narrower is fine.
- Each profile shows the elements the goggles show in that profile.
- A real overlap or an element off screen shows in **Problems**. With none, the badge says
  **No problems**.

**If it fails**

- The grid is wrong: `vcd_video_system` is `AUTO`. Pick the grid by hand.
- An element is in the wrong place: note the element, the profile, QuadCam's row and column, and
  where the goggles show it.
- Report: the board, the Betaflight version, the video system, the goggles, and a photo of the
  goggle screen next to a screenshot of QuadCam. Crop out anything personal.

## 16. Cues

Checks that each cue plays once, at the right time, on each channel.

**Needs**

- A card QuadCam knows (DVR card or radio).
- For **Unplug now**: a saved FC, props **off**, a battery.

**Steps**

1. Open **Settings > Gear > Cues**. Leave **Mute all cues** off. Under **Play cues as** turn on
   **Speech**, **Sound** and **Notification**. Leave **Voice** on **macOS**. Select **Done**.
2. Plug in the card and run a job: **Check Card** or **Back Up Now**.
3. Leave the card in after "done". Wait 60 seconds, then 5 minutes.
4. Pull the card. Plug it in again and pull it while nothing runs.
5. Set **USB power warning** to 1 minute. Plug in the FC, put the battery in, and wait. Set the
   warning back to your value (20 by default) after.
6. Turn on **Quiet hours for speech and sound**, with **From** and **To** around the current
   time. Run a job again. Turn quiet hours off after.
7. Turn on **Mute all cues** and run a job. Turn it off after.

**Pass**

- Step 2: once the job ends, the Mac says "<device> done, safe to unplug.", plays a sound, and
  posts a notification. The first notification asks to allow notifications.
- Step 3: "<device> is still inserted." after 60 seconds, then every 5 minutes, 3 times at
  most. **Dismiss Reminder** on the device page stops it.
- Step 4: pulling the card plays nothing. QuadCam's own mounts and unmounts play nothing.
- Step 5: "Unplug <FC> now. It has run on USB with a battery for …" plays once for that
  battery.
- Step 6: no speech and no sound, only the notification.
- Step 7: nothing plays.
- **Step failed** ("<step> failed on <device>.") plays when a step fails in any trial. Note it
  when it happens.

**If it fails**

- No notification: allow QuadCam in **System Settings > Notifications**.
- A cue plays twice for one job, or the reminder goes past 3: note the times.
- Report: the cue, the channel, the setting values, and the times it played.

The **Voice pack** choice under **Voice** is off: cues speak with the macOS voice in this
release.

## 17. One full bench session in QuadCam only

The 1.0 gate. Run one real session from bench to analysis with QuadCam only: no Betaflight
Configurator, no EdgeTX Companion, no Finder copies. Do it after the other trials you can run.

**Needs**

- A radio and an FC, saved, linked by an aircraft profile, both proven for writes (see
  [Before you start](#before-you-start)).
- One or more staged changes for the session: a harmless FC change (trial 7), a radio model
  edit (trial 8), or both. Mark each **Ready** or **Try** on the **Bench**.
- Packs and a pack type set up on the **Packs** page. A DVR or goggles card.

**Steps**

1. **Before.** Open **Pack up**. Fix each row that does not pass: **Packs charged** (**Mark
   charged** on **Packs**), **Radio model**, **Card space**, **Backups**, **Cards out**.
2. **Back up.** Plug in the radio (USB Storage) and the FC (battery out). Let the on-connect
   backups run, or select **Back Up Now** on each.
3. **Apply.** Open **Bench**. **Next session** names the first change. For each change select
   **Review…**, read **Checks**, and select **Apply**. Wait for "done, safe to unplug." each
   time.
4. **Verify.** Read each change's result on the **Bench**. Check one value on the device: the
   name on the radio, the setting in **Overview** or a fresh backup.
5. **Fly.** Fly the session with the DVR or goggles recording, and the radio logging.
6. **Keep or revert.** For each applied Try change, select **Keep** or **Revert…**.
7. **Import.** Plug in the radio (USB Storage) and the card. Import the clips with the radio's
   logs, and finish. Select **Show Report** in the Finish step.
8. **Analyse.** On **Flights** pick each flight's pack. Read hover, sag, resting voltage, used
   mAh, worst link and dropouts. Read the **Session report**.
9. **After.** Pull the blackbox if you fly with one (trial 4). Open **Pack up** again.

**Pass**

- Every step ran in QuadCam. You did not open another tool.
- Each change ends **Verified** or **Reverted**, and the Bench count drops to 0 or to what you
  left for later.
- The clips are dated from the radio logs and verify.
- The flights match the session: their count, air time, and each pack you flew.
- The report's numbers agree with what you saw: no dropout you did not have, none missing.

**If it fails**

- Note the step where you left QuadCam, and why. That gap is a 1.0 bug.
- Report: each step's result, the changes and their final status, the flight count against the
  packs flown, and the session report (**Copy as Markdown**, with places and names removed).

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
