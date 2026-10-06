//! Staging, conversion, verify, metadata, naming, log matching and recovery, on synthetic clips.

mod common;

use chrono::{NaiveDate, TimeZone, Utc};
use common::*;
use quadcam_lib::logs::{Badge, Tunables};
use quadcam_lib::media::{Encoded, Encoder, Format};
use quadcam_lib::naming::NamePlanner;
use quadcam_lib::pipeline::{self, Clip, ClipJob, ClipStatus, DateSource, ImportSettings, Outcome};
use std::path::Path;

struct Card {
    dir: tempfile::TempDir,
}

impl Card {
    fn root(&self) -> &Path {
        self.dir.path()
    }
}

/// A card folder like an analog DVR writes: `DCIM/` with a short clip, a longer clip without
/// audio, a half-written clip, a zero-byte file and Mac hidden files.
fn synthetic_card() -> Card {
    let dir = tempfile::tempdir().unwrap();
    let dcim = dir.path().join("DCIM");
    std::fs::create_dir_all(&dcim).unwrap();
    make_clip(&dcim.join("PICT0001.AVI"), 5, true);
    make_clip(&dcim.join("PICT0002.AVI"), 3, false);
    let long = dir.path().join("long.tmp");
    make_clip(&long, 4, true);
    truncate_copy(&long, &dcim.join("PICT0003.AVI"), 0.5);
    std::fs::remove_file(long).unwrap();
    std::fs::write(dcim.join("PICT0004.AVI"), b"").unwrap();
    std::fs::write(dcim.join("._PICT0001.AVI"), b"junk").unwrap();
    std::fs::create_dir_all(dir.path().join(".fseventsd")).unwrap();
    Card { dir }
}

fn load(card: &Card, work: &Path) -> Vec<Clip> {
    let t = tools();
    let staging = work.join("staging");
    let mut clips = pipeline::stage(card.root(), &staging, &mut |_, _, _, _| {}).unwrap();
    for c in clips.iter_mut() {
        pipeline::analyse(&t, c, &staging).unwrap();
    }
    clips
}

fn job(id: usize, name: &str) -> ClipJob {
    ClipJob {
        id,
        skip: false,
        date: "2026-09-30".into(),
        time: None,
        source: DateSource::Import,
        name: name.into(),
        note: String::new(),
        meta: Default::default(),
        extra: Vec::new(),
        parts: Vec::new(),
    }
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

#[test]
fn stages_and_classifies_the_card() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let names: Vec<&str> = clips.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "PICT0001.AVI",
            "PICT0002.AVI",
            "PICT0003.AVI",
            "PICT0004.AVI"
        ],
        "hidden files ignored, PICT order"
    );

    assert_eq!(clips[0].status, ClipStatus::Ok);
    assert_eq!(clips[0].probe.as_ref().unwrap().video_packets, 5 * FPS);
    assert!((clips[0].duration - 5.0).abs() < 0.05);
    assert!(clips[0].thumb.as_ref().unwrap().is_file());

    assert_eq!(clips[1].status, ClipStatus::Ok);
    assert_eq!(clips[1].probe.as_ref().unwrap().audio_streams, 0);

    // Check 6: the truncated clip is flagged incomplete and recovered.
    let c3 = &clips[2];
    assert_eq!(c3.status, ClipStatus::Incomplete, "{}", c3.detail);
    assert!(c3.recovered.as_ref().unwrap().is_file());
    let pk = c3.probe.as_ref().unwrap().video_packets;
    assert!(pk > 0 && pk < 4 * FPS, "recovered {pk} packets");
    assert!(
        c3.detail.starts_with("incomplete, recovered 0:0"),
        "{}",
        c3.detail
    );

    assert_eq!(clips[3].status, ClipStatus::Empty);
    assert!(clips.iter().all(|c| c.stage_error.is_none()));
}

