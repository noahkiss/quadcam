//! Moments and cuts on synthetic clips: dead air from real ffmpeg frames, stick moments
//! through log matching, and trimmed exports.

mod common;

use chrono::NaiveDate;
use common::*;
use quadcam_lib::logs::Tunables;
use quadcam_lib::media::{self, Encoder, Format};
use quadcam_lib::moments::{self, MomentKind, Span};
use quadcam_lib::naming::NamePlanner;
use quadcam_lib::pipeline::{self, ClipJob, DateSource, ImportSettings, Outcome};
use std::path::Path;
use std::process::Command;

/// An MJPEG AVI made of lavfi sources played one after the other, each `(graph, seconds)`.
/// `{}` in a graph is replaced with the DVR frame size and rate.
fn make_sequence(path: &Path, parts: &[(&str, u32)]) {
    let mut c = Command::new(tools().ffmpeg);
    c.args(["-v", "error", "-y"]);
    for (graph, secs) in parts {
        c.args(["-f", "lavfi", "-t", &secs.to_string(), "-i"])
            .arg(graph.replace("{}", &format!("s=720x480:r={FPS}")));
    }
    let inputs: String = (0..parts.len()).map(|i| format!("[{i}:v]")).collect();
    c.args([
        "-filter_complex",
        &format!("{inputs}concat=n={}:v=1:a=0,format=yuvj420p", parts.len()),
        "-c:v",
        "mjpeg",
        "-q:v",
        "5",
        "-f",
        "avi",
    ])
    .arg(path);
    assert!(c.status().unwrap().success(), "ffmpeg sequence");
}

#[test]
fn dead_air_from_real_frames() {
    let d = tempfile::tempdir().unwrap();
    let clip = d.path().join("PICT0001.AVI");
    let snow = "color=c=gray:{},noise=alls=100:allf=t+u";
    // 5 s blue screen, 10 s picture, a 2 s static breakup (stays in), 5 s picture,
    // 4 s colour bars, 4 s static, 3 s black.
    make_sequence(
        &clip,
        &[
            ("color=c=0x0000FF:{}", 5),
            ("testsrc={}", 10),
            (snow, 2),
            ("testsrc={}", 5),
            ("smptebars={}", 4),
            (snow, 4),
            ("color=c=black:{}", 3),
        ],
    );
    let t = tools();
    let p = media::probe(&t, &clip).unwrap();
    assert!((p.duration - 33.0).abs() < 0.1, "{}", p.duration);
    let started = std::time::Instant::now();
    let scan = moments::scan_signal(&t, &clip, p.fps, p.duration).unwrap();
    assert!(started.elapsed().as_secs() < 10);
    assert!((scan.step - 0.5).abs() < 1e-9);
    let dead: Vec<(f64, f64, &str)> = scan
        .dead_air
        .iter()
        .map(|m| (m.start, m.end, m.detail.as_str()))
        .collect();
    assert_eq!(dead.len(), 2, "{dead:?}");
    assert_eq!((dead[0].0, dead[0].1), (0.0, 5.0));
    assert!(dead[0].2.starts_with("blue"), "{dead:?}");
    // Bars, static and black run together into one stretch at the end.
    assert_eq!((dead[1].0, dead[1].1), (22.0, 33.0));
    for k in ["test pattern", "static", "black"] {
        assert!(dead[1].2.contains(k), "{dead:?}");
    }
    assert_eq!(
        scan.keep,
        vec![Span {
            start: 5.0,
            end: 22.0
        }]
    );
    assert!(scan.dead_air.iter().all(|m| m.kind == MomentKind::DeadAir));
}

fn settings(out: &Path, format: Format) -> ImportSettings {
    ImportSettings {
        output_dir: out.to_path_buf(),
        format,
        encoder: Encoder::Videotoolbox,
        keep_originals: false,
        add_time: false,
        default_name: "flight".into(),
    }
}

