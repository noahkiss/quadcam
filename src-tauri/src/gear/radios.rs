//! What QuadCam knows of each EdgeTX radio board, in one table: the chip and its firmware
//! image, how the radio reaches ROM DFU and the EdgeTX bootloader, where its storage lives
//! and how big writes reach it, how its splash is set, its internal ExpressLRS module, the
//! USB product name it reports and the last EdgeTX series it gets.
//!
//! The flash (`core/firmware.rs`), the firmware read and restore, the DFU link and the ELRS
//! flash plan read this table instead of assuming one radio. A new board is a new row:
//! its facts come from the makers and the EdgeTX hardware definitions, checked on a real
//! radio where a row says so. A row alone writes nothing: `compat::PROVEN` still names the
//! releases QuadCam flashes on each board.

use super::firmware::edgetx::BoardSpec;
use serde::Serialize;
use specta::Type;

/// The file the EdgeTX release gives this board.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareFormat {
    /// A raw image for the STM32's internal flash, written over ROM DFU.
    Bin,
    /// A UF2 file for the EdgeTX bootloader's drive (H7 radios). QuadCam does not write it.
    Uf2,
}

/// How the radio starts the STM32's ROM DFU bootloader.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum DfuStart {
    /// Plugging USB into the radio while it is off.
    PlugIn,
    /// A button held while the radio powers on with USB plugged in.
    ButtonCombo,
}

/// How a person puts the radio in ROM DFU mode.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct DfuEntry {
    pub start: DfuStart,
    /// The steps the UI lists, one sentence each.
    pub steps: &'static [&'static str],
    /// The same as one instruction, for a refusal or a check (no "No radio is in DFU mode").
    pub hint: &'static str,
}

/// Where the radio keeps its card files.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    /// A microSD slot: the card can go into the Mac's reader.
    CardSlot,
    /// A chip soldered to the board: USB Storage mode is the only way in.
    Chip,
}

/// How a big write (a voice pack) reaches the radio's storage.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WriteRoute {
    /// The card in the Mac's reader, or the radio in USB Storage mode.
    CardReaderOrUsb,
    /// The radio in USB Storage mode only (full speed USB, about 0.3 MB/s).
    UsbStorage,
}

/// How the radio's boot picture is set.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SplashKind {
    /// A 128 x 64, 1-bit picture inside the firmware, between the `SPS`/`SPE` markers
    /// (`gear::splash`).
    FirmwareMono128x64,
    /// A picture on the storage (`/IMAGES/splash.png`), on colour radios.
    StorageImage,
}

/// The radio's internal ExpressLRS module.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct ElrsInternal {
    /// The target's path in the release's `targets.json` (`vendor.group.model`).
    pub target: &'static str,
    /// The name the module's Lua script shows.
    pub lua_name: &'static str,
    /// The routes `targets.json` allows (`uart`, `etx`, `wifi`).
    pub upload_methods: &'static [&'static str],
    /// The oldest ExpressLRS release with this target.
    pub min_version: &'static str,
}

/// One radio board.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct RadioProfile {
    /// The `board:` value of `radio.yml` and the EdgeTX target.
    pub board: &'static str,
    pub maker: &'static str,
    pub model: &'static str,
    pub mcu: &'static str,
    pub firmware_format: FirmwareFormat,
    /// The image checks of an EdgeTX flash. None: QuadCam does not flash this board.
    #[serde(skip)]
    pub flash: Option<BoardSpec>,
    pub dfu: DfuEntry,
    /// How the EdgeTX bootloader starts.
    pub bootloader_entry: &'static str,
    pub storage: StorageKind,
    pub write_route: WriteRoute,
    pub splash: SplashKind,
    pub elrs: Option<ElrsInternal>,
    /// The start of every USB product name the radio reports (`Radiomaster GX12 Mass
    /// Storage`), matched ignoring case: the GX12 says "Radiomaster", EdgeTX "RadioMaster".
    pub usb_product: &'static str,
    /// The last EdgeTX series the board gets (`2.12`).
    pub edgetx_series: &'static str,
}