/// Checks 2 and 3 for one format: frame counts, durations and metadata read back.
fn import_all(format: Format) {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let t = tools();
    let mut planner = NamePlanner::new();
    let st = settings(out.path(), format);
    let mut j0 = job(0, "Wake Up");
    j0.note = "two packs, windy".into();
    let jobs = [j0, job(1, "bench"), job(2, "")];
    let mut results = Vec::new();
    for j in &jobs {
        let r = pipeline::import_clip(&t, &clips[j.id], j, &st, &mut planner, &mut |_| {});
        assert_eq!(r.outcome, Outcome::Verified, "clip {} {:?}", j.id, r.error);
        results.push(r);
    }
    let ext = format.ext();
    let names: Vec<String> = results
        .iter()
        .map(|r| {
            r.output
                .as_ref()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(
        names,
        [
            format!("2026-09-30_wake_up.{ext}"),
            format!("2026-09-30_bench.{ext}"),
            format!("2026-09-30_flight.{ext}")
        ]
    );

    for (r, c) in results.iter().zip(&clips) {
        let o = r.output.as_ref().unwrap();
        let p = quadcam_lib::media::probe(&t, o).unwrap();
        let src = c.probe.as_ref().unwrap();
        assert_eq!(p.video_packets, src.video_packets, "{}", o.display());
        assert!(
            (p.duration - src.duration).abs() <= 0.1,
            "{} {} vs {}",
            o.display(),
            p.duration,
            src.duration
        );
        assert_eq!(p.audio_streams, src.audio_streams.min(1));
    }

    // Metadata through ffprobe and exiftool.
    let first = results[0].output.as_ref().unwrap();
    let tags = format_tags(first);
    assert_eq!(tags["title"], "Wake Up");
    assert_eq!(tags["comment"], "two packs, windy");
    assert_eq!(tags["date"], "2026-09-30");
    // A MOV carries the description only as a QuickTime key (see qtmeta).
    let desc = if tags["description"].is_null() {
        &tags["com.apple.quicktime.description"]
    } else {
        &tags["description"]
    };
    assert_eq!(desc, "DVR PICT0001.AVI; date source: import");
    assert_eq!(tags["com.apple.quicktime.title"], "Wake Up");
    assert!(tags["com.apple.quicktime.software"]
        .as_str()
        .unwrap()
        .starts_with("QuadCam "));
    assert!(tags["creation_time"]
        .as_str()
        .unwrap()
        .contains("2026-09-30T"));
    if let Some(x) = exif(first) {
        assert_eq!(x["Title"], "Wake Up");
        assert_eq!(x["Comment"], "two packs, windy");
        assert_eq!(x["Description"], "DVR PICT0001.AVI; date source: import");
        assert!(x["CreateDate"].as_str().unwrap().starts_with("2026:09:30"));
    }
    // Modified time equals creation time (local noon of the date).
    let want = pipeline::creation_time(
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        None,
        DateSource::Import,
    );
    let mtime: chrono::DateTime<Utc> = first.metadata().unwrap().modified().unwrap().into();
    assert_eq!(mtime.timestamp(), want.timestamp());

    // Check 6: the half-written clip verifies against its recovered length.
    let r = pipeline::import_clip(
        &t,
        &clips[2],
        &job(2, "cut short"),
        &st,
        &mut planner,
        &mut |_| {},
    );
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    // No temporary files left behind.
    assert!(std::fs::read_dir(out.path())
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().starts_with('.')));
}

#[test]
fn import_mp4() {
    import_all(Format::Mp4);
}

#[test]
fn import_mov() {
    import_all(Format::Mov);
}

#[test]
fn import_mp4_x264() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let mut st = settings(out.path(), Format::Mp4);
    st.encoder = Encoder::X264;
    let r = pipeline::import_clip(
        &tools(),
        &clips[0],
        &job(0, "soft"),
        &st,
        &mut NamePlanner::new(),
        &mut |_| {},
    );
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    assert_eq!(r.encoder, Some(Encoded::X264));
}

