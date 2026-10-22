# Radios: design

Status: draft for review, 2026-10-22.
Baseline: `main` at release 0.22.0.
Target: QuadCam sets up and flashes EdgeTX radios with no outside script, in the stages of
section 6.

The reference radios are a RadioMaster Pocket (EdgeTX 2.12.4) and a RadioMaster GX12 (EdgeTX
2.12.5, ELRS 4.1.0, flashed by hand on 2026-10-22). Today three scripts outside
QuadCam do the radio work: a card patcher (the switch layout, sounds, timers, telemetry
screens), an EdgeTX build-and-flash script (the splash and DFU), and an ELRS flash script. The
owner wants to set the switches in the app. This doc is the plan for that:

1. the board profile: the facts QuadCam holds per radio board, and the traps they guard;
2. the flash done by hand on 2026-10-22, step by step, as the reference QuadCam must
   reproduce;
3. the in-app radio setup: everything the card patcher does, what QuadCam has, what is
   missing, and the switch-assignment editor;
4. the stages, each one shippable, with acceptance checks.

It extends [Gear design](gear-design.md): section 6.3 (EdgeTX YAML), 6.5 (DFU), 7.2 (switch
map) and 7.5 (splash and radio firmware). The rules there hold here too: one path for every
write (8.1), a plan and a digest before a write, a backup first, a read-back after.

---

## Contents

