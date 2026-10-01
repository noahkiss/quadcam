//! quadcam-cli: everything the GUI does, for scripts and coding agents. Every command takes
//! `--json`. It drives the same `core::Core` as the app, on a session file, so separate runs
//! continue one session. `quadcam-cli mcp` starts the MCP server on stdio.

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use quadcam_lib::core::{Core, FormatRequest, ImportOptions, LogChoice};
use quadcam_lib::media::{self, Encoder, Format};
use quadcam_lib::moments::Span;
use quadcam_lib::photos::{self, PhotosLibrary, Recorder};
use quadcam_lib::session::{Editor, PlanPatch};
use quadcam_lib::{disk, scan};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "quadcam-cli",
    version,
    about = "Import analog FPV DVR clips: stage, date, name, convert, verify, share, format"
)]
struct Cli {
    /// Print one JSON object: {"ok":true,"result":...} or {"ok":false,"error":{...}}.
    #[arg(long, global = true)]
    json: bool,
    /// Session file (default: ~/Library/Caches/app.quadcam/session.json).
    #[arg(long, global = true, value_name = "FILE")]
    session: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List removable volumes: DVR cards and radio log sources.
    Cards,
    /// List the clips on a volume or folder without copying anything.
    Scan { path: PathBuf },
    /// Copy every clip off a card or folder into staging and start a new session.
    Stage {
        /// Card mount point or folder. Default: the first detected card.
        path: Option<PathBuf>,
    },
    /// Probe staged clips, recover half-written ones, make thumbnails.
    Analyze,
    /// Date clips from radio logs, and override dates per clip.
    Dates {
        /// EdgeTX LOGS folder or the radio's root.
        #[arg(long, conflicts_with = "no_logs")]
        logs: Option<PathBuf>,
        /// Ignore radio logs; every clip gets the import date.
        #[arg(long)]
        no_logs: bool,
        /// Log day to match against (YYYY-MM-DD). Default: newest plausible day.
        #[arg(long)]
        day: Option<NaiveDate>,
        /// Override one clip's date: ID=YYYY-MM-DD. Repeatable.
        #[arg(long = "set", value_name = "ID=DATE")]
        set: Vec<String>,
        /// Set one clip's time of day: ID=HH:MM ("ID=" for noon). Repeatable.
        #[arg(long = "time", value_name = "ID=HH:MM")]
        times: Vec<String>,
    },
    /// Show the current session: clips, plans, results.
    Show,
    /// Forget the current session and delete the session file (staged copies stay).
    Clear,
    /// Show each clip's moments (radio-log sticks and dead air), keep ranges and cuts.
    Moments {
        /// Clip ids. Default: every clip.
        ids: Vec<usize>,
    },
    /// Aircraft profiles: list (default), save, delete, set the default.
    Profiles {
        #[command(subcommand)]
        cmd: Option<ProfCmd>,
    },
    /// Saved places: list (default), search by address or name, save, delete.
    Places {
        #[command(subcommand)]
        cmd: Option<PlaceCmd>,
    },
    /// The app's settings: show (default) or set.
    Settings {
        #[command(subcommand)]
        cmd: Option<SetCmd>,
    },
    /// Set metadata on clips: profile, location, keywords, author.
    Meta {
        /// Clip ids, or `all`.
        #[arg(required = true)]
        ids: Vec<String>,
        /// Aircraft profile name ("" to fall back to the log's model, then the default).
        #[arg(long)]
        profile: Option<String>,
        /// A saved place name.
        #[arg(long, conflicts_with_all = ["location", "clear_location"])]
        place: Option<String>,
        /// LAT,LON in decimal degrees.
        #[arg(long, allow_hyphen_values = true, conflicts_with = "clear_location")]
        location: Option<String>,
        #[arg(long)]
        clear_location: bool,
        /// Comma-separated keywords; replaces the clip's own.
        #[arg(long)]
        keywords: Option<String>,
        #[arg(long)]
        author: Option<String>,
    },
    /// Set the cut ranges of one clip. Each exports as an extra <name>_cutN file on import.
    Cut {
        id: usize,
        /// Ranges in clip seconds or m:ss, for example 12.5-18 or 1:02-1:10. Replaces the list.
        ranges: Vec<String>,
        /// Use the suggested keep ranges (the clip without its dead air).
        #[arg(long, conflicts_with_all = ["ranges", "clear"])]
        keep: bool,
        /// Remove every cut.
        #[arg(long, conflicts_with = "ranges")]
        clear: bool,
        /// Seconds into the clip where the radio log's first armed row falls.
        #[arg(long, allow_hyphen_values = true)]
        log_offset: Option<f64>,
        /// For cuts already exported that the new list drops: keep their files (as clips of
        /// their own) or move them to the Trash.
        #[arg(long, value_parser = ["keep", "trash"])]
        removed: Option<String>,
    },
    /// Convert and verify every non-skipped clip.
    Import {
        /// JSON plan: {"clips":[{"id":0,"name":"..","date":"YYYY-MM-DD","time":"HH:MM","note":"..","skip":false}]}
        #[arg(long, value_name = "FILE")]
        plan: Option<PathBuf>,
        /// Short name for one clip: ID=NAME. Repeatable.
        #[arg(long = "name", value_name = "ID=NAME")]
        names: Vec<String>,
        /// Note for one clip: ID=NOTE. Repeatable.
        #[arg(long = "note", value_name = "ID=NOTE")]
        notes: Vec<String>,
        /// Date for one clip: ID=YYYY-MM-DD. Repeatable.
        #[arg(long = "date", value_name = "ID=DATE")]
        dates: Vec<String>,
        /// Time of day for one clip: ID=HH:MM ("ID=" for noon). Repeatable.
        #[arg(long = "time", value_name = "ID=HH:MM")]
        times: Vec<String>,
        /// Skip a clip. Repeatable.
        #[arg(long = "skip", value_name = "ID")]
        skip: Vec<usize>,
        /// Import a clip that was skipped. Repeatable.
        #[arg(long = "unskip", value_name = "ID")]
        unskip: Vec<usize>,
        /// Add a cut range to a clip: ID=START-END (seconds or m:ss). Repeatable.
        #[arg(long = "cut", value_name = "ID=START-END")]
        cuts: Vec<String>,
        #[arg(long, value_parser = ["mp4", "mov"])]
        format: Option<String>,
        #[arg(long, value_parser = ["videotoolbox", "x264"])]
        encoder: Option<String>,
        /// Output folder (default: ~/Movies/quadcam).
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        keep_originals: bool,
        /// Add HHMM to names of clips dated from a radio log.
        #[arg(long)]
        add_time: bool,
        #[arg(long)]
        add_to_photos: bool,
        /// Photos album (default "Drone"; "" for library only).
        #[arg(long)]
        album: Option<String>,
    },
    /// Re-verify outputs: the session's (default) or one file against its source.
    Verify {
        output: Option<PathBuf>,
        /// Source AVI for a one-off check (frames, duration, streams).
        #[arg(long, requires = "output")]
        source: Option<PathBuf>,
    },
    /// Add videos to Photos: given files, or the session's verified outputs.
    Photos {
        files: Vec<PathBuf>,
        #[arg(long)]
        album: Option<String>,
        /// Check the files and report; add nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Eject a card (mount point or /dev/diskN; default: the session's card).
    Eject { target: Option<String> },
    /// Erase the session's card as FAT32. Runs every guard and refuses without all of
    /// --device, --volume-uuid and --yes.
    Format {
        /// Whole-disk device of the card, for example /dev/disk4.
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        volume_uuid: Option<String>,
        /// FAT32 volume name (default DVR).
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        yes: bool,
        /// Only print what would be erased; runs every guard.
        #[arg(long)]
        plan: bool,
    },
    /// The library: list and search clips, rate and flag them, rebuild the index.
    #[command(subcommand)]
    Library(LibCmd),
    /// Run the MCP server on stdio.
    Mcp,
}

#[derive(Subcommand)]
enum LibCmd {
    /// List clips, newest first.
    List {
        /// Words that must all appear in the name, note, place, aircraft, keywords or file.
        #[arg(long, short)]
        query: Option<String>,
        /// all, last_import, moments, picks, rejected, not_in_photos.
        #[arg(long, value_parser = ["all", "last_import", "moments", "picks", "rejected", "not_in_photos"])]
        group: Option<String>,
        /// One flying day (YYYY-MM-DD).
        #[arg(long)]
        day: Option<NaiveDate>,
        #[arg(long)]
        place: Option<String>,
        #[arg(long)]
        aircraft: Option<String>,
        /// At least this many stars.
        #[arg(long)]
        min_rating: Option<u8>,
    },
    /// Set stars (0 to 5, 0 clears) and pick or reject flags. Written into the files too.
    Rate {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        stars: Option<u8>,
        #[arg(long, conflicts_with_all = ["reject", "unflag"])]
        pick: bool,
        #[arg(long, conflicts_with = "unflag")]
        reject: bool,
        #[arg(long)]
        unflag: bool,
    },
    /// Make the index again from the files (also adopts an existing export folder).
    Rebuild,
    /// Rename a clip's file, its cuts and its original.
    Rename { id: String, name: String },
    /// Change a clip's note, keywords, author, place, aircraft profile, date or time.
    Edit {
        id: String,
        #[arg(long)]
        note: Option<String>,
        /// Comma-separated; replaces the list.
        #[arg(long)]
        keywords: Option<String>,
        #[arg(long)]
        author: Option<String>,
        /// A saved place name ("" removes the location).
        #[arg(long, conflicts_with = "location")]
        place: Option<String>,
        /// LAT,LON in decimal degrees.
        #[arg(long, allow_hyphen_values = true)]
        location: Option<String>,
        /// Aircraft profile name ("" removes the profile's details). Rewrites make, model,
        /// aircraft and the profile's keywords.
        #[arg(long)]
        profile: Option<String>,
        /// New flying day (YYYY-MM-DD). Moves the clip, its cuts and its original.
        #[arg(long)]
        date: Option<NaiveDate>,
        /// Time of day (HH:MM, "" for noon). Without it a new date keeps the clip's time.
        #[arg(long)]
        time: Option<String>,
    },
    /// Set a clip's cut ranges, and write the new ones with --export.
    Cut {
        id: String,
        /// Ranges in seconds or m:ss. Replaces the list.
        ranges: Vec<String>,
        #[arg(long, conflicts_with = "ranges")]
        clear: bool,
        /// For exported cuts the new list drops: keep their files or move them to the Trash.
        #[arg(long, value_parser = ["keep", "trash"])]
        removed: Option<String>,
        /// Write the cuts that are not files yet.
        #[arg(long)]
        export: bool,
    },
    /// Move clips, with their cuts and originals, to the Trash.
    Trash {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Add clips and their cuts to Photos.
    Photos {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        album: Option<String>,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum ProfCmd {
    /// List profiles and the default.
    List,
    /// Create a profile, or change the given fields of an existing one.
    Save {
        name: String,
        #[arg(long)]
        aircraft: Option<String>,
        #[arg(long)]
        camera_make: Option<String>,
        #[arg(long)]
        camera_model: Option<String>,
        /// Analog, DJI O4, Walksnail, HDZero, ...
        #[arg(long)]
        video_system: Option<String>,
        /// Comma-separated; replaces the list.
        #[arg(long)]
        keywords: Option<String>,
        #[arg(long)]
        author: Option<String>,
        /// A saved place name ("" for none).
        #[arg(long)]
        place: Option<String>,
        /// EdgeTX model names, comma-separated; replaces the list.
        #[arg(long)]
        models: Option<String>,
        /// New name for the profile.
        #[arg(long)]
        rename: Option<String>,
        /// Also make it the default profile.
        #[arg(long)]
        default: bool,
    },
    /// Delete a profile.
    Delete { name: String },
    /// Set the default profile ("" for none).
    Default { name: String },
}

#[derive(Subcommand)]
enum PlaceCmd {
    /// List saved places.
    List,
    /// Find an address or a named place: name, address, latitude and longitude.
    Search {
        query: String,
        /// apple or nominatim. Default: the geocoder setting (apple).
        #[arg(long)]
        provider: Option<String>,
        /// At most this many results (1 to 10).
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Create a place, or change an existing one.
    Save {
        name: String,
        /// LAT,LON in decimal degrees.
        #[arg(long, allow_hyphen_values = true, conflicts_with = "search")]
        location: Option<String>,
        /// Take the location from a place search.
        #[arg(long)]
        search: Option<String>,
        /// Which search result to take (1 is the first).
        #[arg(long, default_value_t = 1, requires = "search")]
        pick: usize,
        /// apple or nominatim, for --search.
        #[arg(long, requires = "search")]
        provider: Option<String>,
        /// New name for the place.
        #[arg(long)]
        rename: Option<String>,
    },
    /// Delete a saved place. Profiles that use it lose their default place.
    Delete { name: String },
}

#[derive(Subcommand)]
enum SetCmd {
    /// Show the settings file, its values and the effective settings.
    Show,
    /// Set settings: KEY=VALUE, where VALUE is JSON or plain text ("null" resets a key).
    /// Keys: output_dir, format, encoder, keep_originals, add_time, default_name,
    /// photos_album, format_label, log_dir, layout, place_folders, tunables, geocoder,
    /// default_profile.
    Set {
        #[arg(required = true, value_name = "KEY=VALUE")]
        values: Vec<String>,
    },
}

fn removed(s: Option<String>) -> Option<quadcam_lib::trim::RemovedCuts> {
    s.map(|s| {
        if s == "trash" {
            quadcam_lib::trim::RemovedCuts::Trash
        } else {
            quadcam_lib::trim::RemovedCuts::Keep
        }
    })
}

/// Exit codes: 0 ok, 1 failed, 2 usage, 3 refused by a safety guard, 4 nothing to work on.
fn code_for(msg: &str) -> (i32, &'static str) {
    if msg.starts_with("Refused") {
        (3, "refused")
    } else if msg.starts_with("No clips loaded") || msg.starts_with("No card detected") {
        (4, "no_session")
    } else {
        (1, "failed")
    }
}

fn pair(s: &str) -> Result<(usize, String)> {
    let (id, v) = s
        .split_once('=')
        .with_context(|| format!("{s:?} is not ID=VALUE"))?;
    Ok((
        id.trim()
            .parse()
            .with_context(|| format!("{id:?} is not a clip id"))?,
        v.to_string(),
    ))
}

fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .with_context(|| format!("{s:?} is not YYYY-MM-DD"))
}

/// Seconds from `12.5` or `1:02.5`.
fn secs(s: &str) -> Result<f64> {
    let s = s.trim();
    let v = match s.split_once(':') {
        Some((m, x)) => m
            .parse::<f64>()
            .ok()
            .zip(x.parse::<f64>().ok())
            .map(|(m, x)| m * 60.0 + x),
        None => s.parse().ok(),
    };
    v.with_context(|| format!("{s:?} is not seconds or m:ss"))
}

/// `LAT,LON` in decimal degrees.
fn latlon(s: &str) -> Result<(f64, f64)> {
    let (a, b) = s.split_once(',').context("a location is LAT,LON")?;
    Ok((
        a.trim().parse().context("latitude")?,
        b.trim().parse().context("longitude")?,
    ))
}

fn csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// A range `START-END`.
fn span(s: &str) -> Result<Span> {
    let (a, b) = s
        .split_once('-')
        .with_context(|| format!("{s:?} is not START-END"))?;
    Ok(Span {
        start: secs(a)?,
        end: secs(b)?,
    })
}

#[derive(Deserialize)]
struct PlanFile {
    #[serde(default)]
    clips: Vec<PlanPatch>,
    #[serde(flatten)]
    options: ImportOptions,
}

fn run(cli: Cli) -> Result<Value> {
    let photos: Arc<dyn PhotosLibrary> = match &cli.cmd {
        Cmd::Photos { dry_run: true, .. } => Arc::new(Recorder::default()),
        _ => Core::real_photos(),
    };
    let core = Core::headless(cli.session.clone(), photos.clone());
    Ok(match cli.cmd {
        Cmd::Cards => serde_json::to_value(core.volumes())?,
        Cmd::Scan { path } => {
            let vol = disk::probe_volume(&path);
            json!({"path": path, "volume": vol, "clips": scan::find_clips(&path)})
        }
        Cmd::Stage { path } => serde_json::to_value(core.stage(path.as_deref())?)?,
        Cmd::Analyze => serde_json::to_value(core.analyse()?)?,
        Cmd::Dates {
            logs,
            no_logs,
            day,
            set,
            times,
        } => {
            let choice = match (logs, no_logs) {
                (Some(d), _) => LogChoice::Dir(d),
                (None, true) => LogChoice::None,
                _ => LogChoice::Keep,
            };
            let mut s = core.plan_dates(choice, day)?;
            let patches = set
                .iter()
                .map(|x| {
                    pair(x).and_then(|(id, d)| {
                        Ok(PlanPatch {
                            id,
                            date: Some(date(&d)?),
                            ..Default::default()
                        })
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let mut patches = patches;
            for (id, t) in times.iter().map(|x| pair(x)).collect::<Result<Vec<_>>>()? {
                patches.push(PlanPatch {
                    id,
                    time: Some(t),
                    ..Default::default()
                });
            }
            if !patches.is_empty() {
                s = core.patch(&patches, Editor::User)?;
            }
            json!({"log_dir": s.log_dir, "log_day": s.log_day, "log_days": s.log_days, "warnings": s.date_warnings, "plans": s.plans})
        }
        Cmd::Show => serde_json::to_value(
            core.session()
                .ok_or_else(|| anyhow!("No clips loaded. Run `stage` first."))?,
        )?,
        Cmd::Profiles { cmd } => profiles(&core, cmd.unwrap_or(ProfCmd::List))?,
        Cmd::Places { cmd } => places(&core, cmd.unwrap_or(PlaceCmd::List))?,
        Cmd::Settings { cmd } => match cmd.unwrap_or(SetCmd::Show) {
            SetCmd::Show => serde_json::to_value(core.settings()?)?,
            SetCmd::Set { values } => {
                let mut changes = serde_json::Map::new();
                for kv in &values {
                    let (k, v) = kv
                        .split_once('=')
                        .with_context(|| format!("{kv:?} is not KEY=VALUE"))?;
                    let v = serde_json::from_str::<Value>(v.trim())
                        .unwrap_or_else(|_| Value::String(v.to_string()));
                    changes.insert(k.trim().to_string(), v);
                }
                serde_json::to_value(core.settings_set(&changes)?)?
            }
        },
        Cmd::Meta {
            ids,
            profile,
            place,
            location,
            clear_location,
            keywords,
            author,
        } => {
            let s = core
                .session()
                .ok_or_else(|| anyhow!("No clips loaded. Run `stage` first."))?;
            let ids: Vec<usize> = if ids.iter().any(|i| i == "all") {
                s.clips.iter().map(|c| c.id).collect()
            } else {
                ids.iter()
                    .map(|i| i.parse().with_context(|| format!("{i:?} is not a clip id")))
                    .collect::<Result<_>>()?
            };
            let location = location
                .map(|l| -> Result<quadcam_lib::metadata::Location> {
                    let (a, b) = l.split_once(',').context("--location is LAT,LON")?;
                    Ok(quadcam_lib::metadata::Location {
                        lat: a.trim().parse().context("latitude")?,
                        lon: b.trim().parse().context("longitude")?,
                        name: None,
                    })
                })
                .transpose()?;
            let patches: Vec<PlanPatch> = ids
                .iter()
                .map(|&id| PlanPatch {
                    id,
                    profile: profile.clone(),
                    place: if clear_location {
                        Some(String::new())
                    } else {
                        place.clone()
                    },
                    location: location.clone(),
                    keywords: keywords
                        .as_ref()
                        .map(|k| k.split(',').map(|x| x.trim().to_string()).collect()),
                    author: author.clone(),
                    ..Default::default()
                })
                .collect();
            let s = core.patch(&patches, Editor::User)?;
            json!(s
                .plans
                .iter()
                .filter(|p| ids.contains(&p.id))
                .map(|p| json!({"id": p.id, "metadata": p.meta, "log_model": p.log_model}))
                .collect::<Vec<_>>())
        }
        Cmd::Moments { ids } => {
            let s = core
                .session()
                .ok_or_else(|| anyhow!("No clips loaded. Run `stage` first."))?;
            let want: Vec<u64> = ids.iter().map(|&i| i as u64).collect();
            let view = quadcam_lib::mcp::clip_views(
                &serde_json::to_value(&s)?,
                (!want.is_empty()).then_some(&want[..]),
            );
            let keys = [
                "id",
                "name",
                "duration_s",
                "log_match",
                "log_interval_s",
                "log_offset_s",
                "moments",
                "keep",
                "cuts",
                "cut_results",
            ];
            json!(view
                .iter()
                .map(|c| keys
                    .iter()
                    .map(|k| (k.to_string(), c[*k].clone()))
                    .collect())
                .collect::<Vec<serde_json::Map<String, Value>>>())
        }
        Cmd::Cut {
            id,
            ranges,
            keep,
            clear,
            log_offset,
            removed: removed_files,
        } => {
            let s = core
                .session()
                .ok_or_else(|| anyhow!("No clips loaded. Run `stage` first."))?;
            let cuts = if keep {
                let c = s
                    .clips
                    .iter()
                    .find(|c| c.id == id)
                    .with_context(|| format!("no clip with id {id}"))?;
                let k = c
                    .signal
                    .as_ref()
                    .map(|x| x.keep.clone())
                    .unwrap_or_default();
                if k.is_empty() {
                    bail!("clip {id} has no suggested keep ranges (no dead air found)");
                }
                Some(k)
            } else if clear {
                Some(Vec::new())
            } else if ranges.is_empty() {
                None
            } else {
                Some(ranges.iter().map(|r| span(r)).collect::<Result<Vec<_>>>()?)
            };
            if cuts.is_none() && log_offset.is_none() {
                bail!("give ranges, --keep, --clear or --log-offset");
            }
            let s = core.patch(
                &[PlanPatch {
                    id,
                    cuts,
                    log_offset_s: log_offset,
                    removed_cuts: removed(removed_files),
                    ..Default::default()
                }],
                Editor::User,
            )?;
            let p = s.plans.iter().find(|p| p.id == id).context("no plan")?;
            json!({"id": id, "cuts": p.cuts, "log_offset_s": p.log_offset_s, "moments": p.moments})
        }
        Cmd::Import {
            plan,
            names,
            notes,
            dates,
            times,
            skip,
            unskip,
            cuts,
            format,
            encoder,
            output,
            keep_originals,
            add_time,
            add_to_photos,
            album,
        } => {
            let mut patches = Vec::new();
            let mut opts = ImportOptions::default();
            if let Some(f) = plan {
                let pf: PlanFile = serde_json::from_slice(
                    &std::fs::read(&f).with_context(|| format!("reading {}", f.display()))?,
                )
                .with_context(|| format!("parsing {}", f.display()))?;
                patches.extend(pf.clips);
                opts = pf.options;
            }
            for (id, n) in names.iter().map(|x| pair(x)).collect::<Result<Vec<_>>>()? {
                patches.push(PlanPatch {
                    id,
                    name: Some(n),
                    ..Default::default()
                });
            }
            for (id, n) in notes.iter().map(|x| pair(x)).collect::<Result<Vec<_>>>()? {
                patches.push(PlanPatch {
                    id,
                    note: Some(n),
                    ..Default::default()
                });
            }
            for (id, d) in dates.iter().map(|x| pair(x)).collect::<Result<Vec<_>>>()? {
                patches.push(PlanPatch {
                    id,
                    date: Some(date(&d)?),
                    ..Default::default()
                });
            }
            for (id, t) in times.iter().map(|x| pair(x)).collect::<Result<Vec<_>>>()? {
                patches.push(PlanPatch {
                    id,
                    time: Some(t),
                    ..Default::default()
                });
            }
            patches.extend(skip.iter().map(|&id| PlanPatch {
                id,
                skip: Some(true),
                ..Default::default()
            }));
            patches.extend(unskip.iter().map(|&id| PlanPatch {
                id,
                skip: Some(false),
                ..Default::default()
            }));
            // --cut adds to the clip's current cuts.
            let mut added: Vec<(usize, Vec<Span>)> = Vec::new();
            for (id, r) in cuts.iter().map(|x| pair(x)).collect::<Result<Vec<_>>>()? {
                let sp = span(&r)?;
                match added.iter_mut().find(|(i, _)| *i == id) {
                    Some((_, v)) => v.push(sp),
                    None => {
                        let mut v = core
                            .session()
                            .and_then(|s| {
                                s.plans.iter().find(|p| p.id == id).map(|p| p.cuts.clone())
                            })
                            .unwrap_or_default();
                        v.push(sp);
                        added.push((id, v));
                    }
                }
            }
            patches.extend(added.into_iter().map(|(id, c)| PlanPatch {
                id,
                cuts: Some(c),
                ..Default::default()
            }));
            if !patches.is_empty() {
                core.patch(&patches, Editor::User)?;
            }
            if let Some(f) = format {
                opts.format = Some(if f == "mov" { Format::Mov } else { Format::Mp4 });
            }
            if let Some(e) = encoder {
                opts.encoder = Some(if e == "x264" {
                    Encoder::X264
                } else {
                    Encoder::Videotoolbox
                });
            }
            opts.output_dir = output.or(opts.output_dir);
            opts.keep_originals = Some(keep_originals || opts.keep_originals.unwrap_or(false));
            opts.add_time = Some(add_time || opts.add_time.unwrap_or(false));
            opts.add_to_photos |= add_to_photos;
            opts.album = album.or(opts.album);
            serde_json::to_value(core.import(&opts)?)?
        }
        Cmd::Verify {
            output: Some(out),
            source,
        } => {
            let tools = media::find_tools()?;
            let src = source.context("--source is required when an output file is given")?;
            let sp = media::probe(&tools, &src)?;
            let op = media::probe(&tools, &out)?;
            let mut problems = Vec::new();
            if op.video_streams != 1 {
                problems.push(format!("{} video streams", op.video_streams));
            }
            if op.video_packets != sp.video_packets {
                problems.push(format!(
                    "frame count {} vs source {}",
                    op.video_packets, sp.video_packets
                ));
            }
            if (op.duration - sp.duration).abs() > media::DURATION_TOLERANCE {
                problems.push(format!(
                    "duration {:.3}s vs source {:.3}s",
                    op.duration, sp.duration
                ));
            }
            if problems.is_empty() {
                json!({"ok": true, "output": out, "frames": op.video_packets, "duration": op.duration})
            } else {
                bail!(
                    "verify failed for {}: {}",
                    out.display(),
                    problems.join("; ")
                )
            }
        }
        Cmd::Verify { output: None, .. } => {
            let reports = core.verify(None)?;
            let bad: Vec<_> = reports.iter().filter(|r| !r.ok).collect();
            if !bad.is_empty() {
                bail!(
                    "{} of {} outputs failed verify: {}",
                    bad.len(),
                    reports.len(),
                    bad[0].error.clone().unwrap_or_default()
                );
            }
            serde_json::to_value(reports)?
        }
        Cmd::Photos {
            files,
            album,
            dry_run,
        } => {
            let album = album.or(Some(photos::DEFAULT_ALBUM.to_string()));
            if files.is_empty() {
                serde_json::to_value(core.add_to_photos(None, album)?)?
            } else {
                let report = photos::share(photos.as_ref(), &files, album.as_deref())?;
                json!({"dry_run": dry_run, "report": report})
            }
        }
        Cmd::Clear => {
            core.clear()?;
            json!({"cleared": true})
        }
        Cmd::Eject { target } => {
            core.eject(target.as_deref())?;
            json!({"ejected": true})
        }
        Cmd::Format {
            plan: true, label, ..
        } => serde_json::to_value(core.format_plan(label.as_deref())?)?,
        Cmd::Format {
            device,
            volume_uuid,
            label,
            yes,
            ..
        } => {
            let (Some(device), Some(volume_uuid)) = (device, volume_uuid) else {
                bail!("Refused: format needs --device /dev/diskN and --volume-uuid <uuid> (see `format --plan`).");
            };
            if !yes {
                bail!("Refused: format needs --yes.");
            }
            let req = FormatRequest {
                device,
                volume_uuid,
                label,
                confirm: true,
            };
            // Any reason not to erase is a refusal, so scripts can tell it from a crash.
            let plan = core.format(&req, false).map_err(|e| {
                let m = format!("{e:#}");
                if m.starts_with("Refused") {
                    anyhow!(m)
                } else {
                    anyhow!("Refused: {m}")
                }
            })?;
            serde_json::to_value(plan)?
        }
        Cmd::Library(cmd) => library(&core, cmd)?,
        Cmd::Mcp => unreachable!(),
    })
}

fn library(core: &Core, cmd: LibCmd) -> Result<Value> {
    use quadcam_lib::library::{Filter, Flag};
    Ok(match cmd {
        LibCmd::List {
            query,
            group,
            day,
            place,
            aircraft,
            min_rating,
        } => {
            let v = core.library(&Filter {
                query,
                group,
                day,
                place,
                aircraft,
                min_rating,
            })?;
            serde_json::to_value(v)?
        }
        LibCmd::Rate {
            ids,
            stars,
            pick,
            reject,
            unflag,
        } => {
            let flag = if pick {
                Some(Flag::Pick)
            } else if reject {
                Some(Flag::Reject)
            } else if unflag {
                Some(Flag::None)
            } else {
                None
            };
            if stars.is_none() && flag.is_none() {
                bail!("give --stars, --pick, --reject or --unflag");
            }
            serde_json::to_value(core.library_rate(&ids, stars, flag)?)?
        }
        LibCmd::Rebuild => serde_json::to_value(core.library_rebuild()?)?,
        LibCmd::Rename { id, name } => serde_json::to_value(core.library_rename(&id, &name)?)?,
        LibCmd::Edit {
            id,
            note,
            keywords,
            author,
            place,
            location,
            profile,
            date,
            time,
        } => {
            let location = location
                .map(|l| {
                    latlon(&l).map(|(lat, lon)| quadcam_lib::metadata::Location {
                        lat,
                        lon,
                        name: None,
                    })
                })
                .transpose()?;
            let e = quadcam_lib::core::LibEdit {
                note,
                keywords: keywords.map(|k| csv(&k)),
                author,
                place,
                location,
                date,
                time,
                profile,
            };
            if serde_json::to_value(&e)?
                .as_object()
                .is_some_and(|o| o.values().all(Value::is_null))
            {
                bail!("give at least one of --note, --keywords, --author, --place, --location, --profile, --date, --time");
            }
            serde_json::to_value(core.library_edit(&id, &e)?)?
        }
        LibCmd::Cut {
            id,
            ranges,
            clear,
            removed: r,
            export,
        } => {
            let mut out = json!({});
            if clear || !ranges.is_empty() {
                let cuts = ranges.iter().map(|r| span(r)).collect::<Result<Vec<_>>>()?;
                let change = core.library_set_cuts(&id, &cuts, removed(r))?;
                if let quadcam_lib::trim::CutChange::Confirm { files } = &change {
                    bail!(
                        "these cuts were exported already: {}. Add --removed keep or --removed trash.",
                        files
                            .iter()
                            .map(|f| f.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                out["change"] = serde_json::to_value(change)?;
            }
            if export {
                out["exported"] = serde_json::to_value(core.library_export_cuts(&id)?)?;
            }
            if out.as_object().is_some_and(|o| o.is_empty()) {
                bail!("give ranges, --clear or --export");
            }
            out
        }
        LibCmd::Trash { ids } => serde_json::to_value(core.library_trash(&ids)?)?,
        LibCmd::Photos { ids, album } => serde_json::to_value(core.library_photos(&ids, album)?)?,
    })
}

fn profiles(core: &Core, cmd: ProfCmd) -> Result<Value> {
    Ok(match cmd {
        ProfCmd::List => {
            let (profiles, default) = core.profiles()?;
            json!({"profiles": profiles, "default_profile": default, "places": core.places()?})
        }
        ProfCmd::Save {
            name,
            aircraft,
            camera_make,
            camera_model,
            video_system,
            keywords,
            author,
            place,
            models,
            rename,
            default,
        } => {
            let mut fields = serde_json::Map::new();
            let mut put = |k: &str, v: Option<Value>| {
                if let Some(v) = v {
                    fields.insert(k.to_string(), v);
                }
            };
            put("aircraft", aircraft.map(Value::from));
            put("camera_make", camera_make.map(Value::from));
            put("camera_model", camera_model.map(Value::from));
            put("video_system", video_system.map(Value::from));
            put("keywords", keywords.map(|k| json!(csv(&k))));
            put("author", author.map(Value::from));
            put("place", place.map(Value::from));
            put("edgetx_models", models.map(|m| json!(csv(&m))));
            let p = core.profile_save(&name, &fields, rename.as_deref())?;
            if default {
                core.profile_default(&p.name)?;
            }
            let (_, d) = core.profiles()?;
            json!({"profile": p, "default_profile": d})
        }
        ProfCmd::Delete { name } => {
            json!({"deleted": core.profile_delete(&name)?, "default_profile": core.profiles()?.1})
        }
        ProfCmd::Default { name } => json!({"default_profile": core.profile_default(&name)?}),
    })
}

fn places(core: &Core, cmd: PlaceCmd) -> Result<Value> {
    Ok(match cmd {
        PlaceCmd::List => serde_json::to_value(core.places()?)?,
        PlaceCmd::Search {
            query,
            provider,
            limit,
        } => serde_json::to_value(core.place_search(&query, provider.as_deref(), Some(limit))?)?,
        PlaceCmd::Save {
            name,
            location,
            search,
            pick,
            provider,
            rename,
        } => {
            let (lat, lon, from) = match (location, search) {
                (Some(l), _) => {
                    let (a, b) = latlon(&l)?;
                    (Some(a), Some(b), None)
                }
                (None, Some(q)) => {
                    let hits = core.place_search(&q, provider.as_deref(), Some(pick.max(5)))?;
                    let hit = hits.get(pick.max(1) - 1).cloned().with_context(|| {
                        format!(
                            "the search for {q:?} found {} places; nothing to pick at {pick}",
                            hits.len()
                        )
                    })?;
                    (Some(hit.lat), Some(hit.lon), Some(hit))
                }
                (None, None) => (None, None, None),
            };
            let p = core.place_save(&name, lat, lon, rename.as_deref())?;
            json!({"place": p, "from_search": from})
        }
        PlaceCmd::Delete { name } => serde_json::to_value(core.place_delete(&name)?)?,
    })
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let json = std::env::args().any(|a| a == "--json");
            if json
                && !matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                )
            {
                println!(
                    "{}",
                    json!({"ok": false, "error": {"code": "usage", "exit": 2, "message": e.to_string().trim()}})
                );
                std::process::exit(2);
            }
            e.exit();
        }
    };
    if matches!(cli.cmd, Cmd::Mcp) {
        std::process::exit(match quadcam_lib::mcp::serve_stdio(cli.session.clone()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("quadcam-cli mcp: {e:#}");
                1
            }
        });
    }
    let json = cli.json;
    match run(cli) {
        Ok(v) => {
            if json {
                println!("{}", json!({"ok": true, "result": v}));
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let (exit, code) = code_for(&msg);
            if json {
                println!(
                    "{}",
                    json!({"ok": false, "error": {"code": code, "exit": exit, "message": msg}})
                );
            } else {
                eprintln!("quadcam-cli: {msg}");
            }
            std::process::exit(exit);
        }
    }
}