/// Check 5: duplicate names get -2, -3, in a batch and against files already there.
#[test]
fn duplicate_names() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    std::fs::write(out.path().join("2026-09-30_flight.mp4"), b"not mine").unwrap();
    let mut st = settings(out.path(), Format::Mp4);
    st.keep_originals = true;
    let mut planner = NamePlanner::new();
    let t = tools();
    let names: Vec<String> = [0, 1, 0]
        .iter()
        .map(|&i| {
            let r =
                pipeline::import_clip(&t, &clips[i], &job(i, ""), &st, &mut planner, &mut |_| {});
            assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
            let orig = r.original.unwrap();
            assert!(orig.starts_with(out.path().join("originals")));
            assert_eq!(orig.file_stem(), r.output.as_ref().unwrap().file_stem());
            r.output
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(
        names,
        [
            "2026-09-30_flight-2.mp4",
            "2026-09-30_flight-3.mp4",
            "2026-09-30_flight-4.mp4"
        ]
    );
    assert_eq!(
        std::fs::read(out.path().join("2026-09-30_flight.mp4")).unwrap(),
        b"not mine",
        "never overwrite"
    );
}

/// A failed verify deletes the output and keeps the source.
#[test]
fn failed_verify_removes_output() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut clips = load(&card, work.path());
    clips[0].probe.as_mut().unwrap().video_packets += 7; // pretend the source had more frames
    let r = pipeline::import_clip(
        &tools(),
        &clips[0],
        &job(0, "x"),
        &settings(out.path(), Format::Mp4),
        &mut NamePlanner::new(),
        &mut |_| {},
    );
    assert_eq!(r.outcome, Outcome::Failed);
    assert!(r.error.unwrap().contains("frame count"));
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
    assert!(clips[0].staged.as_ref().unwrap().is_file());
    assert!(clips[0].card_path.is_file());
}

/// Card pulled mid-copy: that file fails, the rest stage, format stays locked.
#[test]
fn unreadable_file_blocks_format() {
    use std::os::unix::fs::PermissionsExt;
    let card = synthetic_card();
    let bad = card.root().join("DCIM/PICT0002.AVI");
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
    let work = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(clips[1].stage_error.is_some());
    assert!(clips[0].staged.is_some());
    let results: Vec<_> = clips
        .iter()
        .map(|c| pipeline::ClipResult {
            id: c.id,
            outcome: Outcome::Verified,
            output: None,
            original: None,
            size: 0,
            error: None,
            encoder: None,
            meta: None,
            cuts: Vec::new(),
            qt: Vec::new(),
        })
        .collect();
    assert!(pipeline::can_format(&clips, &results).is_err());
}

#[test]
fn format_gate_needs_every_clip_verified_or_skipped() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let res = |o: &[Outcome]| -> Vec<pipeline::ClipResult> {
        o.iter()
            .enumerate()
            .map(|(id, &outcome)| pipeline::ClipResult {
                id,
                outcome,
                output: None,
                original: None,
                size: 0,
                error: None,
                encoder: None,
                meta: None,
                cuts: Vec::new(),
                qt: Vec::new(),
            })
            .collect()
    };
    use Outcome::*;
    assert!(pipeline::can_format(&clips, &[]).is_err());
    assert!(
        pipeline::can_format(&clips, &res(&[Verified, Verified, Verified])).is_err(),
        "one clip missing"
    );
    assert!(pipeline::can_format(&clips, &res(&[Verified, Failed, Verified, Skipped])).is_err());
    assert!(pipeline::can_format(&clips, &res(&[Verified, Verified, Verified, Skipped])).is_ok());
}

#[test]
fn output_folder_checks() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let refs: Vec<&Clip> = clips.iter().collect();
    let st = settings(Path::new("/nonexistent/quadcam-out"), Format::Mp4);
    assert!(pipeline::preflight(&st, &refs)
        .unwrap_err()
        .to_string()
        .contains("does not exist"));
    let out = tempfile::tempdir().unwrap();
    pipeline::preflight(&settings(out.path(), Format::Mp4), &refs).unwrap();
    assert!(pipeline::free_bytes(out.path()).unwrap() > 0);
}

