//! Gear's rows of the `api` table, and their params and results.
//!
//! `with_gear_rows!` takes the main table's rows (in `api/mod.rs`), adds these, and hands
//! the whole table to `api!`, so there stays one table, one `dispatch` and one
//! `Core::METHODS`. A package adds its row here, its `Core` method in `core/gear.rs`, and
//! its typed command to `specta_builder` in `lib.rs`.

pub use crate::core::{
    BoardNotesParams, CardParams, CardPreview, CardPreviewParams, DeviceSaveParams, FcJob,
    FcPortParams, FcReadParams, GearCard, GearStatus, OsdParams, ReminderParams, UsbTimer,
};
pub use crate::gear::bf::boards::BoardNote;
pub use crate::gear::bf::{FcInfo, FcRead};
pub use crate::gear::model::Device;
pub use crate::gear::osd::OsdView;
use serde::{Deserialize, Serialize};
use specta::Type;

/// A device, a backup or a change, by id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct IdParams {
    pub id: String,
}

/// Appends Gear's rows to the table it is given, then calls `api!` with all of them.
macro_rules! with_gear_rows {
    ($($rows:tt)*) => {
        api! {
            $($rows)*
            /// Gear: the gear folder, the Gear settings, and what is plugged in now.
            gear_status() -> GearStatus = |c| c.gear_status();
            /// The devices QuadCam knows.
            gear_devices() -> Vec<Device> = |c| c.gear_devices();
            /// Names a device or links it to an aircraft profile.
            gear_device_save(params: DeviceSaveParams) -> Device = |c| c.gear_device_save(&params);
            /// Forgets a device. Its backups stay.
            gear_device_forget(params: IdParams) -> Device = |c| c.gear_device_forget(&params.id);
            /// Reads an FC's identity over MSP (board, firmware, version, its device id). No
            /// reboot. One cue at the end.
            gear_fc_identify(params: FcPortParams) -> FcJob<FcInfo> = |c| c.gear_fc_identify(&params);
            /// Reads an FC through its CLI: read-only commands, a backup's set by default. The FC
            /// reboots when it ends. Writes nothing.
            gear_fc_read(params: FcReadParams) -> FcJob<FcRead> = |c| c.gear_fc_read(&params);
            /// Known issues of FC boards and builds, for the device page.
            gear_board_notes(params: BoardNotesParams) -> Vec<BoardNote> = |c| Ok(c.gear_board_notes(&params));
            /// Each FC's USB heat timer: battery in, seconds on USB, the limit, seconds left.
            gear_usb_timers() -> Vec<UsbTimer> = |c| Ok(c.gear_usb_timers());
            /// Stops the "still inserted" reminder for a device's link. True when one was armed.
            gear_dismiss_reminder(params: ReminderParams) -> bool = |c| Ok(c.gear_dismiss(&params.handle));
            /// An FC's OSD layout per OSD profile, drawn on its grid and checked for overlaps
            /// and cells off screen. Reads only.
            gear_osd(params: OsdParams) -> OsdView = |c| c.gear_osd(&params);
            /// An EdgeTX card: models, the selected model and its aircraft, the radio clock, one model in full.
            gear_card(params: CardParams) -> GearCard = |c| c.gear_card(&params);
            /// Checks and diffs EdgeTX card edits. Writes nothing.
            gear_card_preview(params: CardPreviewParams) -> CardPreview = |c| c.gear_card_preview(&params);
        }
    };
}
