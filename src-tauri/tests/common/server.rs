//! A fixture HTTP server on 127.0.0.1 and fixture module archives, for the module manager's
//! tests. Nothing here reaches the internet.

use quadcam_lib::modules::fetch::Curl;
use quadcam_lib::modules::manifest::{Asset, Manifest, Pin};
use quadcam_lib::modules::Modules;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Serves `files` (URL path to bytes) over plain HTTP on a free local port, and counts the
/// requests.
pub struct Server {
    pub base: String,
    pub files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    pub hits: Arc<Mutex<Vec<String>>>,
}

impl Server {
    pub fn start() -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let hits: Arc<Mutex<Vec<String>>> = Arc::default();
        let (f, h) = (files.clone(), hits.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut line = String::new();
                let mut r = BufReader::new(s.try_clone().unwrap());
                if r.read_line(&mut line).is_err() {
                    continue;
                }
                // Skip the headers.
                loop {
                    let mut x = String::new();
                    if r.read_line(&mut x).unwrap_or(0) == 0 || x == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                h.lock().unwrap().push(path.clone());
                let body = f.lock().unwrap().get(&path).cloned();
                let _ = match body {
                    Some(b) => s
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                b.len()
                            )
                            .as_bytes(),
                        )
                        .and_then(|_| s.write_all(&b)),
                    None => s.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                };
            }
        });
        Server { base, files, hits }
    }

    pub fn put(&self, path: &str, bytes: Vec<u8>) -> String {
        self.files.lock().unwrap().insert(path.to_string(), bytes);
        format!("{}{path}", self.base)
    }

    pub fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A zip (or tar.gz when `tar`) holding one executable script per `(name, body)`.
pub fn archive(scripts: &[(&str, String)], tar: bool) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (name, body) in scripts {
        let p = src.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = d.path().join(if tar { "a.tar.gz" } else { "a.zip" });
    let st = if tar {
        Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status()
    } else {
        Command::new("/usr/bin/ditto")
            .args(["-c", "-k"])
            .arg(&src)
            .arg(&out)
            .status()
    }
    .unwrap();
    assert!(st.success());
    std::fs::read(out).unwrap()
}

/// A script that runs `target` with the same arguments.
pub fn wrapper(target: &Path) -> String {
    format!("#!/bin/sh\nexec '{}' \"$@\"\n", target.display())
}

/// A pin for module `name` whose assets the server holds.
pub fn pin(version: &str, tools: &[(&str, &str)], assets: Vec<(String, Vec<u8>)>) -> Pin {
    Pin {
        title: "Fixture".into(),
        about: "A fixture module.".into(),
        version: version.into(),
        license: "MIT".into(),
        license_url: "https://example.invalid/license".into(),
        source: "https://example.invalid/source".into(),
        homepage: "https://example.invalid".into(),
        hosts: vec!["127.0.0.1".into()],
        tools: tools
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<BTreeMap<_, _>>(),
        assets: assets
            .into_iter()
            .map(|(url, bytes)| Asset {
                url,
                sha256: sha256(&bytes),
                size: bytes.len() as u64,
            })
            .collect(),
    }
}

/// A module manager in `dir` with this manifest, fetching through curl from the fixture
/// server only.
pub fn manager(dir: &Path, manifest: Manifest, newest_url: String) -> Modules {
    Modules::new(
        dir.join("support/modules"),
        dir.join("cache/modules"),
        manifest,
        newest_url,
        Arc::new(Curl {
            loopback_only: true,
        }),
    )
}

/// The ffmpeg module as a fixture: `ffmpeg` and `ffprobe` scripts that run the Homebrew
/// tools, in one zip, installed into `dir`. Returns the manager.
pub fn ffmpeg_module(server: &Server, dir: &Path, homebrew: &quadcam_lib::media::Tools) -> Modules {
    let zip = archive(
        &[
            ("ffmpeg", wrapper(&homebrew.ffmpeg)),
            ("ffprobe", wrapper(&homebrew.ffprobe)),
        ],
        false,
    );
    let url = server.put("/ffmpeg-1.0.zip", zip.clone());
    let m = Manifest(BTreeMap::from([(
        "ffmpeg".to_string(),
        pin(
            "1.0",
            &[("ffmpeg", "ffmpeg"), ("ffprobe", "ffprobe")],
            vec![(url, zip)],
        ),
    )]));
    let mods = manager(dir, m, format!("{}/modules.json", server.base));
    mods.install("ffmpeg", true).unwrap();
    mods
}