/// Check 4: log matching end to end, from EdgeTX CSVs.
#[test]
fn radio_log_dates() {
    let logs = tempfile::tempdir().unwrap();
    std::fs::create_dir(logs.path().join("LOGS")).unwrap();
    // Two packs at 10:00-10:03 and 10:05-10:08, then one at 11:00-11:02.
    let mut csv = String::from("Date,Time,1RSS(dB),RQly(%)\n");
    let mut add = |h: u32, m0: u32, m1: u32| {
        let mut s = h * 3600 + m0 * 60;
        while s <= h * 3600 + m1 * 60 {
            csv.push_str(&format!(
                "2026-09-28,{:02}:{:02}:{:02}.000,-50,100\n",
                s / 3600,
                s / 60 % 60,
                s % 60
            ));
            s += 1;
        }
    };
    add(10, 0, 3);
    add(10, 5, 8);
    add(11, 0, 2);
    std::fs::write(logs.path().join("LOGS/Model01-2026-09-28-100000.csv"), csv).unwrap();
    std::fs::write(
        logs.path().join("LOGS/Model01-2000-01-01-000000.csv"),
        "Date,Time\n2000-01-01,00:00:01.000\n",
    )
    .unwrap();

    let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let tun = Tunables::default();
    // Clip durations: two-pack flight, bench-only clip, one pack with long pre-arm, spare.
    let plan = pipeline::plan_dates(
        &[500.0, 12.0, 360.0, 30.0].map(pipeline::DateInput::duration),
        Some(logs.path()),
        None,
        today,
        &tun,
    );
    assert_eq!(
        plan.day_used,
        Some(NaiveDate::from_ymd_opt(2026, 9, 28).unwrap())
    );
    let b: Vec<Badge> = plan.suggestions.iter().map(|s| s.badge).collect();
    assert_eq!(
        b,
        [
            Badge::Matched,
            Badge::Unmatched,
            Badge::Likely,
            Badge::Unmatched
        ]
    );
    assert_eq!(plan.suggestions[0].source, DateSource::Log);
    assert_eq!(plan.suggestions[0].date.to_string(), "2026-09-28");
    assert_eq!(plan.suggestions[0].time.unwrap().to_string(), "10:00:00");
    assert_eq!(plan.suggestions[0].segments, 2);
    assert_eq!(plan.suggestions[1].source, DateSource::Import);
    assert_eq!(plan.suggestions[1].date, today);

    // An RTC-reset day is refused with a warning.
    let plan = pipeline::plan_dates(
        &[pipeline::DateInput::duration(500.0)],
        Some(logs.path()),
        NaiveDate::from_ymd_opt(2000, 1, 1),
        today,
        &tun,
    );
    assert_eq!(plan.suggestions[0].source, DateSource::Import);
    assert!(plan.warnings[0].contains("clock may have reset"));

    // No logs at all: import date, no warning about dates.
    let plan = pipeline::plan_dates(
        &[pipeline::DateInput::duration(500.0)],
        None,
        None,
        today,
        &tun,
    );
    assert_eq!(plan.suggestions[0].date, today);

    // creation_time from a log is the segment start, converted from local time to UTC.
    let ct = pipeline::creation_time(
        plan_day(),
        Some(chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
        DateSource::Log,
    );
    let local = chrono::Local
        .from_local_datetime(&plan_day().and_hms_opt(10, 0, 0).unwrap())
        .unwrap();
    assert_eq!(ct, local.with_timezone(&Utc));
}

fn plan_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
}

/// With the add-time setting and a log time, the name is YYYY-MM-DD_HHMM_<name>.
#[test]
fn add_time_to_filename() {
    let card = synthetic_card();
    let work = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let mut st = settings(out.path(), Format::Mov);
    st.add_time = true;
    let mut j = job(0, "loops");
    j.source = DateSource::Log;
    j.time = Some("14:03:21".into());
    let r = pipeline::import_clip(
        &tools(),
        &clips[0],
        &j,
        &st,
        &mut NamePlanner::new(),
        &mut |_| {},
    );
    assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
    assert!(r.output.unwrap().ends_with("2026-09-30_1403_loops.mov"));
    // An import-date clip never gets a time, even with the setting on.
    let r = pipeline::import_clip(
        &tools(),
        &clips[1],
        &job(1, "bench"),
        &st,
        &mut NamePlanner::new(),
        &mut |_| {},
    );
    assert!(r.output.unwrap().ends_with("2026-09-30_bench.mov"));
}

