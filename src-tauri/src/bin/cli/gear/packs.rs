//! `quadcam-cli gear packs | crashes`: packs, pack types and the charging notes; the crash
//! and repair log.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Args, Subcommand};
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::packs::{Chemistry, Pack, PackType};
use serde_json::Value;

#[derive(Args)]
pub struct PacksArgs {
    /// The resting volts per cell the warning suggestion aims for (default 3.7).
    #[arg(long)]
    target_v: Option<f64>,
    #[command(subcommand)]
    cmd: Option<PacksCmd>,
}

#[derive(Subcommand)]
pub enum PacksCmd {
    /// Add or change a pack. Fields left out keep their value.
    Save {
        label: String,
        /// Its pack type.
        #[arg(long = "type")]
        pack_type: Option<String>,
        /// The day it arrived (YYYY-MM-DD).
        #[arg(long)]
        received: Option<NaiveDate>,
        #[arg(long)]
        retired: Option<bool>,
        /// Mark it charged now.
        #[arg(long, conflicts_with = "not_charged")]
        charged: bool,
        /// Clear the charged mark.
        #[arg(long)]
        not_charged: bool,
        #[arg(long)]
        note: Option<String>,
    },
    /// Delete a pack.
    Delete { label: String },
    /// Pack types: save or delete one.
    Type {
        #[command(subcommand)]
        cmd: TypeCmd,
    },
    /// Set the charging sheet's notes.
    Notes { text: String },
}

#[derive(Subcommand)]
pub enum TypeCmd {
    /// Add or change a pack type. Fields left out keep their value.
    Save {
        name: String,
        /// lipo, lihv or liion.
        #[arg(long)]
        chemistry: Option<String>,
        #[arg(long)]
        cells: Option<u8>,
        #[arg(long)]
        capacity: Option<f64>,
        #[arg(long)]
        connector: Option<String>,
        /// Full charge, volts per cell.
        #[arg(long)]
        full: Option<f64>,
        /// Storage charge, volts per cell.
        #[arg(long)]
        storage: Option<f64>,
        /// Charge current, amps.
        #[arg(long)]
        charge_a: Option<f64>,
        /// The radio's mAh warning for this type.
        #[arg(long)]
        warn_mah: Option<f64>,
    },
    /// Delete a pack type no pack uses.
    Delete { name: String },
}

#[derive(Args)]
pub struct CrashesArgs {
    #[arg(long, global = true)]
    aircraft: Option<String>,
    #[arg(long, global = true)]
    clip: Option<String>,
    #[command(subcommand)]
    cmd: Option<CrashCmd>,
}

#[derive(Subcommand)]
pub enum CrashCmd {
    /// Log a crash (no --id) or change one.
    Save {
        #[arg(long)]
        id: Option<String>,
        /// Seconds into the clip.
        #[arg(long)]
        time: Option<f64>,
        #[arg(long)]
        day: Option<NaiveDate>,
        /// What broke.
        #[arg(long)]
        broke: Option<String>,
        /// Parts used, comma-separated.
        #[arg(long, value_delimiter = ',')]
        parts: Option<Vec<String>>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        repaired: Option<bool>,
    },
    /// Delete a crash.
    Delete { id: String },
}

fn chemistry(s: &str) -> Result<Chemistry> {
    serde_json::from_value(Value::String(s.to_lowercase()))
        .with_context(|| format!("chemistry {s:?}: use lipo, lihv or liion"))
}

pub fn packs(core: &Core, a: PacksArgs) -> Result<Value> {
    let view = || {
        call::gear_packs(
            core,
            api::PacksParams {
                target_v: a.target_v,
            },
        )
    };
    Ok(match a.cmd {
        None => serde_json::to_value(view()?)?,
        Some(PacksCmd::Save {
            label,
            pack_type,
            received,
            retired,
            charged,
            not_charged,
            note,
        }) => {
            let mut p = view()?
                .packs
                .into_iter()
                .map(|v| v.pack)
                .find(|p| p.label == label)
                .unwrap_or(Pack {
                    label,
                    ..Default::default()
                });
            if let Some(t) = pack_type {
                p.pack_type = Some(t);
            }
            if received.is_some() {
                p.received = received;
            }
            if let Some(r) = retired {
                p.retired = r;
            }
            if let Some(n) = note {
                p.note = n;
            }
            let charged = if charged {
                Some(true)
            } else if not_charged {
                Some(false)
            } else {
                None
            };
            serde_json::to_value(call::gear_pack_save(
                core,
                api::PackSaveParams { pack: p, charged },
            )?)?
        }
        Some(PacksCmd::Delete { label }) => serde_json::to_value(call::gear_pack_delete(
            core,
            api::NameParams { name: label },
        )?)?,
        Some(PacksCmd::Notes { text }) => {
            serde_json::to_value(call::gear_pack_notes(core, api::NotesParams { text })?)?
        }
        Some(PacksCmd::Type { cmd }) => match cmd {
            TypeCmd::Delete { name } => {
                serde_json::to_value(call::gear_pack_type_delete(core, api::NameParams { name })?)?
            }
            TypeCmd::Save {
                name,
                chemistry: chem,
                cells,
                capacity,
                connector,
                full,
                storage,
                charge_a,
                warn_mah,
            } => {
                let mut t = view()?
                    .types
                    .into_iter()
                    .map(|v| v.pack_type)
                    .find(|t| t.name == name)
                    .unwrap_or(PackType {
                        name,
                        cells: 1,
                        ..Default::default()
                    });
                if let Some(c) = chem {
                    t.chemistry = chemistry(&c)?;
                }
                if let Some(c) = cells {
                    t.cells = c;
                }
                t.capacity_mah = capacity.or(t.capacity_mah);
                t.connector = connector.or(t.connector);
                t.full_v = full.or(t.full_v);
                t.storage_v = storage.or(t.storage_v);
                t.charge_a = charge_a.or(t.charge_a);
                t.warn_mah = warn_mah.or(t.warn_mah);
                serde_json::to_value(call::gear_pack_type_save(core, t)?)?
            }
        },
    })
}

pub fn crashes(core: &Core, a: CrashesArgs) -> Result<Value> {
    Ok(match a.cmd {
        None => serde_json::to_value(call::gear_crashes(
            core,
            api::CrashFilter {
                aircraft: a.aircraft,
                clip: a.clip,
            },
        )?)?,
        Some(CrashCmd::Save {
            id,
            time,
            day,
            broke,
            parts,
            note,
            repaired,
        }) => serde_json::to_value(call::gear_crash_save(
            core,
            api::CrashSaveParams {
                id,
                clip: a.clip,
                time_s: time,
                aircraft: a.aircraft,
                day,
                broke,
                parts,
                note,
                repaired,
            },
        )?)?,
        Some(CrashCmd::Delete { id }) => {
            serde_json::to_value(call::gear_crash_delete(core, api::IdParams { id })?)?
        }
    })
}
