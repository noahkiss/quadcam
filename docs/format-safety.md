# Format safety

The format step erases a disk. QuadCam makes it hard to erase the wrong one.

You do not have to use it. If your goggles can format the card from their menu, that works too.

## When the button unlocks

The **Format card** button unlocks only when all of these are true:

- The clips came from a card, not from a folder.
- Every clip copied off the card.
- Every clip that you did not skip verified in the output folder.

The video system of the card must also allow a format. Analog DVR cards allow it.

QuadCam never formats a DJI volume: not an air unit over USB, and not a goggles card. The Finish step shows no **Format card** panel for DJI clips, and the command line and the MCP server refuse with exit code 3. Format a DJI card in the goggles.

You can also format a card that has no session. See [Card prep](#card-prep).

Formatting is not the only way to free a card. **Delete clips after import** (a setting, off by default) deletes only the clip files that verified, on DJI and analog cards alike, and keeps every other file. It never formats and never changes the file system. See [Settings](settings.md#delete-clips-after-import).

## The guards

Immediately before it erases, QuadCam reads the disk information again and checks every guard:

- The disk is not internal, and it is removable or ejectable.
- The disk is not the boot disk and not `disk0`.
- The disk is 64 GB or smaller.
- The disk is the same card that the clips came from: same device and same volume UUID.
- The volume is not a radio's SD card (`LOGS/` next to `MODELS/` or `RADIO/`).
- The volume is not a DJI card. A `DCIM/DJI_*` folder makes it one, even when it is empty.
- The video system of the card allows a format.

Then QuadCam runs `diskutil eraseDisk FAT32 <NAME> MBRFormat /dev/diskN` on the whole card, and ejects it at once.

## Card prep

Card prep formats a card that has no session behind it: a new card, or a card whose clips are all in the library. Use it to make a spare DVR card ready.

Card prep checks every guard above. It also checks that no clip on the card is missing from the library. QuadCam knows a clip by its content, not by its name. A clip that is only loaded in an import does not count. Import each missing clip first. QuadCam checks this again immediately before it erases. For card prep, "the same card" is the card that the plan named.

A card with no clips needs no library. QuadCam formats it as FAT32 for an analog DVR.

Card prep never formats a DJI card. Format a DJI card in the goggles.

Card prep has no button in the app yet. Use the command line (`format --prep`) or an agent (`quadcam_format_card` with `prep=true`). If the app is running, you must still select **Erase** in the app.

## Confirmation

Each way in adds its own confirmation:

| From | Confirmation |
|---|---|
| The app | A dialog names the disk, the volume, the size, and the clip count. Only a click on **Erase** starts the erase. The Return key does not. |
| The command line | `--device`, `--volume-uuid`, and `--yes` must all be given, and they must match the card. |
| An agent (MCP) | The agent must give the device, the volume UUID, and `confirm=true`. If the app is running, you must also select **Erase** in the app. |

QuadCam never saves the format choice as a setting.