/// Check 8: MP4 versus MOV size and time on a longer clip. Run with --ignored --nocapture.
#[test]
#[ignore]
fn size_and_speed() {
    let dir = tempfile::tempdir().unwrap();
    let dcim = dir.path().join("DCIM");
    std::fs::create_dir(&dcim).unwrap();
    let secs = std::env::var("QUADCAM_BENCH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    make_clip(&dcim.join("PICT0001.AVI"), secs, true);
    let card = Card { dir };
    let work = tempfile::tempdir().unwrap();
    let clips = load(&card, work.path());
    let src = clips[0].size;
    for format in [Format::Mp4, Format::Mov] {
        let out = tempfile::tempdir().unwrap();
        let t0 = std::time::Instant::now();
        let r = pipeline::import_clip(
            &tools(),
            &clips[0],
            &job(0, "bench"),
            &settings(out.path(), format),
            &mut NamePlanner::new(),
            &mut |_| {},
        );
        let el = t0.elapsed().as_secs_f64();
        assert_eq!(r.outcome, Outcome::Verified, "{:?}", r.error);
        println!(
            "{secs}s clip {:?}: source {:.1} MB -> {:.1} MB ({:.1}x smaller), {el:.2}s incl. verify ({:.0}x real time)",
            format,
            src as f64 / 1e6,
            r.size as f64 / 1e6,
            src as f64 / r.size as f64,
            secs as f64 / el
        );
    }
}

/// DVRs reuse file names (the Echo restarts at PICT0001 after a format). Two different
/// flights named PICT0001.AVI must never share a staged copy, thumbnail or preview.
#[test]
fn same_file_name_different_flights() {
    use quadcam_lib::core::{Core, NoHooks};
    use quadcam_lib::photos::Recorder;
    use std::sync::Arc;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    make_clip(&a.path().join("PICT0001.AVI"), 2, true);
    make_clip(&b.path().join("PICT0001.AVI"), 3, true);
    let work = tempfile::tempdir().unwrap();
    let core = Core::new(
        work.path().to_path_buf(),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    let sa = core.load(Some(a.path())).unwrap();
    let pa = core.preview(0).unwrap();
    let sb = core.load(Some(b.path())).unwrap();
    let pb = core.preview(0).unwrap();
    let (ta, tb) = (
        sa.clips[0].thumb.clone().unwrap(),
        sb.clips[0].thumb.clone().unwrap(),
    );
    assert_ne!(ta, tb, "thumbnails are keyed on content, not the file name");
    assert!(ta.is_file() && tb.is_file());
    assert_ne!(pa, pb, "previews are keyed on content");
    assert!(
        (pipeline_duration(&pb) - 3.0).abs() < 0.2,
        "the preview is the new flight"
    );

    // Same name, same size, different bytes: staging copies it again.
    let staging = work.path().join("same");
    let c = tempfile::tempdir().unwrap();
    let mut bytes = std::fs::read(a.path().join("PICT0001.AVI")).unwrap();
    pipeline::stage(a.path(), &staging, &mut |_, _, _, _| {}).unwrap();
    let n = bytes.len();
    bytes[n - 10] ^= 0xff;
    std::fs::write(c.path().join("PICT0001.AVI"), &bytes).unwrap();
    let clips = pipeline::stage(c.path(), &staging, &mut |_, _, _, _| {}).unwrap();
    assert_eq!(
        std::fs::read(clips[0].staged.as_ref().unwrap()).unwrap(),
        bytes
    );
}

fn pipeline_duration(p: &Path) -> f64 {
    quadcam_lib::media::probe(&tools(), p).unwrap().duration
}
