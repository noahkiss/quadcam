# Format safety

The format step erases a disk. QuadCam makes it hard to erase the wrong one.

You do not have to use it. If your goggles can format the card from their menu, that works too.

## When the button unlocks

The **Format card** button unlocks only when all of these are true:

- The clips came from a card, not from a folder.
- Every clip copied off the card.
- Every clip that you did not skip verified in the output folder.

The video system of the card must also allow a format. Analog DVR cards allow it.

DJI has two rules:

- QuadCam never formats a DJI device over USB: an O4 air unit, or goggles in storage mode. This rule has no exception.
- After a DJI import, the Finish step shows no **Format card** panel. A removable DJI goggles card, in a card reader or the Mac's SD slot, can be erased with [card prep](#card-prep) once every clip on it is in the library. QuadCam formats it as exFAT. You can also format it in the goggles.

You can also format a card that has no session. See [Card prep](#card-prep).

Formatting is not the only way to free a card. **Delete clips after import** (a setting, off by default) deletes only the clip files that verified, on DJI and analog cards alike, and keeps every other file. It never formats and never changes the file system. See [Settings](settings.md#delete-clips-after-import).

## The guards

Immediately before it erases, QuadCam reads the disk information again and checks every guard:

- The disk is not internal storage, and it is removable or ejectable. A card in the Mac's built-in SD slot counts as removable: macOS calls the slot internal, but the card in it is removable media on the Secure Digital bus.
- The disk is not the boot disk and not `disk0`.
- The disk is the same card that the clips came from: same device and same volume UUID.
- The volume is not a radio's SD card (`LOGS/` next to `MODELS/` or `RADIO/`).
- The volume is not a DJI device over USB. A `DCIM/DJI_*` folder (even an empty one) makes the volume DJI. A DJI volume is refused when the disk's media name names DJI, or when a DJI USB device is attached and the card is not in the Mac's SD slot.
- The video system of the card allows a format.

QuadCam never refuses a card for its size. When a card is larger than the card's DVRs usually read, the confirmation says so. For an analog card over 32 GB: "Most analog DVRs take cards up to 32 GB; this DVR may not read it."

Then QuadCam runs `diskutil eraseDisk <FS> <NAME> MBRFormat /dev/diskN` on the whole card. `<FS>` is the file system of the card's video system: FAT32 for an analog DVR, exFAT for DJI goggles. It then unmounts the card at once, so the card is safe to remove. FAT32 is tested on real cards; exFAT is tested on disk images only.

## Card prep

Card prep formats a card that has no session behind it: a new card, or a card whose clips are all in the library. Use it to make a spare DVR card or a goggles card ready.

Card prep checks every guard above. It also checks that no clip on the card is missing from the library. QuadCam knows a clip by its content, not by its name. A clip that is only loaded in an import does not count. Import each missing clip first. QuadCam checks this again immediately before it erases. For card prep, "the same card" is the card that the plan named.

A card with no clips needs no library. QuadCam formats it as FAT32 for an analog DVR. A card with `DCIM/DJI_*` folders, even empty ones, is a DJI goggles card: QuadCam formats it as exFAT.

Card prep has a button in the app, a command and an agent tool:

- **App:** open the card's page in **Gear** and select **Prepare card…**. After a DJI import, the Finish step has a **Prepare card** panel with the same button. A dialog names the disk, the volume, the size, the file system and any size advice. Only a click on **Erase** starts the erase. A card that is unmounted but still in is mounted for the plan and the erase, and unmounted again.
- **Command line:** `format --prep`. Name the card with `--mount` (a mounted card) or `--card` (its device id from `gear status`), then give `--device`, `--volume-uuid` and `--yes`.
- **Agent:** `quadcam_format_card` with `prep=true`. If the app is running, you must still select **Erase** in the app.

All three run the same guards in `disk::format_card`, right before the erase.

## Confirmation

Each way in adds its own confirmation:

| From | Confirmation |
|---|---|
| The app | A dialog names the disk, the volume, the size, the file system, the clip count, and any size advice. Only a click on **Erase** starts the erase. The Return key does not. |
| The command line | `--device`, `--volume-uuid`, and `--yes` must all be given, and they must match the card. |
| An agent (MCP) | The agent must give the device, the volume UUID, and `confirm=true`. If the app is running, you must also select **Erase** in the app. |

QuadCam never saves the format choice as a setting.
