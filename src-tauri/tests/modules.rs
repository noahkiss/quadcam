//! The module manager against a fixture server on 127.0.0.1: download, checksum match and
//! mismatch, install, run, update, remove, and ffmpeg found through the module or Homebrew.

mod common;

use common::server::*;
use quadcam_lib::media::{self, FfmpegSource, ToolPrefs, Tools};
use quadcam_lib::modules::install::Signing;
use quadcam_lib::modules::manifest::Manifest;
use std::collections::BTreeMap;
use std::time::Duration;

fn homebrew() -> Tools {
    let empty = tempfile::tempdir().unwrap();
    let none = manager(empty.path(), Manifest::default(), String::new());
    media::find_tools_with(
        &ToolPrefs {
            source: FfmpegSource::Homebrew,
            paths: BTreeMap::new(),
        },
        &none,
    )
    .expect("tests need ffmpeg and ffprobe (brew install ffmpeg)")
}

fn prefs(source: FfmpegSource) -> ToolPrefs {
    ToolPrefs {
        source,
        paths: BTreeMap::new(),
    }
}

#[test]
fn install_needs_confirm_and_downloads_nothing_without_it() {
    let server = Server::start();
    let d = tempfile::tempdir().unwrap();
    let zip = archive(&[("tool", "#!/bin/sh\necho hi\n".into())], false);
    let url = server.put("/tool.zip", zip.clone());
    let m = Manifest(BTreeMap::from([(
        "tool".to_string(),
        pin("1.0", &[("tool", "tool")], vec![(url, zip)]),
    )]));
    let mods = manager(d.path(), m, String::new());
    let e = mods.install("tool", false).unwrap_err().to_string();
    assert!(e.starts_with("Refused: "), "{e}");
    assert!(e.contains("Its license is MIT"), "{e}");
    assert!(
        server.hits().is_empty(),
        "no request before the person agrees"
    );
    assert!(mods.status("tool").unwrap().installed.is_none());
}

#[test]
fn install_run_and_remove() {
    let server = Server::start();
    let d = tempfile::tempdir().unwrap();
    let zip = archive(
        &[(
            "bin/tool",
            "#!/bin/sh\necho \"args:$*\"\necho \"cwd:$(pwd)\"\necho \"path:$PATH\"\n".into(),
        )],
        false,
    );
    let url = server.put("/tool-1.0.zip", zip.clone());
    let m = Manifest(BTreeMap::from([(
        "tool".to_string(),
        pin("1.0", &[("tool", "bin/tool")], vec![(url, zip)]),
    )]));
    let mods = manager(d.path(), m, String::new());

    let s = mods.install("tool", true).unwrap();
    let rec = s.installed.as_ref().unwrap();
    assert_eq!(rec.version, "1.0");
    assert_eq!(rec.license, "MIT");
    assert_eq!(rec.tools["tool"].signing, Signing::NotBinary);
    let folder = s.folder.clone().unwrap();
    assert_eq!(folder, d.path().join("support/modules/tool/1.0"));
    assert!(folder.join("installed.json").is_file());
    assert!(s.problem.is_none() && !s.update);
    assert_eq!(server.hits(), ["/tool-1.0.zip"]);

    // A second install reuses the checked download.
    mods.install("tool", true).unwrap();
    assert_eq!(server.hits().len(), 1);

    let out = mods
        .run("tool", &["a".into(), "b c".into()], Duration::from_secs(10))
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("args:a b c"), "{text}");
    assert!(
        text.contains(&format!(
            "cwd:{}",
            d.path().join("cache/modules/run").display()
        )) || text.contains("cache/modules/run"),
        "{text}"
    );
    assert!(
        text.contains("path:/usr/bin:/bin:/usr/sbin:/sbin"),
        "{text}"
    );

    // A changed file is refused before it runs.
    std::fs::write(folder.join("bin/tool"), "#!/bin/sh\necho changed\n").unwrap();
    let e = mods
        .run("tool", &[], Duration::from_secs(10))
        .unwrap_err()
        .to_string();
    assert_eq!(e, "tool was changed after install; reinstall it.");
    assert_eq!(
        mods.status("tool").unwrap().problem.as_deref(),
        Some("tool was changed after install; reinstall it.")
    );

    let s = mods.remove("tool").unwrap();
    assert!(s.installed.is_none());
    assert!(!d.path().join("support/modules/tool").exists());
    let e = mods.run("tool", &[], Duration::from_secs(10)).unwrap_err();
    assert!(e.to_string().contains("not installed"), "{e}");
}

#[test]
fn checksum_mismatch_deletes_the_download() {
    let server = Server::start();
    let d = tempfile::tempdir().unwrap();
    let zip = archive(&[("tool", "#!/bin/sh\n".into())], false);
    let url = server.put("/tool.zip", zip.clone());
    let m = Manifest(BTreeMap::from([(
        "tool".to_string(),
        pin("1.0", &[("tool", "tool")], vec![(url, zip.clone())]),
    )]));
    // The server now hands out other bytes than the pin's.
    server.put(
        "/tool.zip",
        archive(&[("tool", "#!/bin/sh\nevil\n".into())], false),
    );
    let mods = manager(d.path(), m, String::new());
    let e = mods.install("tool", true).unwrap_err().to_string();
    assert_eq!(e, "The download does not match the expected checksum.");
    assert!(mods.status("tool").unwrap().installed.is_none());
    let cached: Vec<_> = walk(&d.path().join("cache/modules"));
    assert!(
        cached.iter().all(|p| p.is_dir()),
        "no downloaded file is kept: {cached:?}"
    );
    assert!(!d.path().join("support/modules/tool/1.0").exists());
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        out.push(e.path());
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        }
    }
    out
}