/// Every radio QuadCam knows. A row is data; `compat::PROVEN` decides what is written.
pub const PROFILES: &[RadioProfile] = &[
    RadioProfile {
        board: "pocket",
        maker: "RadioMaster",
        model: "Pocket",
        mcu: "STM32F407xE",
        firmware_format: FirmwareFormat::Bin,
        flash: Some(BoardSpec {
            id: "pocket",
            names: &["pocket"],
            min_kb: 400,
            max_kb: 1000,
            app_offset: 0x8000,
            flash_bytes: 0x10_0000,
        }),
        dfu: DfuEntry {
            start: DfuStart::PlugIn,
            steps: &[
                "Turn the radio off.",
                "Plug the radio into this Mac with the USB cable. Do not hold any buttons.",
                "Wait a few seconds.",
            ],
            hint: "Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds.",
        },
        bootloader_entry: "Hold both horizontal trims inward, then power on.",
        storage: StorageKind::CardSlot,
        write_route: WriteRoute::CardReaderOrUsb,
        splash: SplashKind::FirmwareMono128x64,
        elrs: Some(ElrsInternal {
            target: "radiomaster.tx_2400.pocket",
            lua_name: "RM Pocket",
            upload_methods: &["uart", "wifi", "etx"],
            min_version: "3.0.0",
        }),
        usb_product: "radiomaster pocket",
        edgetx_series: "2.12",
    },
    RadioProfile {
        board: "gx12",
        maker: "RadioMaster",
        model: "GX12",
        mcu: "STM32F407xG",
        firmware_format: FirmwareFormat::Bin,
        // The 2.12.5 release image: 540 KB, the bootloader's vector table at 0 and the
        // firmware's at 0x8000, as on the Pocket; DFU reports 1 MB (04*016Kg,01*064Kg,
        // 07*128Kg). Checked on a real GX12 on 2026-10-22, outside the test suite.
        flash: Some(BoardSpec {
            id: "gx12",
            names: &["gx12"],
            min_kb: 400,
            max_kb: 1000,
            app_offset: 0x8000,
            flash_bytes: 0x10_0000,
        }),
        dfu: DfuEntry {
            start: DfuStart::ButtonCombo,
            steps: &[
                "Turn the radio off.",
                "Plug the USB cable into the radio's top port and into this Mac.",
                "Hold SG, tap the power button, then let go of SG. The screen stays dark.",
                "Wait a few seconds.",
            ],
            hint: "Turn the radio off and plug the USB cable into its top port. Hold SG, tap the power button, then let go of SG; the screen stays dark. Wait a few seconds.",
        },
        bootloader_entry: "Hold both horizontal trims inward, then power on.",
        storage: StorageKind::Chip,
        write_route: WriteRoute::UsbStorage,
        splash: SplashKind::FirmwareMono128x64,
        elrs: Some(ElrsInternal {
            target: "radiomaster.tx_dual.gx12",
            lua_name: "RM GX12 X-Band",
            upload_methods: &["etx", "wifi"],
            min_version: "3.6.0",
        }),
        usb_product: "radiomaster gx12",
        edgetx_series: "2.12",
    },
];

/// The profile of `board` (`radio.yml`'s `board:`), ignoring case and spaces around it.
pub fn profile(board: &str) -> Option<&'static RadioProfile> {
    let b = board.trim().to_ascii_lowercase();
    PROFILES.iter().find(|p| p.board == b)
}

