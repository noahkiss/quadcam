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
        places: Vec::new(),
        profiles: Vec::new(),
        default_profile: None,
        layout: quadcam_lib::library::Layout::Flat,
        place_folders: false,
        import_id: String::new(),
        name_date_format: Default::default(),
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
        meta: quadcam_lib::metadata::Resolved {
            location: Some(quadcam_lib::metadata::Location {
                lat: 40.68919,
                lon: -74.04449,
                name: None,
            }),
            make: "Maker".into(),
            keywords: vec!["FPV".into()],
            ..Default::default()
        },
        extra: Vec::new(),
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
    assert!(tags["com.apple.quicktime.description"]
        .as_str()
        .unwrap()
        .contains("cut 1.0-3.5 s"));
    // QuickTime metadata carries over to the cut, with the cut's own start time.
    assert_eq!(
        tags["com.apple.quicktime.location.ISO6709"],
        "+40.6892-074.0445/"
    );
    assert_eq!(tags["com.apple.quicktime.make"], "Maker");
    let main = format_tags(r.output.as_ref().unwrap());
    let when = |v: &serde_json::Value| {
        chrono::DateTime::parse_from_str(v.as_str().unwrap(), "%Y-%m-%dT%H:%M:%S%z").unwrap()
    };
    assert_eq!(
        (when(&tags["com.apple.quicktime.creationdate"])
            - when(&main["com.apple.quicktime.creationdate"]))
        .num_seconds(),
        1
    );
    if let Some(x) = exif(&file) {
        assert_eq!(x["Title"], "loops");
    }

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

