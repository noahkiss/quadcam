//! The gear folder (`gearDir`, default `<support>/gear`) and the one writer of it.
//!
//! `gear.json` holds the person's gear data: devices now, packs, voice and sim choices as
//! later packages add them. It follows the `settings.json` rules: every write reads the
//! file fresh under an exclusive lock on `gear.json.lock`, changes only what it was asked
//! to, keeps every key it does not know, and renames a temp file into place. Nobody keeps
//! a full copy and writes it back, so a CLI write while the app runs is never lost.
//!
//! The folder layout (design 4.1) is named here too, so every module asks the store for
//! its paths: `snapshots/<device>/`, `blobs/<xx>/<hash>`, `logs/<radio>/`, `blackbox/<device>/`, `changes/`,
//! `voices/`, `flights.json`.

use super::model::{Device, DeviceKind};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `gear.json`'s keys and values.
pub type Values = Map<String, Value>;

/// The file name of the gear data, inside the gear folder.
pub const GEAR_FILE: &str = "gear.json";

/// `gear.json` key: the list of devices.
pub const DEVICES: &str = "devices";

/// A gear folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<gear>/gear.json`.
    pub fn gear_file(&self) -> PathBuf {
        self.root.join(GEAR_FILE)
    }

    /// `<gear>/snapshots/<device>/`: one manifest per backup.
    pub fn snapshots_dir(&self, device: &str) -> PathBuf {
        self.root.join("snapshots").join(safe(device))
    }

    /// `<gear>/blobs/`: file content by hash.
    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    /// `<gear>/blobs/<xx>/<hash>`.
    pub fn blob_path(&self, hash: &str) -> PathBuf {
        let h = safe(hash);
        let fan = h.get(..2).unwrap_or("00").to_string();
        self.blobs_dir().join(fan).join(h)
    }

    /// `<gear>/logs/<radio>/`: each radio log, stored once.
    pub fn logs_dir(&self, radio: &str) -> PathBuf {
        self.root.join("logs").join(safe(radio))
    }

    /// `<gear>/blackbox/<device>/`: one record per blackbox pull (`blackbox`).
    pub fn blackbox_dir(&self, device: &str) -> PathBuf {
        self.root.join("blackbox").join(safe(device))
    }

    /// `<gear>/changes/`: staged changes and the bench history.
    pub fn changes_dir(&self) -> PathBuf {
        self.root.join("changes")
    }

    /// `<gear>/voices/`: installed voice packs.
    pub fn voices_dir(&self) -> PathBuf {
        self.root.join("voices")
    }

    /// `<gear>/flights.json`: the rebuildable flight index.
    pub fn flights_file(&self) -> PathBuf {
        self.root.join("flights.json")
    }

    /// `gear.json`'s values. A missing file is empty. A file that does not parse is an
    /// error, so a write never replaces data it could not read.
    pub fn read(&self) -> Result<Values> {
        read(&self.gear_file())
    }

    /// Reads `gear.json` fresh, lets `edit` change the values, and writes them back when
    /// they changed, all under the lock. Returns what `edit` returned and the values as
    /// written.
    pub fn update<T>(&self, edit: impl FnOnce(&mut Values) -> Result<T>) -> Result<(T, Values)> {
        update(&self.gear_file(), edit)
    }

    /// A list under `key`, or empty. An entry that does not parse is skipped, so one bad
    /// entry never hides the rest.
    pub fn list<T: DeserializeOwned>(&self, key: &str) -> Result<Vec<T>> {
        Ok(list(&self.read()?, key))
    }

    /// A cheap fingerprint of `gear.json` (modified time and length), None when there is no
    /// file. The app polls it to see writes from the CLI or an MCP server.
    pub fn stamp(&self) -> Option<(std::time::SystemTime, u64)> {
        let m = std::fs::metadata(self.gear_file()).ok()?;
        Some((m.modified().ok()?, m.len()))
    }

    /// The saved devices.
    pub fn devices(&self) -> Result<Vec<Device>> {
        self.list(DEVICES)
    }

    /// The saved device with this id.
    pub fn device(&self, id: &str) -> Result<Option<Device>> {
        Ok(self.devices()?.into_iter().find(|d| d.id == id))
    }

    /// Adds a device or replaces the one with its id. Other devices and every other key
    /// stay as they are, unknown fields of the entries included.
    pub fn save_device(&self, device: &Device) -> Result<Device> {
        let entry = serde_json::to_value(device)?;
        self.update(|v| {
            let list = devices_mut(v)?;
            match list
                .iter_mut()
                .find(|d| d.get("id").and_then(Value::as_str) == Some(device.id.as_str()))
            {
                // Keep fields a later version added to the entry.
                Some(Value::Object(old)) => {
                    if let Value::Object(new) = entry {
                        old.extend(new);
                    }
                }
                Some(other) => *other = entry,
                None => list.push(entry),
            }
            Ok(())
        })?;
        Ok(device.clone())
    }

    /// Removes a device from `gear.json` and returns it. Its backups stay.
    pub fn forget_device(&self, id: &str) -> Result<Device> {
        let (found, _) = self.update(|v| {
            let list = devices_mut(v)?;
            let i = list
                .iter()
                .position(|d| d.get("id").and_then(Value::as_str) == Some(id));
            Ok(i.map(|i| list.remove(i)))
        })?;
        let found = found.with_context(|| format!("No device with id {id:?}."))?;
        serde_json::from_value(found).context("the device entry does not parse")
    }

    /// Notes that a device was seen now, saving it first when it is new (unnamed).
    pub fn seen(
        &self,
        id: &str,
        kind: DeviceKind,
        identity: &super::model::Identity,
    ) -> Result<Device> {
        let now = chrono::Utc::now();
        let mut d = self.device(id)?.unwrap_or(Device {
            id: id.to_string(),
            kind,
            name: String::new(),
            aircraft: None,
            identity: identity.clone(),
            last_seen: None,
            last_backup: None,
            last_space: None,
            aliases: Vec::new(),
            dfu_serial: None,
        });
        d.last_seen = Some(now);
        merge_identity(&mut d.identity, identity);
        self.save_device(&d)
    }
}

