//! What QuadCam knows of each EdgeTX radio board, in one table: the chip and its firmware
//! image, how the radio reaches ROM DFU and the EdgeTX bootloader, where its storage lives
//! and how big writes reach it, how its splash is set, its internal ExpressLRS module, the
//! USB product name it reports, the last EdgeTX series it gets, and the traps around it:
//! look-alike boards, a reused target name, the maker's own firmware, known quirks.
//!
//! The flash (`core/firmware.rs`), the firmware read and restore, the DFU link, detection and
//! the ELRS flash plan read this table instead of assuming one radio. A new board is a new
//! row in `boards.rs`: its facts come from the makers and the EdgeTX hardware definitions,
//! checked on a real radio where a row says so. A row alone writes nothing: `compat::PROVEN`
//! still names the releases QuadCam flashes and the cards it writes on each board. Every
//! other row is read-only: QuadCam shows its profile, DFU steps and warnings.

mod boards;

use super::edgetx::board::BoardControlKind;
use super::model::{Refusal, RefusalCode};
pub use boards::PROFILES;
use serde::{Deserialize, Serialize};
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
    /// A button held while the radio powers on or is plugged in.
    ButtonCombo,
    /// No source says how.
    Unknown,
}

/// How a person puts the radio in ROM DFU mode.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct DfuEntry {
    pub start: DfuStart,
    /// The steps the UI lists, one sentence each. Empty when the way in is unknown.
    pub steps: &'static [&'static str],
    /// The same as one instruction, for a refusal or a check (no "No radio is in DFU mode").
    pub hint: &'static str,
}

/// The DFU entry of a board no source describes.
pub const DFU_UNKNOWN: DfuEntry = DfuEntry {
    start: DfuStart::Unknown,
    steps: &[],
    hint: "",
};

/// Where the radio keeps its card files.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    /// A microSD slot: the card can go into the Mac's reader.
    CardSlot,
    /// A chip soldered to the board: USB Storage mode is the only way in.
    Chip,
    /// A soldered chip and a microSD slot; a card present at boot hides the chip (H7 radios).
    ChipCardOverrides,
}

/// How a big write (a voice pack) reaches the radio's storage.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WriteRoute {
    /// The card in the Mac's reader, or the radio in USB Storage mode.
    CardReaderOrUsb,
    /// The radio in USB Storage mode only (full speed USB, about 0.3 MB/s).
    UsbStorage,
    /// A chip a card overrides: no route until a radio of this kind is tested.
    Untested,
}

/// How the radio's boot picture is set.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SplashKind {
    /// A 128 x 64, 1-bit picture inside the firmware, between the `SPS`/`SPE` markers
    /// (`gear::splash`).
    FirmwareMono128x64,
    /// A 212 x 64 greyscale picture inside the firmware. QuadCam does not patch it.
    FirmwareMono212x64,
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

/// USB Storage speed measured on a real radio, in KB/s.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct UsbSpeed {
    pub write_kb_s: u32,
    pub read_kb_s: u32,
}

/// The largest boot sound a board takes: a bigger or faster `hello.wav` can freeze its
/// switches at boot (the GX12 on 2.12.4).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct HelloWavCap {
    pub max_bytes: u32,
    pub max_rate_hz: u32,
}

/// EdgeTX's own voice packs (`EdgeTX/edgetx-sdcard-sounds`, `SOUNDS/*/SYSTEM/hello.wav` on
/// main, 2026-10-30): 26 packs, 57,646 to 158,764 bytes (`ko` the largest), mono 16-bit PCM
/// at 16 kHz or 32 kHz (`voice-gen-elevenlabs.py` resamples to 32,000 Hz). The cap sits just
/// above the largest: 160 KiB, about 2.5 s at 32 kHz.
pub const EDGETX_HELLO_WAV: HelloWavCap = HelloWavCap {
    max_bytes: 160 * 1024,
    max_rate_hz: 32_000,
};

/// True for a voice pack's boot sound, `SOUNDS/<lang>/SYSTEM/hello.wav`, in any case.
pub fn is_hello_wav(path: &str) -> bool {
    let parts: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(|s| s.to_ascii_lowercase())
        .collect();
    matches!(parts.as_slice(), [s, _, sys, f] if s == "sounds" && sys == "system" && f == "hello.wav")
}

