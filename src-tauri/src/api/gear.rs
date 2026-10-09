//! Gear's rows of the `api` table, and their params and results.
//!
//! `with_gear_rows!` takes the main table's rows (in `api/mod.rs`), adds these, and hands
//! the whole table to `api!`, so there stays one table, one `dispatch` and one
//! `Core::METHODS`. A package adds its row here, its `Core` method in `core/gear.rs`, and
//! its typed command to `specta_builder` in `lib.rs`.

pub use crate::core::{
    BackupDiffParams, BackupFilter, BackupParams, BackupPinParams, BackupReadParams, BackupResult,
    BackupSummary, CardCheckParams, CardChecksParams, CardRepairParams, ExportParams, GearJob,
    ImportBackupsParams, PruneParams, RepairResult, StopParams,
};
pub use crate::core::{
    BoardNotesParams, CardClean, CardCleanParams, CardParams, CardPreview, CardPreviewParams,
    DeviceSaveParams, FcJob, FcPortParams, FcReadParams, GearCard, GearStatus, OsdParams,
    PollPauseParams, RadioParams, RadioWatchParams, RatesParams, RatesPreview, RatesPreviewParams,
    ReminderParams, SimsParams, SwitchMapParams, UsbTimer,
};
pub use crate::core::{
    CalibrateParams, CalibrateView, SimCalibration, SimCalibrationParams, SimCalibrationSaveParams,
    SimDefaultsParams, SimValidateParams,
};
pub use crate::core::{
    CardMountParams, CardMounted, ChangeUpdateParams, CopyParams, DfuLinkParams, DfuLinked,
    ModelDetail, ModelEditParams, ModelParams, OsdEditParams, PackInstallParams, RadioCliAction,
    RadioCliParams, RadioCliReport, RadioMatch, RadioVerify, RenderReport, RestoreParams,
    SimRestoreParams, SimRestoreRequest, StageParams, VoiceChooseParams, VoiceEditParams,
    VoiceLine, VoicePack, VoiceParams, VoicePreviewParams, VoiceRenderParams, VoiceView,
};
pub use crate::core::{
    CrashSaveParams, FlightFilter, FlightFoldersParams, FlightReport, FlightSetParams, FlightsView,
    NotesParams, PackSaveParams, PacksParams, ReportParams, ReportSaveParams, ReportSaved,
};
pub use crate::core::{
    FirmwareParams, FirmwareRead, FirmwareReadParams, FirmwareView, FlashParams, FlashRequest,
};
pub use crate::gear::apply::sim::{SimSyncParams, SimSyncRequest, SimTarget};
pub use crate::gear::apply::{ApplyPlanParams, ApplyReport, ApplyRequest, StepReport, StepState};
pub use crate::gear::backup::{
    BackupContent, ExportReport, ImportBackupsReport, PruneReport, StorageView,
};
pub use crate::gear::bf::boards::BoardNote;
pub use crate::gear::bf::{FcInfo, FcRead};
pub use crate::gear::changes::ChangeFilter;
pub use crate::gear::copy::CopyPlan;
pub use crate::gear::crashes::{Crash, CrashFilter};
pub use crate::gear::health::CardCheck;
pub use crate::gear::model::Device;
pub use crate::gear::model::DiffItem;
pub use crate::gear::model::{ApplyPlan, StagedChange};
pub use crate::gear::osd::{OsdCopy, OsdMove, OsdView};
pub use crate::gear::packs::{Pack, PackType, PacksView};
pub use crate::gear::preflight::Preflight;
pub use crate::gear::radio_hid::RadioSnapshot;
pub use crate::gear::rates::RatesView;
pub use crate::gear::report::SessionReport;
pub use crate::gear::sim_cal::{SavedCalibration, SimDefaults};
pub use crate::gear::sims::SimStatus;
pub use crate::gear::splash::{SplashParams, SplashPreview};
pub use crate::gear::switchmap::SwitchMap;
pub use quadcam_sim::validate::{CheckOutcome, CheckResult, ValidationReport};
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
            /// Pauses or resumes QuadCam's own reads of an FC port (the USB timer's probe and the
            /// on-connect backup). Returns the paused ports. Lasts until QuadCam quits.
            gear_poll_pause(params: PollPauseParams) -> Vec<String> = |c| c.gear_poll_pause(&params);
            /// Stops the "still inserted" reminder for a device's link. True when one was armed.
            gear_dismiss_reminder(params: ReminderParams) -> bool = |c| Ok(c.gear_dismiss(&params.handle));
            /// An FC's OSD layout per OSD profile, drawn on its grid and checked for overlaps
            /// and cells off screen. Reads only.
            gear_osd(params: OsdParams) -> OsdView = |c| c.gear_osd(&params);
            /// Stages OSD moves, toggles or a profile copy for an FC as one "OSD layout"
            /// change. Writes only QuadCam's own data; the apply sheet writes the FC.
            gear_osd_edit(params: OsdEditParams) -> StagedChange = |c| c.gear_osd_edit(&params);
            /// An FC's rate profiles from its latest backup, a backup or dump files: names, the
            /// roll, pitch and yaw curves with maximum and centre rates, the throttle curve.
            /// Reads only.
            gear_rates(params: RatesParams) -> RatesView = |c| c.gear_rates(&params);
            /// A rate profile as edited, with fresh curves; with `to`, fitted onto that rate model
            /// first, with the error per axis. Reads and writes nothing.
            gear_rates_preview(params: RatesPreviewParams) -> RatesPreview = |c| c.gear_rates_preview(&params);
            /// The sims on this Mac with their rate profiles; with a quad, how each differs
            /// from the quad's rates. Reads only.
            gear_sims(params: SimsParams) -> Vec<SimStatus> = |c| c.gear_sims(&params);
            /// What syncing a quad's rate profile into the sims would write: per sim file the
            /// values that change, every guard, warnings and a digest. Writes nothing.
            gear_sim_sync_plan(params: SimSyncParams) -> ApplyPlan = |c| c.gear_sim_sync_plan(&params);
            /// Writes the planned sim files: a backup of each, an atomic write, a read back.
            /// Refuses while a game runs. Needs the plan's digest and confirm=true; with the
            /// app running the person also clicks Apply in its sheet.
            gear_sim_sync(params: SimSyncRequest) -> ApplyReport = |c| c.gear_sim_sync(&params);
            /// What putting a sim's file back from a backup would write: the checks, the rates that change, a warning and a digest. Writes nothing.
            gear_sim_restore_plan(params: SimRestoreParams) -> ApplyPlan = |c| c.gear_sim_restore_plan(&params);
            /// Puts a sim's rate file back from a backup. Needs the plan's digest and confirm; the current file is backed up first.
            gear_sim_restore(params: SimRestoreRequest) -> ApplyReport = |c| c.gear_sim_restore(&params);
            /// Each saved device's firmware against the newest release (EdgeTX, Betaflight,
            /// ExpressLRS). Reads the network only when `check` is true, or unset with
            /// `firmwareCheck` set to `daily` and the last answer a day old.
            gear_firmware(params: FirmwareParams) -> FirmwareView = |c| c.gear_firmware(&params);
            /// A picture at an EdgeTX radio's splash size (128 x 64, 1 bit) with a threshold
            /// and invert, as a 4x PNG. Reads the image file only.
            gear_splash(params: SplashParams) -> SplashPreview = |c| c.gear_splash(&params);
            /// What flashing an EdgeTX radio would do: the release's board binary (downloaded
            /// on first use), the splash patch, every guard and a digest. Writes no device.
            gear_flash_plan(params: FlashParams) -> ApplyPlan = |c| c.gear_flash_plan(&params);
            /// The read-only DFU trial: reads the flash of the one radio in DFU mode twice, saves
            /// it as a firmware copy and compares the version the image names with the radio's
            /// known version. Cannot erase, write or restart the radio (`firmware-read-progress` events).
            gear_firmware_read(params: FirmwareReadParams) -> FirmwareRead = |c| c.gear_firmware_read(&params);
            /// Flashes the planned firmware over DFU: reads the radio's current firmware as a
            /// kept backup, erases, writes, reads back and compares, then leaves DFU. Needs
            /// the plan's digest and confirm=true; with the app running the person also clicks
            /// Apply in its sheet.
            gear_flash(params: FlashRequest) -> ApplyReport = |c| c.gear_flash(&params);
            /// An EdgeTX card: models, the selected model and its aircraft, the radio clock, one model in full.
            gear_card(params: CardParams) -> GearCard = |c| c.gear_card(&params);
            /// A radio's model for the editors: timers, value screens, logging, alarms, callouts and
            /// the checklist, from the mounted card or the latest backup, with staged edits on top. Reads only.
            gear_model(params: ModelParams) -> ModelDetail = |c| c.gear_model(&params);
            /// Stages model editor ops (and a checklist) for a radio as its one "Model edits" change.
            /// Writes only QuadCam's own data; the apply sheet writes the card.
            gear_model_edit(params: ModelEditParams) -> StagedChange = |c| c.gear_model_edit(&params);
            /// The radio's voice: QuadCam's lines with the spoken text, the installed and available
            /// packs, the provider, and a radio's overrides. Reads only; `refresh_index` also reads the
            /// pack index.
            gear_voice(params: VoiceParams) -> VoiceView = |c| c.gear_voice(&params);
            /// A sound the app can play: a pack's take of a line, or the person's own render, copied
            /// into the cache. Returns the file's path.
            gear_voice_preview(params: VoicePreviewParams) -> String = |c| c.gear_voice_preview(&params);
            /// Overrides one line on one radio: another pack's take, or the person's own text
            /// rendered with their provider. Writes only QuadCam's own data.
            gear_voice_edit(params: VoiceEditParams) -> VoiceLine = |c| c.gear_voice_edit(&params);
            /// Renders QuadCam's lines (and the person's own) with the provider from the settings into
            /// a local pack. A render that costs money waits for `confirm`; `dry_run` only reports.
            gear_voice_render(params: VoiceRenderParams) -> RenderReport = |c| c.gear_voice_render(&params);
            /// Installs a voice pack from the index after a hash check.
            gear_voice_pack_install(params: PackInstallParams) -> VoicePack = |c| c.gear_voice_pack_install(&params);
            /// Stages one card change that puts a pack's sounds on a radio, keeping the person's
            /// per-line overrides when asked. The apply sheet writes the card.
            gear_voice_choose(params: VoiceChooseParams) -> StagedChange = |c| c.gear_voice_choose(&params);
            /// Checks and diffs EdgeTX card edits. Writes nothing.
            gear_card_preview(params: CardPreviewParams) -> CardPreview = |c| c.gear_card_preview(&params);
            /// The switch map: each control's positions with their channel values, FC modes
            /// and radio effects, from an EdgeTX model and a Betaflight dump; with `live`,
            /// where each control is now. Reads only.
            gear_switch_map(params: SwitchMapParams) -> SwitchMap = |c| c.gear_switch_map(&params);
            /// One look at the radio in USB Joystick mode: its buttons, axes and channels.
            gear_radio(params: RadioParams) -> RadioSnapshot = |c| c.gear_radio(&params);
            /// Starts or stops the radio stream the app's controls page draws (`radio-input`
            /// events). True while it runs.
            gear_radio_watch(params: RadioWatchParams) -> bool = |c| c.gear_radio_watch(&params);
            /// The sim's calibration of a radio; with no radio named, the joystick plugged in,
            /// matched to a saved radio (or a provisional key). Reads only.
            gear_sim_calibration(params: SimCalibrationParams) -> SimCalibration = |c| c.gear_sim_calibration(&params);
            /// Saves a radio's sim calibration, keyed by its Gear radio id.
            gear_sim_calibration_save(params: SimCalibrationSaveParams) -> SavedCalibration = |c| c.gear_sim_calibration_save(&params);
            /// What the sim pre-fills for an aircraft: stick channels, arm, angle, horizon,
            /// turtle and air mode switches, a reset control, each with its source. Reads only.
            gear_sim_defaults(params: SimDefaultsParams) -> SimDefaults = |c| c.gear_sim_defaults(&params);
            /// A sim profile checked against a folder of decoded blackbox logs: each check's
            /// band and result (sim-design 6.4). Reads only.
            gear_sim_validate(params: SimValidateParams) -> ValidationReport = |c| c.gear_sim_validate(&params);
            /// Drives the sim's calibration session (`sim-calibration-event` events).
            gear_sim_calibrate(params: CalibrateParams) -> CalibrateView = |c| c.gear_sim_calibrate(&params);
            /// Flights from the radio logs: hover, sag, resting voltage, mAh, the threshold,
            /// the worst link and dropouts, with the pack, place and clip of each.
            gear_flights(params: FlightFilter) -> FlightsView = |c| c.gear_flights(&params);
            /// Sets a flight's pack or place ("" clears one).
            gear_flight_set(params: FlightSetParams) -> FlightReport = |c| c.gear_flight_set(&params);
            /// Adds or removes a log folder the flights read; returns the list.
            gear_flight_folders(params: FlightFoldersParams) -> Vec<std::path::PathBuf> = |c| c.gear_flight_folders(&params);
            /// Packs with their history, pack types with the charging sheet, the notes.
            gear_packs(params: PacksParams) -> PacksView = |c| c.gear_packs(&params);
            /// Saves a pack; `charged` marks it charged now (true) or clears the mark.
            gear_pack_save(params: PackSaveParams) -> Pack = |c| c.gear_pack_save(&params);
            /// Deletes a pack. Its flights keep their label.
            gear_pack_delete(params: NameParams) -> Pack = |c| c.gear_pack_delete(&params.name);
            /// Saves a pack type.
            gear_pack_type_save(params: PackType) -> PackType = |c| c.gear_pack_type_save(&params);
            /// Deletes a pack type no pack uses.
            gear_pack_type_delete(params: NameParams) -> PackType = |c| c.gear_pack_type_delete(&params.name);
            /// Saves the charging sheet's notes.
            gear_pack_notes(params: NotesParams) -> String = |c| c.gear_pack_notes(&params);
            /// The session report for a day, else the last import's days, with its Markdown.
            gear_session_report(params: ReportParams) -> SessionReport = |c| c.gear_session_report(&params);
            /// Writes the session report's Markdown to a file.
            gear_session_report_save(params: ReportSaveParams) -> ReportSaved = |c| c.gear_session_report_save(&params);
            /// The "Pack up" check before a session: packs, radio model, card space,
            /// backups, cards still in. Reads only.
            gear_preflight() -> Preflight = |c| c.gear_preflight();
            /// Crashes, narrowed by aircraft or clip.
            gear_crashes(params: CrashFilter) -> Vec<Crash> = |c| c.gear_crashes(&params);
            /// Saves a crash on a clip: the time in the clip, what broke, the parts used.
            gear_crash_save(params: CrashSaveParams) -> Crash = |c| c.gear_crash_save(&params);
            /// Deletes a crash.
            gear_crash_delete(params: IdParams) -> Crash = |c| c.gear_crash_delete(&params.id);
            /// Backs up a radio card or an FC (the FC reboots). Writes nothing when nothing changed.
            gear_backup(params: BackupParams) -> BackupResult = |c| c.gear_backup(&params);
            /// Snapshots, newest first, without their file lists.
            gear_backups(params: BackupFilter) -> Vec<BackupSummary> = |c| c.gear_backups(&params);
            /// A snapshot with its files, or one file's content.
            gear_backup_read(params: BackupReadParams) -> BackupContent = |c| c.gear_backup_read(&params);
            /// What changed between two snapshots (from the one before `a` when `b` is empty).
            gear_backup_diff(params: BackupDiffParams) -> Vec<DiffItem> = |c| c.gear_backup_diff(&params);
            /// Pins a snapshot (never pruned) or unpins it.
            gear_backup_pin(params: BackupPinParams) -> BackupSummary = |c| c.gear_backup_pin(&params);
            /// Sizes of the gear folder, in total and per device.
            gear_storage() -> StorageView = |c| c.gear_storage();
            /// Thins snapshots by the retention settings and removes blobs nothing names.
            gear_prune(params: PruneParams) -> PruneReport = |c| c.gear_prune(&params);
            /// Writes a snapshot, or a device's snapshots, as plain folders.
            gear_export(params: ExportParams) -> ExportReport = |c| c.gear_export(&params);
            /// Imports an old backup folder: card copies, FC diff and dump files, LOGS folders.
            gear_import_backups(params: ImportBackupsParams) -> ImportBackupsReport = |c| c.gear_import_backups(&params);
            /// Links the radio in DFU mode to a saved radio (the pick, else the radio seen last), or removes the link.
            gear_dfu_link(params: DfuLinkParams) -> DfuLinked = |c| c.gear_dfu_link(&params);
            /// A radio on its USB serial port (EdgeTX CLI): identify, ls, play, beep, reboot, or verify the card's files against a backup.
            gear_radio_cli(params: RadioCliParams) -> RadioCliReport = |c| c.gear_radio_cli(&params);
            /// Lists the ._ files macOS left on a card; with remove and confirm, deletes them.
            gear_card_clean(params: CardCleanParams) -> CardClean = |c| c.gear_card_clean(&params);
            /// Checks a card's file system (diskutil verifyVolume) and logs the result.
            gear_card_check(params: CardCheckParams) -> CardCheck = |c| c.gear_card_check(&params);
            /// A card's checks, newest first.
            gear_card_checks(params: CardChecksParams) -> Vec<CardCheck> = |c| Ok(c.gear_card_checks(&params));
            /// Repairs a card whose latest check failed: a backup first, the repair, a check after.
            gear_card_repair(params: CardRepairParams) -> RepairResult = |c| c.gear_card_repair(&params);
            /// Staged changes: those waiting by default, or the bench history with `history`.
            gear_changes(params: ChangeFilter) -> Vec<StagedChange> = |c| c.gear_changes(&params);
            /// Stages edits for a device (FC settings as raw CLI lines or `set`s). Refuses
            /// names the FC's latest backup does not hold and lines the engine never sends.
            gear_change_stage(params: StageParams) -> StagedChange = |c| c.gear_change_stage(&params);
            /// Edits a staged change: its edits, title, note, order, or draft/ready.
            gear_change_update(params: ChangeUpdateParams) -> StagedChange = |c| c.gear_change_update(&params);
            /// Discards a staged change. It stays in the history.
            gear_change_discard(params: IdParams) -> StagedChange = |c| c.gear_change_discard(&params.id);
            /// Stages an FC backup's settings back as a change.
            gear_restore_stage(params: RestoreParams) -> StagedChange = |c| c.gear_restore_stage(&params);
            /// Runs every guard for a staged change and builds its diff and digest. Writes
            /// nothing and does not reboot the FC.
            gear_apply_plan(params: ApplyPlanParams) -> ApplyPlan = |c| c.gear_apply_plan(&params);
            /// Applies a staged change to an FC or a radio card: backup, write, read back,
            /// verify; a card is mounted for it and unmounted after. Needs the plan's digest
            /// and confirm=true; with the app running the person also clicks Apply in its sheet.
            gear_apply(params: ApplyRequest) -> ApplyReport = |c| c.gear_apply(&params);
            /// Keeps an applied Try change.
            gear_change_keep(params: IdParams) -> StagedChange = |c| c.gear_change_keep(&params.id);
            /// Stages a restore of the backup an applied change took; the change becomes
            /// Reverted when that restore verifies.
            gear_change_revert(params: IdParams) -> StagedChange = |c| c.gear_change_revert(&params.id);
            /// What copying settings from one FC's backup to another would stage: checks, diff.
            gear_copy_plan(params: CopyParams) -> CopyPlan = |c| c.gear_copy_plan(&params);
            /// Stages the copy as one change for the target FC.
            gear_copy_stage(params: CopyParams) -> StagedChange = |c| c.gear_copy_stage(&params);
            /// Mounts an unmounted radio card for the person to browse; it unmounts again
            /// after the minutes given (10 by default) or on gear_card_unmount.
            gear_card_mount(params: CardMountParams) -> CardMounted = |c| c.gear_card_mount(&params);
            /// Unmounts a radio card (Done) and plays the safe-to-unplug cue.
            gear_card_unmount(params: CardMountParams) -> bool = |c| c.gear_card_unmount(&params).map(|_| true);
            /// Stops a running backup or card check on a link. True when one was running.
            gear_stop(params: StopParams) -> bool = |c| Ok(c.gear_stop(&params.handle));
        }
    };
}