/// Fills fields of `into` that `from` knows; a field `from` does not know keeps its value.
pub fn merge_identity(into: &mut super::model::Identity, from: &super::model::Identity) {
    let set = |a: &mut Option<String>, b: &Option<String>| {
        if b.is_some() {
            a.clone_from(b);
        }
    };
    set(&mut into.board, &from.board);
    set(&mut into.firmware, &from.firmware);
    set(&mut into.version, &from.version);
    set(&mut into.build, &from.build);
    set(&mut into.target, &from.target);
}

fn devices_mut(v: &mut Values) -> Result<&mut Vec<Value>> {
    match v.entry(DEVICES).or_insert_with(|| Value::Array(Vec::new())) {
        Value::Array(a) => Ok(a),
        _ => bail!("gear.json: devices is not a list; fix or remove it"),
    }
}

/// A path part from an id: anything but letters, digits, `-`, `_` and `.` becomes `_`, so
/// an id can never leave its folder.
pub fn safe(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    match s.as_str() {
        "" | "." | ".." => "_".into(),
        _ => s,
    }
}

/// A list under `key` in `values`; entries that do not parse are skipped.
pub fn list<T: DeserializeOwned>(values: &Values, key: &str) -> Vec<T> {
    values
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| serde_json::from_value(x.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Puts a list under `key`.
pub fn set_list<T: Serialize>(values: &mut Values, key: &str, items: &[T]) -> Result<()> {
    values.insert(key.to_string(), serde_json::to_value(items)?);
    Ok(())
}

fn read(path: &Path) -> Result<Values> {
    match std::fs::read(path) {
        Ok(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(Values::new()),
        Ok(b) => match serde_json::from_slice::<Value>(&b)
            .with_context(|| format!("{} is not valid JSON; fix or remove it", path.display()))?
        {
            Value::Object(m) => Ok(m),
            _ => bail!("{} is not a JSON object; fix or remove it", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Values::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn update<T>(path: &Path, edit: impl FnOnce(&mut Values) -> Result<T>) -> Result<(T, Values)> {
    let dir = path.parent().context("gear.json has no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut lock_name = path.file_name().unwrap_or_default().to_os_string();
    lock_name.push(".lock");
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_file_name(lock_name))
        .context("opening the gear lock")?;
    lock.lock().context("locking gear.json")?;
    let before = read(path)?;
    let mut values = before.clone();
    let out = edit(&mut values)?;
    if values != before {
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(&values)?)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    }
    drop(lock);
    Ok((out, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::model::Identity;
    use serde_json::json;

    fn dev(id: &str) -> Device {
        Device {
            id: id.into(),
            kind: DeviceKind::Fc,
            name: "Bench quad".into(),
            aircraft: None,
            identity: Identity::default(),
            last_seen: None,
            last_backup: None,
            last_space: None,
            aliases: Vec::new(),
            dfu_serial: None,
        }
    }

    #[test]
    fn unknown_keys_survive_a_write() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path().join("gear"));
        std::fs::create_dir_all(s.root()).unwrap();
        std::fs::write(
            s.gear_file(),
            r#"{"fromTheFuture":{"a":1},"devices":[{"id":"fc-1","kind":"fc","name":"Old","futureField":true},{"id":"x","kind":"not-a-kind"}]}"#,
        )
        .unwrap();
        // The entry that does not parse is skipped on read but kept on disk.
        assert_eq!(s.devices().unwrap().len(), 1);
        s.save_device(&dev("fc-1")).unwrap();
        s.save_device(&dev("fc-2")).unwrap();
        let v = s.read().unwrap();
        assert_eq!(v["fromTheFuture"]["a"], 1);
        let list = v["devices"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["name"], "Bench quad");
        assert_eq!(
            list[0]["futureField"], true,
            "an entry's unknown field stays"
        );
        assert_eq!(list[1]["kind"], "not-a-kind");
        let gone = s.forget_device("fc-2").unwrap();
        assert_eq!(gone.id, "fc-2");
        assert!(s.forget_device("fc-2").is_err());
        assert_eq!(s.read().unwrap()["devices"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_replaced() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path());
        std::fs::write(s.gear_file(), "{not json").unwrap();
        assert!(s.save_device(&dev("fc-1")).is_err());
        assert_eq!(std::fs::read_to_string(s.gear_file()).unwrap(), "{not json");
        std::fs::write(s.gear_file(), r#"{"devices": 3}"#).unwrap();
        let e = s.save_device(&dev("fc-1")).unwrap_err();
        assert!(format!("{e:#}").contains("not a list"), "{e:#}");
    }

    #[test]
    fn concurrent_writers_keep_every_device() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path().join("gear"));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let s = s.clone();
                std::thread::spawn(move || {
                    s.save_device(&dev(&format!("fc-{i}"))).unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(s.devices().unwrap().len(), 8);
    }

    #[test]
    fn no_write_when_nothing_changes() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path());
        s.update(|_| Ok(())).unwrap();
        assert!(!s.gear_file().exists(), "an empty edit writes no file");
        s.save_device(&dev("fc-1")).unwrap();
        let before = std::fs::metadata(s.gear_file())
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.save_device(&dev("fc-1")).unwrap();
        let after = std::fs::metadata(s.gear_file())
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn seen_saves_a_new_device_and_keeps_a_name() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path());
        let ident = Identity {
            board: Some("pocket".into()),
            version: Some("2.12.4".into()),
            ..Default::default()
        };
        let x = s.seen("radio-1", DeviceKind::Radio, &ident).unwrap();
        assert_eq!(x.display_name(), "Unnamed Radio");
        assert!(x.last_seen.is_some());
        let mut named = x.clone();
        named.name = "Bench radio".into();
        s.save_device(&named).unwrap();
        let newer = Identity {
            version: Some("2.12.5".into()),
            ..Default::default()
        };
        let y = s.seen("radio-1", DeviceKind::Radio, &newer).unwrap();
        assert_eq!(y.name, "Bench radio");
        assert_eq!(y.identity.board.as_deref(), Some("pocket"));
        assert_eq!(y.identity.version.as_deref(), Some("2.12.5"));
    }

    #[test]
    fn paths_stay_inside_the_folder() {
        let s = Store::new("/g");
        assert_eq!(s.snapshots_dir("../x"), PathBuf::from("/g/snapshots/.._x"));
        assert_eq!(s.snapshots_dir(".."), PathBuf::from("/g/snapshots/_"));
        assert_eq!(
            s.blob_path("abcdef0123456789"),
            PathBuf::from("/g/blobs/ab/abcdef0123456789")
        );
        assert_eq!(s.logs_dir("radio-1"), PathBuf::from("/g/logs/radio-1"));
        let mut v = Values::new();
        set_list(&mut v, "packs", &[json!({"label": "1"})]).unwrap();
        assert_eq!(list::<Value>(&v, "packs").len(), 1);
    }
}