impl HelloWavCap {
    /// Refuses a boot sound over the cap. The refusal names the file, its size or rate,
    /// and the limit. A file whose rate cannot be read is held to the size cap only.
    pub fn check(&self, board: &str, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        let size = bytes.len();
        if size > self.max_bytes as usize {
            return Err(Refusal::new(
                RefusalCode::Incompatible,
                format!(
                    "{path} is {size} bytes; a {board} takes a hello.wav of at most {} bytes (a large one can freeze its switches at boot). Shorten it.",
                    self.max_bytes
                ),
            ));
        }
        if let Some(rate) = super::voice::wav::rate(bytes) {
            if rate > self.max_rate_hz {
                return Err(Refusal::new(
                    RefusalCode::Incompatible,
                    format!(
                        "{path} is {rate} Hz; a {board} takes a hello.wav of at most {} Hz (a high-rate one can freeze its switches at boot). Resample it.",
                        self.max_rate_hz
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// A radio that looks like this one but takes other firmware.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct LookAlike {
    /// The look-alike's name (`T20 V2`).
    pub name: &'static str,
    /// Its board, when EdgeTX has one (`t20v2`); None for a name with no firmware.
    pub board: Option<&'static str>,
    /// The warning the flash plan shows.
    pub note: &'static str,
}

/// A target name EdgeTX once used for another radio.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct TargetHistory {
    /// The first EdgeTX release whose target is this radio (`2.12.3`): older ones refuse.
    pub until: &'static str,
    pub note: &'static str,
}

/// The maker's own EdgeTX builds.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct MakerFork {
    /// Where the fork's source lives.
    pub source: &'static str,
    /// What an upstream flash loses, as the warning before the first flash.
    pub note: &'static str,
}

/// One control slot of a board, from its EdgeTX hardware definition. The slot is fixed; the
/// control fitted in it is not (GX12 switch modules swap, pot positions take blanking plugs),
/// so the fitted type always comes from the radio's `radio.yml` (`edgetx::board::inventory`
/// joins them for the boards QuadCam writes).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct ControlSlot {
    /// `SA`, `SW1`, `P1`, `SL1`.
    pub name: &'static str,
    /// A switch, a custom switch, or an analog input as EdgeTX sets it by default (a pot, or
    /// a slider when the default is `SLIDER`).
    pub kind: BoardControlKind,
    /// What the hardware reads: `2POS` or `3POS` (the most positions the slot gives), `ADC`
    /// (a switch read as an analog input), `FLEX` (an analog input).
    pub hardware: &'static str,
    /// EdgeTX's default type for the slot (`3POS`, `TOGGLE`, `POT_CENTER`, `SLIDER`, `NONE`);
    /// None when the definition gives none.
    pub default: Option<&'static str>,
}

const fn slot(
    name: &'static str,
    kind: BoardControlKind,
    hardware: &'static str,
    default: Option<&'static str>,
) -> ControlSlot {
    ControlSlot {
        name,
        kind,
        hardware,
        default,
    }
}

/// One radio board.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
pub struct RadioProfile {
    /// The `board:` value of `radio.yml` and the EdgeTX target.
    pub board: &'static str,
    pub maker: &'static str,
    pub model: &'static str,
    /// Other names sold with the same firmware (`GX12 Crush`).
    pub variants: &'static [&'static str],
    pub mcu: &'static str,
    pub firmware_format: FirmwareFormat,
    /// The image checks of an EdgeTX flash. None: QuadCam does not flash this board.
    #[serde(skip)]
    pub flash: Option<BoardSpec>,
    pub dfu: DfuEntry,
    /// How the EdgeTX bootloader starts. None: no source says.
    pub bootloader_entry: Option<&'static str>,
    pub storage: StorageKind,
    pub write_route: WriteRoute,
    /// USB Storage speed measured on a real radio. None: never measured. Every EdgeTX radio
    /// runs full speed USB (12 Mb/s), so expect about 0.3 MB/s.
    pub usb_speed: Option<UsbSpeed>,
    /// The EdgeTX SD-card pack for the board (`bw128x64`, `c480x272`).
    pub sd_pack: &'static str,
    /// The EdgeTX YAML schema family the board's files use (`128x64`, `gx12`, `x10`).
    pub yaml_schema: &'static str,
    pub splash: SplashKind,
    /// The internal RF module as sold. None: no source says.
    pub internal_rf: Option<&'static str>,
    /// The ExpressLRS targets of the internal module, one per RF variant the board ships
    /// with (2.4 GHz, 900 MHz). Empty: no ExpressLRS internal module, or none named.
    pub elrs: &'static [ElrsInternal],
    /// The start of every USB product name the radio reports (`Radiomaster GX12 Mass
    /// Storage`), matched ignoring case: EdgeTX names it `<maker> <model>` from its hardware
    /// definition, and the GX12 says "Radiomaster" where EdgeTX says "RadioMaster".
    pub usb_product: &'static str,
    /// The USB ports and which one carries data. None: no source says.
    pub usb_ports: Option<&'static str>,
    /// The last EdgeTX series the board gets (`2.12`; `2.11` on STM32F2 boards).
    pub edgetx_series: &'static str,
    /// A target name EdgeTX once used for another radio.
    pub target_history: Option<TargetHistory>,
    /// The maker's own EdgeTX builds, which an upstream flash replaces.
    pub fork: Option<MakerFork>,
    /// Radios that look like this one but take other firmware.
    pub look_alikes: &'static [LookAlike],
    /// The switch and pot slots on the case.
    pub slots: &'static [ControlSlot],
    pub trims: u8,
    /// The limit on the boot sound (`SOUNDS/<lang>/SYSTEM/hello.wav`) a card write may put.
    /// None: no limit.
    pub hello_wav: Option<HelloWavCap>,
    /// Known bugs and oddities, one sentence each.
    pub quirks: &'static [&'static str],
}

use super::firmware::edgetx::BoardSpec;

impl RadioProfile {
    /// True when QuadCam has proven any write on this board (`compat::PROVEN`). Every other
    /// board is read-only.
    pub fn proven(&self) -> bool {
        super::compat::PROVEN
            .iter()
            .any(|p| p.board == Some(self.board))
    }