#[test]
fn check_and_update() {
    let server = Server::start();
    let d = tempfile::tempdir().unwrap();
    let v1 = archive(&[("tool", "#!/bin/sh\necho one\n".into())], false);
    let url1 = server.put("/tool-1.zip", v1.clone());
    let m = Manifest(BTreeMap::from([(
        "tool".to_string(),
        pin("1.0", &[("tool", "tool")], vec![(url1, v1)]),
    )]));
    let newest_url = format!("{}/modules.json", server.base);
    let mods = manager(d.path(), m.clone(), newest_url);
    mods.install("tool", true).unwrap();

    // No modules.json yet: the check fails and changes nothing.
    assert!(mods.check().is_err());
    assert!(!mods.status("tool").unwrap().update);

    // A release publishes 2.0 as a tar.gz; another module and an off-host pin are ignored.
    let v2 = archive(&[("dir/tool", "#!/bin/sh\necho two\n".into())], true);
    let url2 = server.put("/tool-2.tar.gz", v2.clone());
    let mut remote = m.clone();
    remote.0.insert(
        "tool".into(),
        pin("2.0", &[("tool", "dir/tool")], vec![(url2, v2)]),
    );
    let mut stranger = pin(
        "9.0",
        &[("x", "x")],
        vec![("https://example.com/x.zip".into(), vec![1])],
    );
    stranger.hosts = vec!["example.com".into()];
    remote.0.insert("stranger".into(), stranger);
    server.put("/modules.json", serde_json::to_vec(&remote).unwrap());
    let list = mods.check().unwrap();
    assert_eq!(list.len(), 1, "only known modules");
    let s = &list[0];
    assert!(s.update);
    assert_eq!(s.newest.as_ref().unwrap().version, "2.0");
    assert_eq!(
        s.installed.as_ref().unwrap().version,
        "1.0",
        "a check installs nothing"
    );

    let s = mods.install("tool", true).unwrap();
    assert_eq!(s.installed.as_ref().unwrap().version, "2.0");
    assert!(!s.update);
    assert!(
        !d.path().join("support/modules/tool/1.0").exists(),
        "the old version goes"
    );
    let out = mods.run("tool", &[], Duration::from_secs(10)).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "two");
}

#[test]
fn ffmpeg_from_the_module_or_from_homebrew() {
    let brew = homebrew();
    let server = Server::start();
    let d = tempfile::tempdir().unwrap();
    let mods = ffmpeg_module(&server, d.path(), &brew);
    let root = d.path().join("support/modules/ffmpeg/1.0");

    let t = media::find_tools_with(&prefs(FfmpegSource::Module), &mods).unwrap();
    assert_eq!(t.ffmpeg, root.join("ffmpeg"));
    assert_eq!(t.ffprobe, root.join("ffprobe"));
    let v = std::process::Command::new(&t.ffmpeg)
        .arg("-version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&v.stdout).starts_with("ffmpeg version"));

    let t = media::find_tools_with(&prefs(FfmpegSource::Homebrew), &mods).unwrap();
    assert_eq!(
        t.ffmpeg, brew.ffmpeg,
        "the homebrew setting skips the module"
    );

    // A path set per tool wins over both.
    let mut p = prefs(FfmpegSource::Module);
    p.paths.insert("ffprobe".into(), brew.ffprobe.clone());
    let t = media::find_tools_with(&p, &mods).unwrap();
    assert_eq!(
        (t.ffmpeg, t.ffprobe),
        (root.join("ffmpeg"), brew.ffprobe.clone())
    );

    // A changed module refuses instead of falling back.
    std::fs::write(root.join("ffmpeg"), "#!/bin/sh\n").unwrap();
    let e = media::find_tools_with(&prefs(FfmpegSource::Module), &mods).unwrap_err();
    assert_eq!(
        e.to_string(),
        "ffmpeg was changed after install; reinstall it."
    );

    // Without the module QuadCam works as before: Homebrew.
    mods.remove("ffmpeg").unwrap();
    let t = media::find_tools_with(&prefs(FfmpegSource::Module), &mods).unwrap();
    assert_eq!((t.ffmpeg, t.ffprobe), (brew.ffmpeg, brew.ffprobe));
}

#[test]
fn settings_name_the_ffmpeg_source() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("settings.json");
    assert_eq!(ToolPrefs::from_settings(&f).source, FfmpegSource::Module);
    quadcam_lib::settings::set(
        &f,
        &serde_json::from_str(
            r#"{"ffmpeg_source":"homebrew","modules":{"esptool":"/opt/x/esptool"}}"#,
        )
        .unwrap(),
    )
    .unwrap();
    let p = ToolPrefs::from_settings(&f);
    assert_eq!(p.source, FfmpegSource::Homebrew);
    assert_eq!(
        p.paths["esptool"],
        std::path::PathBuf::from("/opt/x/esptool")
    );
    assert!(quadcam_lib::settings::set(
        &f,
        &serde_json::from_str(r#"{"ffmpeg_source":"static"}"#).unwrap()
    )
    .is_err());
    assert!(quadcam_lib::settings::set(
        &f,
        &serde_json::from_str(r#"{"modules":{"ffmpeg":"relative/ffmpeg"}}"#).unwrap()
    )
    .is_err());
}
