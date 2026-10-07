//! `quadcam-cli gear flights | report | preflight`: flights from the radio logs, the
//! session report, and the "Pack up" check.

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct FlightsArgs {
    /// Only this day (YYYY-MM-DD).
    #[arg(long)]
    day: Option<NaiveDate>,
    /// Only this aircraft profile.
    #[arg(long)]
    aircraft: Option<String>,
    /// Only flights on this pack.
    #[arg(long)]
    pack: Option<String>,
    /// Only flights at this place.
    #[arg(long)]
    place: Option<String>,
    /// One more log folder to read, this run only.
    #[arg(long)]
    logs: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<FlightsCmd>,
}

#[derive(Subcommand)]
pub enum FlightsCmd {
    /// Set a flight's pack or place ("" clears it).
    Set {
        /// The flight id (from `gear flights`).
        flight: String,
        #[arg(long)]
        pack: Option<String>,
        #[arg(long)]
        place: Option<String>,
    },
    /// The log folders flights read: list, --add or --remove one.
    Folders {
        #[arg(long)]
        add: Option<PathBuf>,
        #[arg(long)]
        remove: Option<PathBuf>,
    },
}

#[derive(Args)]
pub struct ReportArgs {
    /// The day (YYYY-MM-DD). Default: the days of the last import.
    #[arg(long)]
    day: Option<NaiveDate>,
    /// Print the Markdown only.
    #[arg(long)]
    markdown: bool,
}

pub fn flights(core: &Core, a: FlightsArgs) -> Result<Value> {
    Ok(match a.cmd {
        None => serde_json::to_value(call::gear_flights(
            core,
            api::FlightFilter {
                day: a.day,
                aircraft: a.aircraft,
                pack: a.pack,
                place: a.place,
                logs: a.logs,
            },
        )?)?,
        Some(FlightsCmd::Set {
            flight,
            pack,
            place,
        }) => serde_json::to_value(call::gear_flight_set(
            core,
            api::FlightSetParams {
                flight,
                pack,
                place,
            },
        )?)?,
        Some(FlightsCmd::Folders { add, remove }) => serde_json::to_value(
            call::gear_flight_folders(core, api::FlightFoldersParams { add, remove })?,
        )?,
    })
}

pub fn report(core: &Core, a: ReportArgs) -> Result<Value> {
    let r = call::gear_session_report(core, api::ReportParams { day: a.day })?;
    Ok(if a.markdown {
        Value::String(r.markdown)
    } else {
        serde_json::to_value(r)?
    })
}

pub fn preflight(core: &Core) -> Result<Value> {
    Ok(serde_json::to_value(call::gear_preflight(core)?)?)
}