fn cut_clip(format: Format) {
    let d = tempfile::tempdir().unwrap();
    let card = d.path().join("card");
    std::fs::create_dir(&card).unwrap();
    make_clip(&card.join("PICT0001.AVI"), 6, true);
    let t = tools();
    let mut clips =
        pipeline::stage(&card, &d.path().join("staging"), &mut |_, _, _, _| {}).unwrap();
    pipeline::analyse(&t, &mut clips[0], &d.path().join("thumbs")).unwrap();
    let clip = &clips[0];
    // A clip with a picture throughout has no dead air and no keep suggestion.
    let scan = clip.signal.as_ref().unwrap();
    assert!(scan.dead_air.is_empty() && scan.keep.is_empty(), "{scan:?}");

    let out = d.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let st = settings(&out, format);
    let job = ClipJob {
        id: 0,
        skip: false,
        date: "2026-09-30".into(),
        time: None,
        source: DateSource::Import,
        name: "loops".into(),
        note: String::new(),
    };
    let mut planner = NamePlanner::new();
    let mut r = pipeline::import_clip(&t, clip, &job, &st, &mut planner, &mut |_| {});
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    let ext = format.ext();
    let cuts = [Span {
        start: 1.0,
        end: 3.5,
    }];
    r.cuts = pipeline::export_cuts(&t, clip, &r, &cuts, &st, &mut planner);
    let c = &r.cuts[0];
    assert_eq!(c.outcome, Outcome::Verified, "{:?}", c.error);
    let file = c.output.clone().unwrap();
    assert_eq!(
        file.file_name().unwrap().to_string_lossy(),
        format!("2026-09-30_loops_cut1.{ext}")
    );
    let p = media::probe(&t, &file).unwrap();
    assert_eq!(p.video_packets, 75, "2.5 s at 30 fps, frame-exact");
    assert!((p.duration - 2.5).abs() < 0.05, "{}", p.duration);
    assert_eq!(p.audio_streams, 1);
    let tags = format_tags(&file);
    assert_eq!(tags["title"], "loops");
    assert!(tags["description"]
        .as_str()
        .unwrap()
        .contains("cut 1.0-3.5 s"));

    // A second run keeps the verified cut and writes only the new one.
    let mtime = file.metadata().unwrap().modified().unwrap();
    let cuts = [
        cuts[0],
        Span {
            start: 4.0,
            end: 6.0,
        },
    ];
    let mut planner = NamePlanner::new();
    r.cuts = pipeline::export_cuts(&t, clip, &r, &cuts, &st, &mut planner);
    assert_eq!(r.cuts[0].output.as_deref(), Some(file.as_path()));
    assert_eq!(file.metadata().unwrap().modified().unwrap(), mtime);
    assert_eq!(
        r.cuts[1].outcome,
        Outcome::Verified,
        "{:?}",
        r.cuts[1].error
    );
    assert!(r.cuts[1]
        .output
        .as_ref()
        .unwrap()
        .ends_with(format!("2026-09-30_loops_cut2.{ext}")));
    let leftovers: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn cut_mp4() {
    cut_clip(Format::Mp4);
}

#[test]
fn cut_mov() {
    cut_clip(Format::Mov);
}

/// Stick moments come through log matching, timed from the clip's first armed row.
#[test]
fn log_moments_through_date_planning() {
    let logs = tempfile::tempdir().unwrap();
    std::fs::create_dir(logs.path().join("LOGS")).unwrap();
    let mut csv = String::from("Date,Time,RQly(%),FM,Rud,Ele,Thr,Ail\n");
    // 60 s armed at 0.1 s rows from 10:00:00: hover, full aileron at 20.0-20.6 s.
    for i in 0..600 {
        let t = i as f64 / 10.0;
        let ail = if (200..206).contains(&i) { 1024 } else { 0 };
        csv.push_str(&format!(
            "2026-09-28,10:{:02}:{:06.3},100,\"ACRO\",0,0,0,{ail}\n",
            (t as u32) / 60,
            t % 60.0
        ));
    }
    std::fs::write(logs.path().join("LOGS/Model01-2026-09-28-100000.csv"), csv).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let plan = pipeline::plan_dates(
        &[75.0],
        Some(logs.path()),
        None,
        today,
        &Tunables::default(),
    );
    let s = &plan.suggestions[0];
    assert_eq!(s.source, DateSource::Log);
    assert_eq!(s.log_interval_s, Some(0.1));
    let roll = s
        .moments
        .iter()
        .find(|m| m.kind == MomentKind::Roll)
        .unwrap_or_else(|| panic!("{:?}", s.moments));
    assert!((roll.start - 20.0).abs() < 0.01, "{roll:?}");
    assert!((roll.end - 20.6).abs() < 0.01, "{roll:?}");
}
