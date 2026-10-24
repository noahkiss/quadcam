//! The telemetry overlay on a synthetic library clip: a subtitle file from a matched radio
//! log or a goggles sidecar, the burned-in copy, and the folder rules.

use crate::common::*;
use chrono::NaiveDate;
use quadcam_lib::core::{Core, LogChoice, NoHooks, OverlayParams, OverlaySource};
use quadcam_lib::media;
use quadcam_lib::photos::Recorder;
use quadcam_lib::session::{Editor, PlanPatch};
use quadcam_lib::telemetry::{self, TelemetryField};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A core with one 7 s analog clip imported on 2026-09-28, and its id and file.
fn one_clip(work: &Path) -> (Core, String, PathBuf) {
    one_clip_with(work, None)
}

/// `one_clip`, with the core reading `settings` when given.
fn one_clip_with(work: &Path, settings: Option<PathBuf>) -> (Core, String, PathBuf) {
    let src = work.join("card");
    std::fs::create_dir_all(&src).unwrap();
    make_clip(&src.join("PICT0001.AVI"), 7, true);
    let mut core = Core::new(
        work.join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );
    if let Some(f) = settings {
        core = core.with_settings(f);
    }
    let mut d = core.defaults();
    d.output_dir = Some(work.join("lib"));
    d.encoder = media::Encoder::X264;
    std::fs::create_dir_all(work.join("lib")).unwrap();
    core.set_defaults(d);
    core.load(Some(&src)).unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    core.plan_dates(LogChoice::None, Some(day)).unwrap();
    core.patch(
        &[PlanPatch {
            id: 0,
            name: Some("loops".into()),
            date: Some(day),
            ..Default::default()
        }],
        Editor::User,
    )
    .unwrap();
    core.import(&Default::default()).unwrap();
    let item = core.library(&Default::default()).unwrap().clips[0].clone();
    (core, item.clip.id.clone(), item.file.clone())
}

/// A radio log of that day, armed for about the clip's length, at 10 rows a second.
fn write_log(dir: &Path) -> PathBuf {
    let logs = dir.join("LOGS");
    std::fs::create_dir_all(&logs).unwrap();
    let mut csv = String::from("Date,Time,1RSS(dB),RQly(%),RxBt(V),Rud,Ele,Thr,Ail,FM\n");
    for i in 0..70 {
        csv.push_str(&format!(
            "2026-09-28,10:00:{:04.1},-60,100,{:.2},0,0,{},0,{}\n",
            i as f64 / 10.0,
            4.2 - i as f64 * 0.005,
            if i < 68 { 400 } else { -1024 },
            if i < 68 { "ACRO" } else { "ACRO*" }
        ));
    }
    std::fs::write(logs.join("Whoop-2026-09-28-100000.csv"), csv).unwrap();
    dir.to_path_buf()
}

#[test]
fn overlay_from_the_radio_log_writes_subtitles_outside_the_library() {
    let work = tempfile::tempdir().unwrap();
    let (core, id, file) = one_clip(work.path());
    let bytes = std::fs::read(&file).unwrap();
    let logs = write_log(&work.path().join("radio"));
    let out = work.path().join("overlays");
    let p = OverlayParams {
        id: id.clone(),
        logs: Some(logs.clone()),
        out: Some(out.clone()),
        ..Default::default()
    };
    let r = core.library_overlay(&p).unwrap();
    assert_eq!(r.source, OverlaySource::Log);
    assert_eq!(r.from, "matched");
    for f in [
        TelemetryField::Battery,
        TelemetryField::Lq,
        TelemetryField::Rssi,
        TelemetryField::Throttle,
        TelemetryField::Mode,
        TelemetryField::Timer,
    ] {
        assert!(r.available.contains(&f), "{f:?} in {:?}", r.available);
    }
    assert!(r.cues > 0);
    assert_eq!(r.srt.parent().unwrap(), out);
    assert!(r
        .srt
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with(".telemetry.srt"));
    let text = std::fs::read_to_string(&r.srt).unwrap();
    assert!(text.contains("LQ 100%  RSSI -60dB"), "{text}");
    assert!(text.contains("BAT 4.20V"), "{text}");
    assert!(text.contains("THR 70%  ACRO  0:00"), "{text}");
    let cues = telemetry::parse_srt(&text);
    assert_eq!(cues.len(), r.cues);
    assert!(cues.iter().all(|c| c.end <= 7.1), "{cues:?}");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "the clip is not changed"
    );
    assert!(r.video.is_none());

    // A second run never overwrites: it adds a number.
    let again = core
        .library_overlay(&OverlayParams {
            fields: vec![TelemetryField::Lq, TelemetryField::Delay],
            ..p.clone()
        })
        .unwrap();
    assert_ne!(again.srt, r.srt);
    assert!(
        again.srt.to_string_lossy().ends_with(" 2.telemetry.srt"),
        "{:?}",
        again.srt
    );
    assert_eq!(again.fields, [TelemetryField::Lq, TelemetryField::Delay]);
    assert!(
        again.warnings.iter().any(|w| w.contains("no delay")),
        "{:?}",
        again.warnings
    );
    let text = std::fs::read_to_string(&again.srt).unwrap();
    assert!(!text.contains("BAT"), "{text}");

    // Never into the library folder.
    let e = core
        .library_overlay(&OverlayParams {
            out: Some(work.path().join("lib/overlays")),
            ..p.clone()
        })
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("inside the library folder"),
        "{e:#}"
    );
    assert!(!work.path().join("lib/overlays").exists());
    let e = core
        .library_overlay(&OverlayParams {
            id: "nosuchclip".into(),
            ..p
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("No clip nosuchclip"), "{e:#}");
}

