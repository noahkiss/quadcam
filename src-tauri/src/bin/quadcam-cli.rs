//! quadcam-cli: everything the GUI does, for scripts and coding agents. Every command takes
//! `--json`. It drives the same `core::Core` as the app, on a session file, so separate runs
//! continue one session. `quadcam-cli mcp` starts the MCP server on stdio.

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use quadcam_lib::core::{Core, FormatRequest, ImportOptions, LogChoice};
use quadcam_lib::media::{self, Encoder, Format};
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
    },
    /// Show the current session: clips, plans, results.
    Show,
    /// Convert and verify every non-skipped clip.
    Import {
        /// JSON plan: {"clips":[{"id":0,"name":"..","date":"YYYY-MM-DD","note":"..","skip":false}]}
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
        /// Skip a clip. Repeatable.
        #[arg(long = "skip", value_name = "ID")]
        skip: Vec<usize>,
        /// Import a clip that was skipped. Repeatable.
        #[arg(long = "unskip", value_name = "ID")]
        unskip: Vec<usize>,
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
    /// Run the MCP server on stdio.
    Mcp,
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
            if !patches.is_empty() {
                s = core.patch(&patches, Editor::User)?;
            }
            json!({"log_dir": s.log_dir, "log_day": s.log_day, "log_days": s.log_days, "warnings": s.date_warnings, "plans": s.plans})
        }
        Cmd::Show => serde_json::to_value(
            core.session()
                .ok_or_else(|| anyhow!("No clips loaded. Run `stage` first."))?,
        )?,
        Cmd::Import {
            plan,
            names,
            notes,
            dates,
            skip,
            unskip,
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
        Cmd::Mcp => unreachable!(),
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