    /// The internal module's one ExpressLRS target; None when the board has none or ships
    /// with several (the module's own report must decide).
    pub fn elrs_target(&self) -> Option<&'static ElrsInternal> {
        match self.elrs {
            [one] => Some(one),
            _ => None,
        }
    }

    /// What a person must know before a flash: look-alikes, a reused target name, the
    /// maker's own firmware, quirks, and whether QuadCam writes this board at all.
    pub fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if !self.proven() {
            out.push(format!(
                "QuadCam has not proven the {} {}: it shows this radio but does not flash or write it.",
                self.maker, self.model
            ));
        }
        if self.firmware_format == FirmwareFormat::Uf2 {
            out.push(uf2_route(self));
        }
        out.extend(self.look_alikes.iter().map(|l| l.note.to_string()));
        if let Some(h) = &self.target_history {
            out.push(format!(
                "{} QuadCam refuses {} firmware older than {}.",
                h.note, self.board, h.until
            ));
        }
        if let Some(f) = &self.fork {
            out.push(format!("{} Source: {}.", f.note, f.source));
        }
        out.extend(self.quirks.iter().map(|q| q.to_string()));
        out
    }
}

/// Where a connected radio's board came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum BoardFrom {
    /// `RADIO/radio.yml` `board:`.
    RadioYml,
    /// The USB product name.
    UsbProduct,
}

/// A radio's profile as detection shows it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Type)]
pub struct RadioBoard {
    pub profile: RadioProfile,
    pub from: BoardFrom,
    /// `RadioProfile::proven`: false means read-only.
    pub proven: bool,
    /// `RadioProfile::dfu`'s steps, empty when no source says how.
    pub dfu_steps: Vec<String>,
    /// `RadioProfile::warnings`.
    pub warnings: Vec<String>,
}

/// A saved `RadioBoard` reads back from its board and source: the table, not the saved
/// copy, gives the profile.
impl<'de> Deserialize<'de> for RadioBoard {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Board {
            board: String,
        }
        #[derive(Deserialize)]
        struct Saved {
            profile: Board,
            from: BoardFrom,
        }
        let s = Saved::deserialize(d)?;
        let p = profile(&s.profile.board).ok_or_else(|| {
            serde::de::Error::custom(format!("no radio profile for {:?}", s.profile.board))
        })?;
        Ok(info(p, s.from))
    }
}

/// The profile of `p` as detection shows it.
pub fn info(p: &RadioProfile, from: BoardFrom) -> RadioBoard {
    RadioBoard {
        profile: *p,
        from,
        proven: p.proven(),
        dfu_steps: p.dfu.steps.iter().map(|s| s.to_string()).collect(),
        warnings: p.warnings(),
    }
}