/// A profile picked by the log's EdgeTX model fills the gear, author and keywords; a saved
/// place gives the location; flight numbers come from the log. All of it reads back.
#[test]
fn profile_place_and_flight_stats_through_the_core() {
    use quadcam_lib::core::{Core, LogChoice, NoHooks};
    use quadcam_lib::metadata::{Place, Profile};
    use quadcam_lib::photos::Recorder;
    use quadcam_lib::session::{Editor, PlanPatch};
    use std::sync::Arc;

    let logs = tempfile::tempdir().unwrap();
    std::fs::create_dir(logs.path().join("LOGS")).unwrap();
    let mut csv = String::from("Date,Time,1RSS(dB),RQly(%),RxBt(V),Rud,Ele,Thr,Ail\n");
    for i in 0..60 {
        // Rows every 0.1 s for 6 s; full aileron at 2.0-2.6 s.
        let ail = if (20..26).contains(&i) { 1024 } else { 0 };
        csv.push_str(&format!(
            "2026-09-28,10:00:{:04.1},{},{},{:.2},0,0,{},{ail}\n",
            i as f64 / 10.0,
            -60 - i % 7,
            100 - i % 5,
            4.2 - i as f64 * 0.01,
            if i > 40 { 1024 } else { 0 }
        ));
    }
    std::fs::write(logs.path().join("LOGS/Whoop-2026-09-28-100000.csv"), csv).unwrap();
    let src = tempfile::tempdir().unwrap();
    make_clip(&src.path().join("PICT0001.AVI"), 7, true);

    let work = tempfile::tempdir().unwrap();
    let core = Core::new(
        work.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let mut d = core.defaults();
    d.output_dir = Some(work.path().join("out"));
    std::fs::create_dir_all(work.path().join("out")).unwrap();
    d.places = vec![Place {
        name: "Field".into(),
        lat: 40.68919,
        lon: -74.04449,
    }];
    d.profiles = vec![
        Profile {
            name: "Whoop".into(),
            camera_make: "Maker".into(),
            camera_model: "Goggles".into(),
            aircraft: "65 mm whoop".into(),
            keywords: vec!["tinywhoop".into()],
            author: "Pilot".into(),
            edgetx_models: vec!["Whoop".into()],
            ..Default::default()
        },
        Profile {
            name: "Other".into(),
            camera_make: "Wrong".into(),
            ..Default::default()
        },
    ];
    d.default_profile = Some("Other".into());
    core.set_defaults(d);
    core.load(Some(src.path())).unwrap();
    let s = core
        .plan_dates(
            LogChoice::Dir(logs.path().to_path_buf()),
            NaiveDate::from_ymd_opt(2026, 9, 28),
        )
        .unwrap();
    let p = &s.plans[0];
    assert_eq!(p.log_model.as_deref(), Some("Whoop"));
    let f = p.flight.as_ref().unwrap();
    assert_eq!(
        (f.packs, f.min_rssi_db, f.min_lq),
        (1, Some(-66.0), Some(96.0))
    );

    // An unknown place is refused with the saved names; a known one sets the location.
    let place = |n: &str| PlanPatch {
        id: 0,
        place: Some(n.into()),
        keywords: Some(vec!["park".into()]),
        ..Default::default()
    };
    let e = core.patch(&[place("Nowhere")], Editor::User).unwrap_err();
    assert!(format!("{e:#}").contains("Field"));
    core.patch(&[place("field")], Editor::User).unwrap();

    let out = core.import(&Default::default()).unwrap();
    let r = &out.summary.results[0];
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    let tags = format_tags(r.output.as_ref().unwrap());
    let q = |k: &str| tags[format!("com.apple.quicktime.{k}")].clone();
    assert_eq!(
        q("make"),
        "Maker",
        "the log's model picks the profile, not the default"
    );
    assert_eq!(q("model"), "Goggles");
    assert_eq!(q("author"), "Pilot");
    assert_eq!(q("location.ISO6709"), "+40.6892-074.0445/");
    assert_eq!(q("keywords"), "FPV,tinywhoop,park,roll");
    assert_eq!(tags["app.quadcam.aircraft"], "65 mm whoop");
    let flight = tags["app.quadcam.flight"].as_str().unwrap();
    assert!(
        flight.contains("min RxBt 3.61 V") && flight.contains("max throttle 100%"),
        "{flight}"
    );
    assert!(core.verify(None).unwrap()[0].ok);
}

/// Prints the dead air and keep ranges of real clips, for checking the thresholds against
/// footage that cannot live in the repo:
/// `QUADCAM_REAL_CLIPS="a.AVI:b.AVI" cargo test --test moments real_clips -- --ignored --nocapture`
#[test]
#[ignore]
fn real_clips() {
    let t = tools();
    let list = std::env::var("QUADCAM_REAL_CLIPS").expect("set QUADCAM_REAL_CLIPS");
    for f in list.split(':').filter(|f| !f.is_empty()) {
        let p = media::probe(&t, Path::new(f)).unwrap();
        let started = std::time::Instant::now();
        let scan = moments::scan_signal(&t, Path::new(f), p.fps, p.duration).unwrap();
        println!(
            "{f}: {:.1} s, {} samples in {:.1} s",
            p.duration,
            scan.samples,
            started.elapsed().as_secs_f64()
        );
        for m in &scan.dead_air {
            println!(
                "  dead {:7.1} - {:7.1}  {:.2}  {}",
                m.start, m.end, m.score, m.detail
            );
        }
        for k in &scan.keep {
            println!("  keep {:7.1} - {:7.1}", k.start, k.end);
        }
    }
}

/// Regression on real footage: feature vectors measured on Echo DVR clips (numbers only,
/// in tests/fixtures/echo_frames.json), with the class a person saw in each frame.
#[test]
fn classifies_real_echo_frames() {
    use quadcam_lib::moments::{classify, FrameStats, Signal};
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/echo_frames.json")).unwrap();
    let mut wrong = Vec::new();
    for f in doc["frames"].as_array().unwrap() {
        let n = |k: &str| f[k].as_f64().unwrap();
        let s = FrameStats {
            mean_y: n("mean_y"),
            sd_y: n("sd_y"),
            sat: n("sat"),
            blue: n("blue"),
            h_detail: n("h_detail"),
            v_detail: n("v_detail"),
            change: Some(n("change")),
        };
        let want = match f["expect"].as_str().unwrap() {
            "live" => Signal::Live,
            "blue" => Signal::Blue,
            "static" => Signal::Static,
            "mono" => Signal::Mono,
            other => panic!("unknown class {other}"),
        };
        if classify(&s) != want {
            wrong.push(format!("{f} -> {:?}", classify(&s)));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// DVRs drop frames, so a frame's position is not its time. A clip with 3 s of frames
/// missing must still put the end of its blue screen at 5 s.
#[test]
fn dead_air_times_survive_dropped_frames() {
    let d = tempfile::tempdir().unwrap();
    let clip = d.path().join("PICT0001.AVI");
    let st = Command::new(tools().ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-t", "5", "-i"])
        .arg(format!("color=c=0x0000FF:s=720x480:r={FPS}"))
        .args(["-f", "lavfi", "-t", "10", "-i"])
        .arg(format!("testsrc=s=720x480:r={FPS}"))
        .args([
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0,select='not(between(t\\,1\\,4))',format=yuvj420p",
            "-fps_mode",
            "passthrough",
            "-c:v",
            "mjpeg",
            "-q:v",
            "5",
            "-f",
            "avi",
        ])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(st.success());
    let t = tools();
    let p = media::probe(&t, &clip).unwrap();
    let scan = moments::scan_signal(&t, &clip, p.fps, p.duration).unwrap();
    assert_eq!(scan.dead_air.len(), 1, "{scan:?}");
    assert!((scan.dead_air[0].end - 5.0).abs() <= 0.1, "{scan:?}");
    assert_eq!(scan.keep.len(), 1);
    assert!((scan.keep[0].start - 5.0).abs() <= 0.1);
}