1. [Work in flight](#1-work-in-flight)
2. [Board profiles](#2-board-profiles)
3. [Flashing by hand, 2026-10-22](#3-flashing-by-hand-2026-10-22)
4. [In-app radio setup](#4-in-app-radio-setup)
5. [The switch-assignment editor](#5-the-switch-assignment-editor)
6. [Stages](#6-stages)
7. [Open questions for the owner](#7-open-questions-for-the-owner)

---

## 1. Work in flight

Stages 1, 2, 3 and 7 (section 6) shipped on `main` on 2026-10-22. This doc does not repeat
their scope; it states what the later stages need from them.

| Package | Stage | Commits | Scope |
|---|---|---|---|
| GX12 flash | 1 | `1734cb1`, `3b547ad` | The board profile table in Rust; `gx12` proven for the EdgeTX flash, firmware read and restore; the DFU instruction from the profile; the ELRS TX target picked from the radio's board; the UF2 and last-series refusals |
| GX12 card | 2 | `3294953`, `c978c3c` | GX12 card read and write (its own YAML schema); `gx12` proven for card writes; a model copied across boards through a per-board control map (`gear/edgetx/board.rs`) |
| Radio setup | 3 | `5f448a5`, `dd41217` | The setup as data, its generator, rows, CLI and MCP (section 6, stage 3 note) |
| Other boards | 7 | this package | 50 board profiles as read-only data; the profile on a detected radio; the guards of 2.4 (section 2.5) |

## 2. Board profiles

### 2.1 Fields

A board profile is the data QuadCam holds for one EdgeTX target (the `board:` value in
`radio.yml`, and the firmware file prefix). One table in Rust holds every profile. Code reads
the table; it never tests a board name inline. A board not in the table is read-only, as an
unproven board is today.

"Data" means the field is in the table. "Read" means QuadCam reads it from the radio and does not
store it. "Doc" means people need it but no code branches on it.

| Group | Field | Kind | What QuadCam does with it |
|---|---|---|---|
| Identity | `board` (target) | Data | The key. Matched to `radio.yml` `board:`, to `INFO_UF2.TXT` `Board-ID:` on UF2 radios, and to the `edgetx-<board>-<version>` string in an image |
| | Maker and model name, variants that share the firmware | Data | Labels |
| | USB product pattern per mode | Data | Tells the radio apart in Storage, Serial, Joystick and bootloader modes. Matched case-insensitively |
| | Look-alike boards that take another firmware | Data | A warning in the flash plan: "the case does not decide the firmware" |
| | Target history (a name reused for another radio) | Data | A version floor under which the target means another radio (`v12`) |
| Chip | MCU, internal flash size | Data | The DFU layout check (`flash_bytes`), the size band of an image |
| | Last EdgeTX series | Data | Refuses a newer series (`F2` boards stop at 2.11) |
| Firmware | Format: `bin` or `uf2` | Data | `bin`: DFU write. `uf2`: refused until a UF2 path exists |
| | ROM DFU starts on plug-in while off | Data | Picks the DFU instruction |
| | DFU entry, as the steps the UI shows | Data | The flash sheet, the Read firmware steps, the recovery steps |
| | EdgeTX bootloader entry | Data | Recovery steps |
| | Image layout: bootloader size, size band | Data | `check_image` |
| | Factory firmware and bootloader strings | Doc | Spot an untouched radio |
| | Maker's own firmware fork, and what upstream loses | Data | A warning before the first flash |
| Storage | Kind: card slot, soldered chip, chip a card overrides | Data | Picks the write route (2.3) |
| | Measured USB Storage speed | Data | The time estimate on every write plan |
| | EdgeTX SD-card pack (`bw128x64`, `c480x272`, ...) | Data | The storage-pack update |
| | YAML schema family | Data | Which field set the card engine accepts and writes |
| USB | Ports, and which one carries data | Data | Instruction text |
| | Advanced joystick mode | Data | The Controls page and the sim |
| Screen | Black-and-white or colour, resolution | Data | Checklist line length, splash mechanism, Lua screens |
| | Splash mechanism: in the firmware, or `/IMAGES/splash.png` | Data | The splash flow |
| Internal RF | ELRS target, firmware name, Lua name | Data | The ELRS flash plan; the version read |
| | ELRS upload methods (`etx`, `wifi`, `uart`) | Data | Refuses a method the target lacks |
| | Lowest ELRS version for the target | Data | Refuses an older release |
| | CRSF baud index (`internalModuleBaudrate`) | Read | Shown; never changed |
| | Max power | Doc | Shown |
| Controls | Switches with type, buttons, pots, sliders, trims | Data | The control inventory (5.2), the defaults of `switchConfig` and `potsConfig` |
| | Customizable switches (SW1-SW6) and RGB LEDs | Data | The editor's extra controls |
| | Gimbal interface: analog or serial | Data | Serial gimbals: QuadCam never offers stick calibration |
| Power | Battery kind, `vBatMin` and `vBatMax` | Read | Shown |
| | Power-on method, what plug-in while off does | Data | Instruction text |
| Quirks | Known bugs per firmware, each with what QuadCam does | Data | Warnings and guards (2.4) |

### 2.2 The two radios the owner has

Values checked on the radios on 2026-10-07 (Pocket) and 2026-10-22 (GX12), and in the EdgeTX
source at `v2.12.5`.

| Field | `pocket` | `gx12` |
|---|---|---|
| Maker, model | RadioMaster Pocket (and Pocket Crush) | RadioMaster GX12 (and GX12 Crush) |
| USB product, Storage | `<radio> Mass Storage`, name from EdgeTX's hardware file | `Radiomaster GX12 Mass Storage` (lower-case m on the unit; EdgeTX's hardware file says `RadioMaster`) |
| MCU, flash | STM32F407xE, 1 MB | STM32F407xG, 1 MB |
| Last EdgeTX series | current (2.12) | current (2.12) |
| Format | `bin` | `bin` |
| ROM DFU on plug-in while off | yes | **no** |
| DFU entry | Radio off. Plug USB in. Hold nothing | Radio off. USB in the top port. Hold **SG**, tap power, release SG. The screen stays dark |
| Bootloader entry | Both trim hats inward, then power | Both horizontal trims inward, then power |
| DFU device | `0483:df11`, `STM32  BOOTLOADER`, alt 0 `@Internal Flash /0x08000000/04*016Kg,01*064Kg,07*128Kg` | The same (ROM bootloader, same layout) |
| Image | Bootloader 32 KB (`0x8000`), 400-1000 KB | The same layout; the 2.12.5 image is 552,588 bytes |
| Storage | microSD slot | 512 MB soldered SD-NAND chip on SPI, no slot. FAT16 |
| Fast write route | Pull the card into the Mac's reader | None: USB Storage only |
| USB Storage speed | 0.30 MB/s write, 0.5 MB/s read | 0.36 MB/s write, 0.37 MB/s read |
| SD-card pack | `bw128x64` | `bw128x64` |
| YAML schema | `128x64` family | Its own (`gx12`) |
| Screen | 128 x 64 B&W LCD | 128 x 64 B&W OLED |
| Splash | In the firmware (`SPS\0` 0x80 0x40, 1,024 bytes, `SPE`) | The same; markers found once in the 2.12.5 image |
| Joystick | Classic only (F407xE) | Advanced available |
| ELRS target | `radiomaster.tx_2400.pocket`, `Unified_ESP32_2400_TX`, Lua `RM Pocket`, SX1280, 250 mW | `radiomaster.tx_dual.gx12`, `Unified_ESP32_LR1121_TX`, Lua `RM GX12 X-Band`, 2x LR1121 (2.4 GHz and 900 MHz at once), 1 W, fan |
| ELRS upload methods | `uart`, `wifi`, `etx` | `etx`, `wifi` (no `uart`) |
| Lowest ELRS | unknown (older than 3.0) | 3.6.0 |
| ELRS chip | ESP32 | ESP32-PICO-D4, 4 MB |
| CRSF baud index | as found | 2 (921600) |
| Switches | SA, SD 2-pos latching buttons with LED; SB, SC 3-pos toggles on the top edge; SE momentary | SA 2-pos, SB 3-pos, SC 3-pos, SD 2-pos, SE 3-pos, SF 3-pos, SG and SH momentary (`toggle`) |
| Customizable switches | none | SW1-SW6, set per model, RGB LEDs (eight LEDs: SW1-SW6, SA, SD) |
| Pots, sliders | S1 wheel (EdgeTX `P1`, no detent) | P1 and P2 (centre detent), sliders SL1 and SL2 |
| Trims | 4 | 4 |
| Gimbals | Hall, analog: calibrate on the radio | GX01 hall, serial, factory calibrated: never offer calibration |
| Power | 2x 18650; power button | 2S; power button; plug-in while off charges |
| Quirks | The factory `radio.yml` had SA, SD and SE types inverted (fixed by hand 2026-09-27) | Switches can freeze behind the I2C expander (EdgeTX #6159; PR #6325 adds a 100 ms poll timer); a large custom `hello.wav` can freeze them at boot on 2.12.4 |

### 2.3 Write routes

Storage kind decides the route. QuadCam already writes only files whose bytes change, and
already estimates time at a floor speed (gear-design 6.3).

| Storage | Route | What the plan shows |
|---|---|---|
| Card slot (Pocket) | Pull the card into the Mac's reader for big writes (a voice pack). USB Storage for small edits | "Faster with the card in a reader" when the write is over 5 MB |
| Soldered chip (GX12) | USB Storage only, about 0.36 MB/s | The bytes to write and a time estimate from the profile's measured speed. A 66 MB voice pack takes about 3 minutes; a model edit takes seconds |
| Chip a card overrides (H7 boards) | A card at boot hides the chip | Refused until a radio of this kind is tested |

On a chip radio a full voice pack is the slow case. The voice pack apply already writes only
lines that differ; the plan must say so in its estimate.

### 2.4 Traps the profiles guard

Each trap names the guard. A guard refuses before any download or write.

| Trap | Guard |
|---|---|
| The GX12 does not enter ROM DFU on plug-in while off. Two waits of 10 and 30 minutes saw no DFU device until the owner held SG and tapped power | The DFU steps come from the profile, never from shared text (`dfu.steps`, `radios::dfu_hint`). A board whose way in no source gives has `DfuStart::Unknown`: no steps, and the hint says QuadCam does not know |
| A `.uf2` board (every H7: TX15, GX15, TX16S MK3, Jumper T15 Pro, T15-H7, T18 H7 and T22, HelloRadioSky V12 and V15, iFlight Commando 14, Flysky PA01 and ST16) cannot be flashed with a plain DFU tool. Its firmware lives in external QSPI flash; ST's tools cannot recover it | `firmware_format: Uf2` refuses the flash and names the UF2 route: start the EdgeTX bootloader (the profile's entry), copy the board's `.uf2` to the `EDGETX_UF2` drive, or use EdgeTX Buddy or Companion 2.12+ |
| STM32F2 boards stop at EdgeTX 2.11 (RadioMaster TX12 MK1, Jumper T12, T-Lite, T-Pro V1, BetaFPV LR3 Pro, FrSky X9D, X9D+, X-Lite, X-Lite S) | `edgetx_series: "2.11"` on every F2 row (a test ties it to the MCU): a 2.12 release refuses |
| EdgeTX reused the target `v12`. Before 2.12.3 it named an F407 B&W radio that was never built; from 2.12.3 it is HelloRadioSky's H7 colour V12. The old firmware bricked a V12 Max | `target_history` (`until: 2.12.3`): `v12` below 2.12.3 refuses before any other check, and the profile's warnings say why |
| Look-alike radios take another firmware: Jumper T-Lite V2 runs as F2 (there is no T-Lite F4 firmware); T20 and T20 V2 differ in flash size; T-Pro firmware mirrors a T-Pro V2's screen; HelloRadioSky V14 (OLED) and V14 LCD are `v14` and `v14lcd`; TX12 MK1 and MK2; T15 and T15-H7; X9D+ and X9D+ 2019 | `look_alikes` on both rows, shown as warnings. Never infer the board from a name or case: read `board:`, or `Board-ID:` on a UF2 drive. A USB product name maps to a board by the longest match (`Jumper T20 V2` is not `Jumper T20`) |
| Makers ship their own firmware forks (HelloRadioSky); upstream drops their features | `fork` on the five HelloRadioSky rows, shown as a warning on the radio and in `radio_boards`. Those boards are read-only, so no flash plan reaches them yet |
| Every production radio has full-speed USB (12 Mb/s), H7 included | `usb_speed` holds a measured speed (Pocket, GX12) or None, never an assumed faster one |
| A chip radio has no card to pull | Write route by storage kind (2.3) |
| Serial gimbals are factory calibrated (GX12) | No calibration prompt; the Settings editor hides calibration |
| A large or high-rate `hello.wav` can freeze the GX12's switches at boot (2.12.4 reports) | The voice pack apply on a GX12 checks `SYSTEM/hello.wav`: refuse over a size cap, or over the pack's normal sample rate. The owner's own pack keeps it short |
| ELRS upload methods differ per target. The GX12 module has no UART route | The ELRS plan offers only the target's methods; QuadCam uses `etx` |
| ELRS Lua must match the module's major version. A forced target mismatch left a TX15 module reporting V0.0.0 | After a TX flash, the plan's next step installs `elrs.lua` and removes `elrsV3.lua*` |
| ROM DFU and the H7 bootloader DFU share `0483:df11` | Tell them apart by the product string |
| The GX12 factory `radio.yml` (2.11) writes switch types in lower case (`2pos`, `3pos`, `toggle`); the Pocket's file has upper case. EdgeTX reads both. `switchmap::switch_types` compares upper case only, so a GX12 `2pos` reads as 3-position | Read types case-insensitively. Write the case the file already uses |
| On 2.12.5 the GX12 model keys `functionSwitchConfig`, `functionSwitchGroup`, `functionSwitchStartConfig`, `functionSwitchLogicalState`, `switchNames` and the LED colour lists are read-only: EdgeTX converts them and writes `customSwitches:` and `cfsGroupOn:` instead (`yaml_datastructs_gx12.cpp`, v2.12.5) | Read both forms. Write only `customSwitches:` (5.4) |
| A model file keeps its old `semver` until the radio saves it. After the 2.12.5 flash, `model00.yml` still said 2.11.0 | The version guard reads `radio.yml` `semver`. A model's own `semver` is shown, not gated on |

### 2.5 Other boards

The research of 2026-10-22 covers the RadioMaster, Jumper and HelloRadioSky lines and the other
EdgeTX makers (BetaFPV, iFlight, Flysky, FrSky). Stage 7 put its values into the table as data,
unproven: 50 rows in `gear/radios/boards.rs`. Until a board is proven its radios are read-only:
QuadCam shows the profile, the DFU steps and the traps, and refuses writes and flashes. The ELRS
flash refuses an unproven board's internal module too.

What shipped:

- **Sources.** The chip, the USB name (`<maker> <model>`), the screen, the switch and pot slots
  and the trims come from EdgeTX's hardware definitions (`radio/src/boards/hw_defs/*.json`, main
  on 2026-10-22). The SD-card pack comes from `edgetx-sdcard` `sdcard.json` (v2.11.5 for the F2
  boards); the YAML schema from `yaml_datastructs.cpp` at v2.12.5; the internal ELRS targets
  from ExpressLRS `targets.json`. DFU and bootloader entry, storage, ports and quirks come from
  the research. A value no source gave is None (`bootloader_entry`, `usb_ports`, `usb_speed`,
  `internal_rf`) or `DfuStart::Unknown`; nothing is guessed.
- **Fields added to 2.1's list:** `variants`, `look_alikes`, `target_history`, `fork`,
  `usb_speed`, `sd_pack`, `yaml_schema`, `slots` (each switch and pot slot with what the
  hardware reads and EdgeTX's default type; the fitted type stays `radio.yml`'s), `trims`,
  `usb_ports`, `internal_rf`, `quirks`. `elrs` became a list: a board sold in 2.4 GHz and
  900 MHz versions (Jumper T20, T14, T-Pro) has one target per version, and the ELRS plan takes
  a board's target only when there is one.
- **Detection.** `Connected.radio` carries the profile of a radio volume (from `radio.yml`'s
  `board:`, else the USB product name), a radio serial port (its product name) and a DFU device
  linked to a saved radio, with whether it is proven, its DFU steps and its warnings. A product
  name a profile knows marks a USB device as a radio (`detect::is_radio_usb`).
- **Reach.** `gear_radio_boards` (CLI `gear radio-boards`, MCP `quadcam_gear` `radio_boards`)
  lists one board, the board a USB product name names, or every board. `quadcam_gear` `status`
  names a radio's profile on its line.
- **DFU entry unknown** on `tpros`, `t15pro`, `t15h7`, `t18h7`, `t22`, the five HelloRadioSky
  boards, `c14`, `pl18u` and the FrSky boards. The DFU hint says so; the flash refuses these
  boards anyway.

| Line | Boards | Notes for the table |
|---|---|---|
| RadioMaster | `tx16s`, `tx16smk3`, `tx15`, `gx15`, `gx12`, `tx12`, `tx12mk2`, `zorro`, `boxer`, `pocket`, `mt12` | H7 (`tx16smk3`, `tx15`, `gx15`): `uf2`, DFU by tapping SYS. `tx12`: F2, last 2.11. `mt12`: bootloader trims T1 and T2, USB on the side |
| Jumper | `tlite`, `tpro`, `tprov2`, `tpros`, `bumblebee`, `t12`, `t12max`, `t14`, `t20`, `t20v2`, `t15`, `t15pro`, `t15h7`, `t22`, `t18h7`, `t16`, `t18` | Chip storage like the GX12: `t20`, `t20v2`, `t14`, `t12max`, `tpros`, `bumblebee`. Boot0 or DFU buttons on most. Early `t22`: no ROM DFU |
| HelloRadioSky | `v12`, `v14`, `v14lcd`, `v15`, `v16` | `v12` name reuse. V12 and V15 power on with SYS+MDL; plug-in while off charges. V14 has separate data and charge ports |
| Others | `lr3pro`, `commando8`, `c14`, `pl18`, `pl18ev`, `pl18u`, `el18`, `nv14`, `pa01`, `st16`, `nb4p`, `x9d`, `x9dp`, `x9dp2019`, `xlite`, `xlites`, `x7access` | `lr3pro`, `x9d`, `x9dp`, `xlite`, `xlites`: F2. PL18 family and NB4+: soldered SPI flash. PA01, ST16, C14: `uf2` |

## 3. Flashing by hand, 2026-10-22

This is the session QuadCam's GX12 flash and ELRS flash must reproduce. Each step names what
the app checks. Where QuadCam already does the step for the Pocket, the table says so.

### 3.1 EdgeTX 2.11.0 to 2.12.5 with the splash

| # | Step by hand | What the app must check or do | QuadCam today |
|---|---|---|---|
| 1 | Copied the whole factory storage to the Mac over USB Storage | A card backup exists and is newer than the last write. On a chip radio this is the only copy of the models | Done (the flash plan requires a backup) |
| 2 | Downloaded `edgetx-firmware-v2.12.5.zip` and matched its SHA-256 to the release | The release digest, else the digest recorded at the first download | Done |
| 3 | Took `gx12-<hash>.bin` from the zip | One file matches the board's names. The image names `edgetx-gx12-2.12.5` | Pocket only: `BOARDS` lists `pocket` |
| 4 | Patched the splash: found `SPS\0` once, 0x80 0x40, 1,024 bytes, `SPE`; decoded it back | The same marker checks as the Pocket. The `Splash` pair (`gx12`, 2.12.5) proven | Pocket 2.12.4 only |
| 5 | Updated the storage pack from 2.10 to the 2.12.3 `bw128x64` pack, and put a spare copy of the bin in `FIRMWARE/` | Offer the storage-pack update when `edgetx.sdcard.version` is older than the firmware needs. Write only changed files. Optional: a fallback bin in `FIRMWARE/` for the bootloader route | Missing |
| 6 | Asked the owner to enter DFU: radio off, USB in, hold SG, tap power, release SG. Plain plug-in did not work | The profile's DFU steps. Wait for exactly one `0483:df11` device with Stop. Say that the screen stays dark | Pocket text |
| 7 | Read the full 1 MB flash (`dfu-util -a 0 -s 0x08000000:1048576 -U`) | `dfu::read_verified`: two equal reads, not all zeros, saved and read back from disk. The current image names `gx12` | Done for any board in `BOARDS` |
| 8 | Wrote the image at `0x08000000` and left DFU (`-s 0x08000000:leave -D`) | The layout string gives 1 MB. 2,048-byte transfers. Segment read-back, full read-back, leave | Done for any board in `BOARDS` |
| 9 | The radio restarted. The owner chose USB Storage | Ask the owner to power-cycle if the screen stays dark | Done (text) |
| 10 | Read `radio.yml`: `semver: 2.12.5`, `board: gx12` | Read the version in Storage mode and record it on the saved radio. Link the DFU serial to the radio | Partly: the report leaves the read to the person |
| 11 | Copied `elrs.lua` into `SCRIPTS/TOOLS/` and deleted `elrsV3.lua` and `elrsV3.luac` | A post-flash step, staged as a card change: the Lua that matches the module's ELRS major | Missing |
| 12 | Renamed the volume from `NO NAME` to a short label for the radio | Offer a volume label from the radio's saved name (`diskutil rename`). Labels help a person; QuadCam's identity stays the card markers | Missing |

### 3.2 ELRS 4.1.0 on the GX12's internal module

| # | Step by hand | What the app must check or do | QuadCam today (preview) |
|---|---|---|---|
| 1 | The radio's `serialPort: VCP: mode: CLI` was already set at the factory | Read it from the backup's `radio.yml`. If it is not `CLI`, stage that `radio.yml` change first | Missing as a check |
| 2 | The owner replugged with the radio on and chose USB Serial (VCP) | Wait for one `0483:5740` port whose product names the radio | Done |
| 3 | Picked the target `radiomaster.tx_dual.gx12` from the release's `hardware/targets.json` | Target from the profile; product `RadioMaster GX12 2.4/900 TX`; firmware `Unified_ESP32_LR1121_TX`; upload methods include `etx` | Missing: `elrs/flash.rs` knows `Unified_ESP32_2400_TX` and `Unified_ESP8285_2400_RX` only |
| 4 | Built the image with the binding phrase and auto-WiFi 60 s; read the options block back: UID = MD5 of the phrase, first 6 bytes | `elrs/image.rs` writes and reads the block. Check that the LR1121 target's layout has no `logo_file` (a logo refuses today) | Done for the 2.4 GHz TX target |
| 5 | The script did not check the board | `ver` names `gx12` before the pulses stop | Done |
| 6 | Sent `serialpassthrough rfmod 0 460800` | The module path at 460800 | Done |
| 7 | esptool with `--before no-reset` wrote four segments (bootloader, partitions, boot_app0, firmware) to the ESP32-PICO-D4, each hash-verified. The old flash was not read | Read the chip's 4 MB first and keep it. Every segment verified | Done for the 2.4 GHz TX target |
| 8 | The owner unplugged and restarted the radio. The factory ELRS version was not read | Read the version first (CRSF device info) so the report names before and after | Open (gear-design open question 9) |
| 9 | The Lua (`elrs.lua`) went on in 3.1 step 11 | Order: EdgeTX, Lua, then ELRS | Missing |

### 3.3 Where the GX12 differs from the Pocket

- DFU entry needs SG held. Everything after the DFU device appears is the same.
- The image is the same layout (F407, 1 MB, bootloader at 0, firmware at `0x8000`).
- The storage is a chip, so the backup before the flash matters more, and every write is slow.
- The ELRS module is an LR1121 dual-band target with no UART route.
- The YAML schema differs only in the custom switches (2.4): model, logical switch, special
  function, timer and sensor structures are the same as the `128x64` family.

## 4. In-app radio setup

### 4.1 What the card patcher does, against QuadCam

The card patcher (revision 10) sets up two models from tables: a whoop with an analog DVR, and
a quad that records on board. The
table maps each thing it writes to what QuadCam has.

| Patcher | What it writes | QuadCam today | Gap |
|---|---|---|---|
| Model name | `header.name` | `ModelOp::Rename` | none |
| Second model | Clones model00 to model01, timer values 0 | `Edit::ModelCopy` | Cross-board copy: stage 2 |
| ELRS model ID | `header.modelId` 0 and 1 | `ModelOp::SetModelId` | none |
| Mixes | CH3 throttle with the turtle cut; CH5 arm with the turtle pulses; CH6, CH7 at weight -100; CH10 blackbox erase on a trim; CH11 the wheel; CH12-15 trims as sim buttons; CH16 +100 | `ModelOp::SetMixes` | Generated from the assignment (5.3), not typed by hand |
| Logical switches | L1-L23: the flying latch, battery alarms, the finder edges, the link test, the turtle gate, the DVR reminder | `ModelOp::SetLogicalSwitch` | Generated from the assignment |
| Special functions | Callouts per position (`!1x`), finder screen on edges, turtle sound and haptic, logging every 0.1 s, battery haptics, DVR reminder, timer reset, trims as read-outs | `SpecialFunctions`, `MoveSpecialFunction`, callouts, logging | Generated from the assignment |
| Switch warnings | SA up only, as the 2.12 list | `SetSwitchWarnings` | The editor picks which controls are checked |
| Timers | TOT (lifetime), FLT (this flight), DVR (reminder), swap | `SetTimer`, `RemoveTimer`, `SwapTimers` | none |
| Telemetry screens | Screen 1 values; screen 2 `volt`, screen 3 `finder` | `SetScreenValues`, `SetScreen` | none for the YAML |
| Lua screen files | `SCRIPTS/TELEMETRY/volt.lua`, `finder.lua` | `Edit::CardFiles` exists for voice | QuadCam must ship its own scripts (MIT, its own code) and install them. `finder.lua` reads `sd` by name: it must take the finder control from the assignment |
| Custom sounds | Nine clips in `SOUNDS/en/`, copied only when missing | Voice packs and `CardFiles` | The setup names the clips it needs; a missing clip is a check |
| Checklist | Text files, flags off | `SetChecklist`, `Edit::Checklist` | B&W line length is 20 on 128x64: take it from the profile |
| `radio.yml` scalars | `hapticMode: mode_nokeys`, volumes, `currModel` | `RadioOp::SetScalar`, `SelectModel` | none |
| `radio.yml` switch types | Fixed SA, SD `2POS` and SE `TOGGLE` on the Pocket | Missing | A radio op to set `switchConfig` and `potsConfig` types (5.4) |
| Old models | Deletes other `model*.yml` | `Edit::ModelDelete` | none |
| Retired items | Removes callouts and functions older revisions made | Ownership by track name and by change | Ownership per setup (5.3) |
| Sensor slots | `{RxBt}` and friends become `tele(N)`; refuses with no sensor | The same rule | A new GX12 model has no sensors. A copy from the Pocket brings them; a fresh model needs "discover sensors" on the radio first |
| Custom switches | not done | Missing | SW1-SW6 type, group, start state and LED colours per model (5.4) |
| Line endings | Keeps CRLF | Done | none |
| `checksum: 0` | On every edited file | Done | none |

### 4.2 The setup as data

QuadCam stores a radio setup as data, not as a script revision. A setup is:

- **Jobs**: what the pilot wants (arm, OSD layout, rates, finder, turtle, VTX power,
  blackbox erase, read-outs, sim buttons, spare channels).
- **Assignments**: per radio, which control does each job, and in which direction.
- **Model template**: the parts that do not depend on a control (timers, battery alarms per
  aircraft, telemetry screens, checklist, logging).
- **Aircraft overrides**: the per-quad parts (a quad that records on board needs no DVR
  reminder; one model warns on mAh drawn, another on voltage).

QuadCam generates the model edits from that data and stages them as one change, with the usual
diff, plan, digest and apply. A later edit to a job regenerates only what that job owns.

## 5. The switch-assignment editor

### 5.1 Jobs, channels and values

The channels and values are what Betaflight reads today on both quads (`aux`, `adjrange` and
`vtx` lines). AUXn is CHn+4. With the mix at ±100 %, EdgeTX sends 988 µs and 2012 µs; QuadCam
shows them as 1000 and 2000, and checks ranges with the real values.

| Job | Channel | Positions and values | Betaflight side | Control kinds that fit |
|---|---|---|---|---|
| Arm | CH5 (AUX1) | off 1000; on 2000 | `aux` ARM 1700-2100 | 2-pos switch or latching button. Never a momentary or a customizable switch that can start on |
| OSD layout | CH6 (AUX2) | profile 1 at 1000, 2 at 1500, 3 at 2000 | `adjrange` OSD profile on AUX2 | 3-pos switch |
| Rates | CH7 (AUX3) | profile 1 at 1000, 2 at 1500, 3 at 2000 | `adjrange` rate profile on AUX3 | 3-pos switch |
| Finder | CH8 (AUX4) | off 1000; on 2000 | `aux` BEEPER 1700-2100 | 2-pos switch, latching button, or a customizable switch in toggle mode |
| Turtle | CH9 (AUX5) | off 1000; held 2000, gated by throttle low for 0.2 s | `aux` FLIP OVER AFTER CRASH 1700-2100 | Momentary only |
| Blackbox erase | CH10 (AUX6) | off 1000; pressed 2000 | `aux` BLACKBOX ERASE 1700-2100 | A trim or a momentary |
| VTX power | CH11 (AUX7) | low 25 mW below 1300; 200 mW 1300-1700; 400 mW above 1700 | `vtx` table on AUX7 | A pot, slider or wheel; a 3-pos switch also fits |
| Sim buttons | CH12-CH15 | 2000 while pressed | none (AUX8 on CH12 unused) | Trims or buttons |
| Spare | CH16 | +100 always | none | none |

In ELRS Wide mode (the owner's setting) the link sends CH5 and CH6-CH12. A job on CH13-CH16
never reaches the quad.

The radio-side effects of each job (callouts per position, the finder screen on SD edges, the
turtle gate's logical switches, the link and battery gates) follow the job, not the control.
Moving a job to another control moves all of them.

### 5.2 Control inventory per board

Each profile lists its controls with kind and positions. The editor offers a control for a job
only if its kind fits (5.1).

| Board | 2-pos | 3-pos | Momentary | Latching button | Customizable | Pots and sliders | Trims |
|---|---|---|---|---|---|---|---|
| `pocket` | none | SB, SC (top edge) | SE | SA, SD (LED) | none | S1 wheel (`P1`) | 4 |
| `gx12` | SA, SD | SB, SC, SE, SF | SG, SH | none | SW1-SW6: per model, 2-pos or toggle, grouped, start off, on or as before, LED colours | P1, P2 (detent), SL1, SL2 | 4 |

Default GX12 assignment, the same jobs as the Pocket: arm SA, OSD layout SB, rates SC, finder
SD, turtle SH, VTX power P1. SE, SF, SG, SW1-SW6, P2 and the sliders stay free. The owner sets
the final map in the editor.

**Direction.** On the Pocket the top-edge toggles read +100 in the owner's "up", so CH6 and CH7
use weight -100. The GX12's front toggles probably read -100 in "up" (position 0), so the same
jobs would use weight +100. The source does not prove the physical direction. The editor
therefore stores the direction per assignment, and stage 5 checks it on the radio with the
live joystick view: move the switch to the owner's "up", read the channel.

### 5.3 What the editor writes

For each assignment the generator emits model ops:

- the job's mix line on its channel (`SetMixes`), weight by direction;
- the job's logical switches with the control's position names (`SA2`, `SH2`, `SC0`), using the
  board's naming (index × 3 + position);
- the job's special functions (callouts per position, screens on edge pulses, haptics);
- the switch warning for the jobs that need one (arm released);
- for a customizable switch, its `customSwitches:` entry (5.4);
- the Lua screen settings that name a control (the finder screen's control).

The change records which logical switches and special functions each job owns, as changes do
today. Reassigning a job replaces only its items. Two jobs on one control refuse, unless both
allow sharing (sim buttons on trims do not share with read-outs on the same trim).

### 5.4 Valid YAML per board

- Field set from the profile's schema family. The `gx12` family equals `128x64` except custom
  switches.
- `customSwitches:` entries carry `sw` (SW1-SW6, the radio's default switch names), `name`,
  `type`, `group`, `start` (`START_OFF`, `START_ON`, `START_PREVIOUS`), `state`, the two Lua
  override flags, and `onColor`/`offColor` as `r`, `g`, `b`. Groups that need one switch on use
  `cfsGroupOn:`. QuadCam never writes the legacy `functionSwitch*` keys; when it edits a file
  that still holds them, it reads them, writes the converted `customSwitches:` and drops the
  legacy lines, the way EdgeTX would on its next save. Stage 2 must prove this against a file the
  radio saved on 2.12.5.
- `radio.yml` `switchConfig` and `potsConfig` types through a new radio op, case as the file has
  it, ranges from the profile.
- Switch warnings as the 2.12 `switchWarning:` list. Never `switchWarningState:`.
- `checksum: 0` on each edited file that has a checksum line.

### 5.5 Copy a setup from the Pocket to the GX12

The owner's models live on the Pocket. Moving to the GX12 is one flow:

1. Pick the source radio (Pocket card or its latest backup) and the target (GX12).
2. QuadCam reads the source's setup back into jobs: each job's control on the Pocket.
3. It proposes the target's controls from the default GX12 assignment, and lists anything it
   cannot map.
4. The owner adjusts in the editor.
5. Apply: model copy (stage 2's cross-board copy), then the generated ops for the GX12's
   controls, then sounds and Lua screens. One plan, one confirm, a backup first. The plan's
   estimate uses the GX12's USB speed.

### 5.6 The UI

Native macOS, as the rest of Gear (`docs/development.md`):

- Radio page, a **Setup** segment beside Switches: a table, one row per job, with columns Job,
  Control (a pop-up of the controls that fit), Direction, Channel, and what each position does in
  plain words.
- **Learn**: with the radio in USB Joystick mode, select a row and move a control; the control
  that moved is picked. Uses the existing `radio-input` stream. No synthetic input.
- A board picture is not needed; the control names match the radio's labels.
- **Review…** opens the apply sheet with the diff per file.
- **Copy from…** runs 5.5.

Every UI change goes to the owner in a dev build first. The owner approves it before it ships.

## 6. Stages

Each stage ships on its own, with the backend, CLI and MCP reaching it (house rule) and its
docs. A stage on real gear ends with a trial in [Device trials](trials.md).

| # | Stage | Ships | Accepted when |
|---|---|---|---|
| 1 | Board profiles and the GX12 flash (`feat/gx12-flash`) | The profile table; `gx12` proven for the EdgeTX flash, read and restore at 2.12.5; DFU steps from the profile; the ELRS TX target from the board, `etx` only for the GX12, the LR1121 firmware | Unit tests on the table; the flash plan for `gx12` on fakes names the SG steps and the board binary; the ELRS plan for a GX12 picks `radiomaster.tx_dual.gx12` and refuses `uart`. On glass: Read firmware on the GX12 saves 1 MB and names `edgetx-gx12-2.12.5` |
| 2 | GX12 card and cross-board copy (`feat/gx12-card`) | GX12 card read and write; switch types read case-insensitively; `customSwitches:` read; `gx12` proven for card writes; model copy Pocket to GX12 with a control map | Round-trip byte-equal on GX12 fixtures (factory 2.11 and a file the radio saved on 2.12.5); each edit op on `gx12`; the cross-board copy lists what it cannot map |
| 3 | Setup as data | Jobs, assignments, template and overrides (4.2); the generator; radio ops for switch and pot types; `customSwitches:` write; QuadCam's own telemetry Lua screens with the finder control as a setting; post-flash steps (storage pack, `elrs.lua`, volume label) | On a copy of the Pocket card at revision 10, the generated change for the Pocket assignment plans **no files** (QuadCam reproduces the patcher). The GX12 default assignment on a GX12 fixture gives valid YAML by the schema |
| 4 | The editor | The Setup segment, Learn, Review, Copy from; rows `gear_radio_setup` and `gear_radio_setup_edit`; CLI `gear setup`; MCP `quadcam_gear` `radio_setup` and `quadcam_gear_edit` `radio_setup_edit` | e2e on the mock core; the owner approves the dev build |
| 5 | GX12 on glass | Both models on the GX12 through QuadCam | Each switch's direction matches the assignment on the Controls page; the switch map matches position by position; with a quad linked, Betaflight's Receiver tab shows the expected values and the Modes tab lights ARM, BEEPER and turtle; callouts play; the finder screen opens on the finder control |
| 6 | Retire the scripts | Nothing in QuadCam. The fpv notes switch the route to QuadCam once the owner passes stages 1-5 | The owner runs a bench session with no script |
| 7 | Other boards | Profiles for the lines in 2.5 as unproven data; the UF2 refusal; the `v12` floor; the F2 cap | Unit tests for each guard; a profile shows on an unknown radio of a listed board |

Stage 3 shipped as: `gear/edgetx/setup/` (the types, the defaults, the generator, QuadCam's
Lua screens, the post-flash planners), which takes each board's controls from the board table
(`gear/edgetx/board.rs`) and writes switch and pot types with stage 2's `set_control_type`;
the radio op `remove_functions`, the edit `card_text` (a Lua script in the same change as the
models), and `core/radio_setup.rs` with the rows `gear_radio_setup`, `gear_radio_setup_edit`,
`gear_radio_post_flash` and `gear_volume_label` (CLI `gear setup`, `setup-edit`, `post-flash`,
`volume-label`; MCP `radio_setup`, `post_flash_plan`, `radio_setup_edit`, `post_flash_stage`,
`volume_label`). Stage 4 adds the editor on these rows. Two choices differ from the plan: a
Lua screen of the same name that QuadCam did not write stays (with a note), so the
patcher's own scripts plan no file; and sounds are only checked, never written by the setup.
A customizable switch (SW1-SW6) refuses as a control until `customSwitches:` writes land.

Stage 7 shipped as: `gear/radios/` (`mod.rs` the types, guards and warnings; `boards.rs` the
50 rows; `tests.rs` a test per guard), `Connected.radio` filled by `gear/detect.rs`, and the row
`gear_radio_boards` (section 2.5). The guards: a UF2 board refuses with the UF2 route named, an
F2 board refuses a 2.12 release, `v12` refuses below 2.12.3, look-alikes warn on both boards.
One choice differs from the plan: the slots use the card engine's `BoardControlKind`
(`gear/edgetx/board.rs`), and a test checks that every control `board.rs` lists sits in its
board's slots. `board.rs` still holds its own USB write speed per board, beside the profile's
`usb_speed`.

Stage 1 is the existing task "QuadCam Gear: per-radio profile table and prove board gx12".
Stages 1 and 2 run now. Stage 3 needs both. Stage 4 needs 3. Stage 5 needs 4 and the radio.

## 7. Open questions for the owner

Each has a default the build uses until the owner decides.

| # | Question | Default |
|---|---|---|
| 1 | The GX12 control map: arm SA, OSD SB, rates SC, finder SD, turtle SH, VTX power P1? Anything for SE, SF, SG, SW1-SW6, P2 and the sliders? | That map; the rest free |
| 2 | Is "up" on the GX12's front toggles the owner's "up" in the same sense as the Pocket's top-edge toggles (rate and OSD profile 1)? | Yes; checked on glass in stage 5 |
| 3 | Should the Pocket keep both models and stay in sync with the GX12, or become a backup that QuadCam updates only on request? | Backup, updated on request |
| 4 | Install a fallback firmware bin in `FIRMWARE/` on each flash (bootloader recovery without the Mac)? | Yes on chip radios, where it costs no card swap |
| 5 | Rename the radio's volume to a label from its saved name? | Yes, offered in the post-flash steps |