#[test]
fn overlay_from_a_goggles_sidecar_and_the_fallback() {
    let work = tempfile::tempdir().unwrap();
    let (core, id, _) = one_clip(work.path());
    let srt = work.path().join("AvatarG0001.srt");
    let mut text = String::new();
    for i in 0..20 {
        text.push_str(&format!(
            "{}\n00:00:{:02},{:03} --> 00:00:{:02},{:03}\nSignal:4 CH:8 FlightTime:{} SBat:{:.1}V GBat:12.0V Delay:{}ms Bitrate:25.0Mbps Distance:{}m\n\n",
            i + 1,
            i * 300 / 1000,
            i * 300 % 1000,
            (i + 1) * 300 / 1000,
            (i + 1) * 300 % 1000,
            i * 300 / 1000,
            8.4 - i as f64 * 0.05,
            40 + i,
            i * 2
        ));
    }
    std::fs::write(&srt, text).unwrap();
    let out = work.path().join("overlays");

    // No log folder anywhere: the sidecar, with a warning.
    let r = core
        .library_overlay(&OverlayParams {
            id: id.clone(),
            srt: Some(srt.clone()),
            out: Some(out.clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.source, OverlaySource::Srt);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("used the .srt sidecar")),
        "{:?}",
        r.warnings
    );
    assert_eq!(r.unknown_keys, ["ch"]);
    assert_eq!(r.cues, 20);
    let text = std::fs::read_to_string(&r.srt).unwrap();
    assert!(text.contains("BAT 8.4V 4.20/c  GOG 12.0V"), "{text}");
    assert!(text.contains("SIG 4  40ms  25.0Mbps  0m"), "{text}");

    // `source: log` without logs refuses; `source: srt` without a sidecar refuses.
    let e = core
        .library_overlay(&OverlayParams {
            id: id.clone(),
            source: Some(OverlaySource::Log),
            out: Some(out.clone()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("No log folder"), "{e:#}");
    let e = core
        .library_overlay(&OverlayParams {
            id,
            source: Some(OverlaySource::Srt),
            out: Some(out),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("no kept .srt sidecar"), "{e:#}");
}

#[test]
fn overlay_burns_the_numbers_into_a_copy() {
    let work = tempfile::tempdir().unwrap();
    let (core, id, _) = one_clip(work.path());
    let logs = write_log(&work.path().join("radio"));
    let r = core
        .library_overlay(&OverlayParams {
            id,
            logs: Some(logs),
            out: Some(work.path().join("overlays")),
            burn: true,
            ..Default::default()
        })
        .unwrap();
    if !telemetry::can_burn(&tools()).unwrap() {
        assert!(r.video.is_none());
        assert!(
            r.warnings.iter().any(|w| w.contains("no subtitles filter")),
            "{:?}",
            r.warnings
        );
        return;
    }
    let video = r.video.expect("a burned-in copy");
    assert!(video.to_string_lossy().ends_with(".telemetry.mp4"));
    let p = media::probe(&tools(), &video).unwrap();
    assert!((p.duration - 7.0).abs() < 0.5, "{}", p.duration);
    assert_eq!(p.video_streams, 1);
    // No half-written file stays.
    let left: Vec<_> = std::fs::read_dir(video.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(left.is_empty());
}

/// The clip file's bytes, length and modification time.
fn fingerprint(file: &Path) -> (Vec<u8>, std::time::SystemTime) {
    let m = std::fs::metadata(file).unwrap();
    (std::fs::read(file).unwrap(), m.modified().unwrap())
}

#[test]
fn overlay_never_writes_the_clip_and_replaces_a_file_only_with_overwrite() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().unwrap();
    let (core, id, file) = one_clip(work.path());
    // A read-only clip: any open for writing would fail the run.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    let before = fingerprint(&file);
    let logs = write_log(&work.path().join("radio"));
    let out = work.path().join("overlays");
    let p = OverlayParams {
        id: id.clone(),
        logs: Some(logs),
        out: Some(out.clone()),
        burn: true,
        ..Default::default()
    };
    let first = core.library_overlay(&p).unwrap();
    assert_eq!(fingerprint(&file), before, "the clip is not changed");

    // A file in the way stays; the run takes the next name.
    std::fs::write(&first.srt, "mine").unwrap();
    let second = core.library_overlay(&p).unwrap();
    assert_eq!(std::fs::read_to_string(&first.srt).unwrap(), "mine");
    assert_ne!(second.srt, first.srt);
    if let (Some(a), Some(b)) = (&first.video, &second.video) {
        assert_ne!(a, b);
    }

    // With overwrite, the clip's own name is replaced.
    let third = core
        .library_overlay(&OverlayParams {
            overwrite: true,
            ..p.clone()
        })
        .unwrap();
    assert_eq!(third.srt, first.srt);
    assert!(std::fs::read_to_string(&third.srt)
        .unwrap()
        .contains("LQ 100%"));
    assert_eq!(third.video, first.video);
    assert_eq!(fingerprint(&file), before, "the clip is not changed");
    let left: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn overlay_refuses_a_folder_inside_the_library_by_any_path() {
    let work = tempfile::tempdir().unwrap();
    let (core, id, file) = one_clip(work.path());
    let before = fingerprint(&file);
    let logs = write_log(&work.path().join("radio"));
    let lib = work.path().join("lib");
    // A symlink outside the library that points into it.
    let link = work.path().join("elsewhere");
    std::os::unix::fs::symlink(&lib, &link).unwrap();
    for out in [lib.clone(), lib.join("a/b"), link.clone(), link.join("new")] {
        let e = core
            .library_overlay(&OverlayParams {
                id: id.clone(),
                logs: Some(logs.clone()),
                out: Some(out.clone()),
                ..Default::default()
            })
            .unwrap_err();
        assert!(
            format!("{e:#}").contains("inside the library folder"),
            "{out:?}: {e:#}"
        );
    }
    assert!(!lib.join("a").exists() && !lib.join("new").exists());
    let e = core
        .library_overlay(&OverlayParams {
            id,
            logs: Some(logs),
            out: Some("relative/folder".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(format!("{e:#}").contains("not an absolute"), "{e:#}");
    assert_eq!(fingerprint(&file), before);
}

#[test]
fn overlay_writes_to_the_overlay_dir_setting() {
    let work = tempfile::tempdir().unwrap();
    let settings = work.path().join("settings.json");
    let dir = work.path().join("from-settings");
    std::fs::write(
        &settings,
        serde_json::json!({"overlayDir": dir}).to_string(),
    )
    .unwrap();
    let (core, id, _) = one_clip_with(work.path(), Some(settings));
    let logs = write_log(&work.path().join("radio"));
    let r = core
        .library_overlay(&OverlayParams {
            id,
            logs: Some(logs),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.srt.parent().unwrap(), dir);
    let named = core.settings().unwrap().named;
    assert_eq!(named["overlay_dir"], serde_json::json!(dir));
}

#[test]
fn mcp_overlay_burns_only_with_confirm() {
    use quadcam_lib::mcp::{LocalBackend, Server};
    let work = tempfile::tempdir().unwrap();
    let (core, id, _) = one_clip(work.path());
    let logs = write_log(&work.path().join("radio"));
    let out = work.path().join("overlays");
    let mut s = Server::new(LocalBackend(Arc::new(core)));
    let args = serde_json::json!({
        "action": "overlay", "ids": [id], "log_dir": logs, "out": out, "burn": true,
    });
    let r = s.call_tool("quadcam_library_files", args.clone());
    assert_eq!(r["isError"], false, "{r}");
    assert!(r.to_string().contains("confirm=true"), "{r}");
    assert!(!out.exists(), "nothing is written without confirm");

    let mut plain = args.clone();
    plain["burn"] = false.into();
    let r = s.call_tool("quadcam_library_files", plain);
    assert_eq!(r["isError"], false, "{r}");
    assert!(r.to_string().contains(".telemetry.srt"), "{r}");
    assert!(!r.to_string().contains("Burned-in copy"), "{r}");

    let mut go = args;
    go["confirm"] = true.into();
    let r = s.call_tool("quadcam_library_files", go);
    assert_eq!(r["isError"], false, "{r}");
    if telemetry::can_burn(&tools()).unwrap() {
        assert!(r.to_string().contains("Burned-in copy"), "{r}");
    }
}
