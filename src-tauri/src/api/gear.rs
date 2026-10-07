//! Gear's rows of the `api` table, and their params and results.
//!
//! `with_gear_rows!` takes the main table's rows (in `api/mod.rs`), adds these, and hands
//! the whole table to `api!`, so there stays one table, one `dispatch` and one
//! `Core::METHODS`. A package adds its row here, its `Core` method in `core/gear.rs`, and
//! its typed command to `specta_builder` in `lib.rs`.

pub use crate::core::{DeviceSaveParams, GearStatus, ReminderParams};
pub use crate::gear::model::Device;
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
            /// Stops the "still inserted" reminder for a device's link. True when one was armed.
            gear_dismiss_reminder(params: ReminderParams) -> bool = |c| Ok(c.gear_dismiss(&params.handle));
        }
    };
}