/// The profile whose USB product name starts `product` (`Radiomaster GX12 Serial Port`),
/// ignoring case.
pub fn by_usb_product(product: &str) -> Option<&'static RadioProfile> {
    let p = product.trim().to_ascii_lowercase();
    PROFILES.iter().find(|r| {
        p.strip_prefix(r.usb_product)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// The DFU instruction for a radio on `board`. A board QuadCam has no row for gets every
/// known way in, so the person can pick theirs.
pub fn dfu_hint(board: Option<&str>) -> String {
    if let Some(p) = board.and_then(profile) {
        return p.dfu.hint.to_string();
    }
    let combos: Vec<String> = PROFILES
        .iter()
        .filter(|p| p.dfu.start == DfuStart::ButtonCombo)
        .map(|p| format!("on a {} {}: {}", p.maker, p.model, p.dfu.hint))
        .collect();
    let plain = "Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds.";
    if combos.is_empty() {
        plain.to_string()
    } else {
        format!(
            "{plain} Some radios need a button instead, {}",
            combos.join(" ")
        )
    }
}

/// The DFU steps the UI lists for a radio on `board`; empty for a board QuadCam has no row
/// for.
pub fn dfu_steps(board: Option<&str>) -> Vec<String> {
    board
        .and_then(profile)
        .map(|p| p.dfu.steps.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boards_are_unique_lowercase_and_found_ignoring_case() {
        for (i, p) in PROFILES.iter().enumerate() {
            assert_eq!(p.board, p.board.to_ascii_lowercase());
            assert!(PROFILES[i + 1..].iter().all(|q| q.board != p.board));
            assert_eq!(profile(&format!(" {} ", p.board.to_uppercase())), Some(p));
            if let Some(f) = &p.flash {
                assert_eq!(f.id, p.board, "the flash spec is the board's own");
                assert!(f.names.contains(&p.board));
                assert!(f.min_kb < f.max_kb && f.max_kb * 1024 <= f.flash_bytes);
            }
            assert_eq!(p.usb_product, p.usb_product.to_ascii_lowercase());
            assert!(!p.dfu.steps.is_empty() && !p.dfu.hint.is_empty());
        }
        assert!(profile("tx16s").is_none());
    }

    #[test]
    fn the_gx12_row() {
        let g = profile("gx12").unwrap();
        assert_eq!(g.mcu, "STM32F407xG");
        assert_eq!(g.firmware_format, FirmwareFormat::Bin);
        assert_eq!(g.dfu.start, DfuStart::ButtonCombo);
        assert!(g.dfu.hint.contains("Hold SG"));
        assert_eq!(g.storage, StorageKind::Chip);
        assert_eq!(g.write_route, WriteRoute::UsbStorage);
        let e = g.elrs.unwrap();
        assert_eq!(e.target, "radiomaster.tx_dual.gx12");
        assert!(!e.upload_methods.contains(&"uart"));
        assert!(e.upload_methods.contains(&"etx"));
        let f = g.flash.unwrap();
        assert_eq!(f.flash_bytes, 0x10_0000);
        let p = profile("pocket").unwrap();
        assert_eq!(p.dfu.start, DfuStart::PlugIn);
        assert_eq!(p.storage, StorageKind::CardSlot);
        assert_eq!(p.elrs.unwrap().target, "radiomaster.tx_2400.pocket");
    }

    #[test]
    fn usb_products_match_ignoring_case_and_only_whole_models() {
        assert_eq!(
            by_usb_product("Radiomaster GX12 Mass Storage").map(|p| p.board),
            Some("gx12")
        );
        assert_eq!(
            by_usb_product("RadioMaster GX12 Serial Port").map(|p| p.board),
            Some("gx12")
        );
        assert_eq!(
            by_usb_product("RadioMaster Pocket Joystick").map(|p| p.board),
            Some("pocket")
        );
        assert_eq!(by_usb_product("RadioMaster GX120 DFU"), None);
        assert_eq!(by_usb_product("RadioMaster TX16S Mass Storage"), None);
    }

    #[test]
    fn the_dfu_hint_follows_the_board() {
        assert!(dfu_hint(Some("pocket")).contains("do not hold the trim buttons"));
        assert!(!dfu_hint(Some("pocket")).contains("SG"));
        assert!(dfu_hint(Some("GX12")).contains("Hold SG"));
        // An unknown board, or none, hears both ways.
        for b in [None, Some("tx16s")] {
            let h = dfu_hint(b);
            assert!(h.contains("do not hold the trim buttons"), "{h}");
            assert!(
                h.contains("RadioMaster GX12") && h.contains("Hold SG"),
                "{h}"
            );
        }
        assert_eq!(dfu_steps(Some("gx12")).len(), 4);
        assert!(dfu_steps(Some("tx16s")).is_empty());
    }

    #[test]
    fn a_firmware_splash_board_takes_the_128x64_patch() {
        for p in PROFILES {
            if p.splash == SplashKind::FirmwareMono128x64 {
                assert!(
                    super::super::splash::board_supported(p.board),
                    "{}",
                    p.board
                );
            }
        }
    }
}
