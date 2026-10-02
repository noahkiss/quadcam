# Format safety

The format step erases a disk. QuadCam makes it hard to erase the wrong one.

You do not have to use it. If your goggles can format the card from their menu, that works too.

## When the button unlocks

The **Format card** button unlocks only when all of these are true:

- The clips came from a card, not from a folder.
- Every clip copied off the card.
- Every clip that you did not skip verified in the output folder.

The video system of the card must also allow a format. Analog DVR cards allow it.

## The guards

Immediately before it erases, QuadCam reads the disk information again and checks every guard:

- The disk is not internal, and it is removable or ejectable.
- The disk is not the boot disk and not `disk0`.
- The disk is 64 GB or smaller.
- The disk is the same card that the clips came from: same device and same volume UUID.

Then QuadCam runs `diskutil eraseDisk FAT32 <NAME> MBRFormat /dev/diskN` on the whole card, and ejects it at once.

## Confirmation

Each way in adds its own confirmation:

| From | Confirmation |
|---|---|
| The app | A dialog names the disk, the volume, the size, and the clip count. Only a click on **Erase** starts the erase. The Return key does not. |
| The command line | `--device`, `--volume-uuid`, and `--yes` must all be given, and they must match the card. |
| An agent (MCP) | The agent must give the device, the volume UUID, and `confirm=true`. If the app is running, you must also select **Erase** in the app. |

QuadCam never saves the format choice as a setting.