/// The profile of `board` (`radio.yml`'s `board:`), ignoring case and spaces around it.
pub fn profile(board: &str) -> Option<&'static RadioProfile> {
    let b = board.trim().to_ascii_lowercase();
    PROFILES.iter().find(|p| p.board == b)
}

/// The profile whose USB product name starts `product` (`Radiomaster GX12 Serial Port`),
/// ignoring case. The longest name wins, so `Jumper T20 V2 Mass Storage` is the T20 V2,
/// not the T20.
pub fn by_usb_product(product: &str) -> Option<&'static RadioProfile> {
    let p = product.trim().to_ascii_lowercase();
    PROFILES
        .iter()
        .filter(|r| {
            p.strip_prefix(r.usb_product)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
        .max_by_key(|r| r.usb_product.len())
}

/// The plain DFU instruction for a board that enters DFU on plug-in.
const PLAIN_DFU: &str = "Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds.";

/// The DFU instruction for a radio on `board`. A board QuadCam has no row for gets every
/// way in that QuadCam has proven, so the person can pick theirs.
pub fn dfu_hint(board: Option<&str>) -> String {
    if let Some(p) = board.and_then(profile) {
        if p.dfu.start == DfuStart::Unknown {
            return format!(
                "QuadCam does not know how the {} {} enters DFU mode. Follow its maker's manual or the EdgeTX manual's DFU page.",
                p.maker, p.model
            );
        }
        return p.dfu.hint.to_string();
    }
    let combos: Vec<String> = PROFILES
        .iter()
        .filter(|p| p.dfu.start == DfuStart::ButtonCombo && p.proven())
        .map(|p| format!("on a {} {}: {}", p.maker, p.model, p.dfu.hint))
        .collect();
    if combos.is_empty() {
        PLAIN_DFU.to_string()
    } else {
        format!(
            "{PLAIN_DFU} Some radios need a button instead, {}",
            combos.join(" ")
        )
    }
}

/// The DFU steps the UI lists for a radio on `board`; empty for a board QuadCam has no row
/// for, or whose way in no source describes.
pub fn dfu_steps(board: Option<&str>) -> Vec<String> {
    board
        .and_then(profile)
        .map(|p| p.dfu.steps.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// How a UF2 board updates, as the person can do it without QuadCam.
fn uf2_route(p: &RadioProfile) -> String {
    format!(
        "The {} {} updates by a UF2 file, not over DFU: start its EdgeTX bootloader ({}), then copy the release's {} .uf2 file to the EDGETX_UF2 drive, or use EdgeTX Buddy or Companion 2.12 or later.",
        p.maker,
        p.model,
        p.bootloader_entry
            .map(|b| b.trim_end_matches('.'))
            .unwrap_or("the maker's manual says how"),
        p.board
    )
}

/// `major.minor.patch` of a version (`v2.12.3`, `2.12`); a missing part reads as 0.
fn semver(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches(['v', 'V']).split(['.', '-']);
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// The profile's own guards on an EdgeTX flash of `version`: a UF2 board is not written over
/// DFU, a release past the board's last series is not for it (STM32F2 boards stop at 2.11),
/// and a release older than a reused target's floor flashes another radio's firmware. A
/// board with no profile passes here; `compat` refuses it.
pub fn check_flash(board: Option<&str>, version: &str) -> Result<(), Refusal> {
    let Some(p) = board.and_then(profile) else {
        return Ok(());
    };
    let want = semver(version);
    if let (Some(want), Some(h)) = (want, &p.target_history) {
        if semver(h.until).is_some_and(|floor| want < floor) {
            return Err(Refusal::new(
                RefusalCode::Incompatible,
                format!(
                    "EdgeTX {} is not for the {} {}. {}",
                    version.trim(),
                    p.maker,
                    p.model,
                    h.note
                ),
            ));
        }
    }
    if let (Some(want), Some(last)) = (want, semver(p.edgetx_series)) {
        if (want.0, want.1) > (last.0, last.1) {
            return Err(Refusal::new(
                RefusalCode::Incompatible,
                format!(
                    "The {} {} gets EdgeTX up to {}; {} is not for it.",
                    p.maker,
                    p.model,
                    p.edgetx_series,
                    version.trim()
                ),
            ));
        }
    }
    if p.firmware_format == FirmwareFormat::Uf2 {
        return Err(Refusal::new(RefusalCode::Incompatible, uf2_route(p)));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
