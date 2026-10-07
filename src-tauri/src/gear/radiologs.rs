//! The radio log store (design 7.1, rule 5): each EdgeTX log (`LOGS/*.csv`) is kept once
//! per radio in `<gear>/logs/<radio>/`, under its own name, as a plain CSV file. Logs are
//! flight data, not snapshot content: no snapshot names them and they are never pruned.
//! The flight index reads the folder like any log folder.
//!
//! - A new log is copied in.
//! - A log that grew (its bytes start with the stored bytes) replaces the stored one.
//! - A log the store already holds in full, or a shorter copy of it, changes nothing.
//! - A log that changed any other way is kept as a second file, `<stem> (2).csv`.
//!
//! Every write goes to a temporary name and is renamed into place. The stored file gets
//! the source's modified time, so the next look skips a log whose size and time match
//! without reading it (a radio over USB reads about 0.5 MB/s).

use super::store::Store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What happened to one log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LogOutcome {
    /// A log the store did not have.
    Added,
    /// The stored log grew; the longer one replaced it.
    Grown,
    /// The store already holds these bytes (or more of the same log).
    Same,
    /// The log changed in a way that is not growth; kept as a second file.
    KeptBoth,
    /// Its size and time match the stored copy: not read.
    Unchanged,
}

/// One stored log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StoredLog {
    pub name: String,
    pub size: u64,
}

/// The logs of a gear folder.
#[derive(Debug, Clone)]
pub struct LogStore {
    store: Store,
}

/// True for a log file name: a plain `.csv` name, not hidden, no folder part.
pub fn is_log_name(name: &str) -> bool {
    !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && name.to_ascii_lowercase().ends_with(".csv")
}

impl LogStore {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// `<gear>/logs/<radio>/`.
    pub fn dir(&self, radio: &str) -> PathBuf {
        self.store.logs_dir(radio)
    }

    /// Takes one log file into the radio's folder.
    pub fn put_file(&self, radio: &str, src: &Path) -> Result<(String, LogOutcome)> {
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| is_log_name(n))
            .with_context(|| format!("{} is not a log file", src.display()))?
            .to_string();
        let meta = std::fs::metadata(src).with_context(|| format!("reading {}", src.display()))?;
        let dest = self.dir(radio).join(&name);
        if let Ok(m) = std::fs::metadata(&dest) {
            if m.len() == meta.len() && m.modified().ok() == meta.modified().ok() {
                return Ok((name, LogOutcome::Unchanged));
            }
        }
        let bytes = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
        let outcome = self.put(radio, &name, &bytes, meta.modified().ok())?;
        Ok((name, outcome))
    }

    /// Takes one log's bytes. `mtime` becomes the stored file's modified time.
    pub fn put(
        &self,
        radio: &str,
        name: &str,
        bytes: &[u8],
        mtime: Option<std::time::SystemTime>,
    ) -> Result<LogOutcome> {
        anyhow::ensure!(is_log_name(name), "{name:?} is not a log file name");
        let dir = self.dir(radio);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let (stem, ext) = split(name);
        // The log and its second copies, in order: the first that this one matches wins.
        let mut n = 1;
        loop {
            let candidate = if n == 1 {
                name.to_string()
            } else {
                format!("{stem} ({n}).{ext}")
            };
            let path = dir.join(&candidate);
            let stored = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    write(&path, bytes, mtime)?;
                    return Ok(if n == 1 {
                        LogOutcome::Added
                    } else {
                        LogOutcome::KeptBoth
                    });
                }
                Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
            };
            if stored == bytes || stored.starts_with(bytes) {
                return Ok(LogOutcome::Same);
            }
            if bytes.starts_with(&stored) {
                write(&path, bytes, mtime)?;
                return Ok(LogOutcome::Grown);
            }
            n += 1;
        }
    }

    /// The radio's stored logs, by name.
    pub fn list(&self, radio: &str) -> Vec<StoredLog> {
        let mut out: Vec<StoredLog> = std::fs::read_dir(self.dir(radio))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                let m = e.metadata().ok()?;
                (m.is_file() && is_log_name(&name)).then_some(StoredLog {
                    name,
                    size: m.len(),
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// The radios with a log folder.
    pub fn radios(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(self.store.root().join("logs"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        out.sort();
        out
    }
}

fn split(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or((name, "csv"))
}

fn write(path: &Path, bytes: &[u8], mtime: Option<std::time::SystemTime>) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("csv.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    if let Some(t) = mtime {
        f.set_modified(t)?;
    }
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grown_replaces_changed_keeps_both() {
        let d = tempfile::tempdir().unwrap();
        let s = LogStore::new(Store::new(d.path()));
        let name = "ALPHA-2026-05-01.csv";
        assert_eq!(
            s.put("r", name, b"h\n1\n", None).unwrap(),
            LogOutcome::Added
        );
        assert_eq!(s.put("r", name, b"h\n1\n", None).unwrap(), LogOutcome::Same);
        assert_eq!(
            s.put("r", name, b"h\n1\n2\n", None).unwrap(),
            LogOutcome::Grown
        );
        assert_eq!(std::fs::read(s.dir("r").join(name)).unwrap(), b"h\n1\n2\n");
        assert_eq!(
            s.put("r", name, b"h\n1\n", None).unwrap(),
            LogOutcome::Same,
            "an older copy"
        );
        assert_eq!(
            s.put("r", name, b"other\n", None).unwrap(),
            LogOutcome::KeptBoth
        );
        let second = s.dir("r").join("ALPHA-2026-05-01 (2).csv");
        assert_eq!(std::fs::read(&second).unwrap(), b"other\n");
        assert_eq!(
            s.put("r", name, b"other\nmore\n", None).unwrap(),
            LogOutcome::Grown
        );
        assert_eq!(std::fs::read(&second).unwrap(), b"other\nmore\n");
        assert_eq!(s.list("r").len(), 2);
        assert_eq!(s.radios(), vec!["r".to_string()]);
    }

    #[test]
    fn unchanged_size_and_time_is_not_read() {
        let d = tempfile::tempdir().unwrap();
        let s = LogStore::new(Store::new(d.path().join("gear")));
        let src = d.path().join("BRAVO-2026-05-02.csv");
        std::fs::write(&src, "a,b\n1,2\n").unwrap();
        assert_eq!(s.put_file("r", &src).unwrap().1, LogOutcome::Added);
        assert_eq!(s.put_file("r", &src).unwrap().1, LogOutcome::Unchanged);
        assert!(s.put_file("r", &d.path().join("x.txt")).is_err());
        assert!(!is_log_name(".hidden.csv"));
    }
}
