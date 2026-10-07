//! `Core`'s switch map (design 7.2) and the radio as a USB joystick.
//!
//! - `gear_switch_map` reads an EdgeTX card folder (or one model file) and Betaflight dump,
//!   diff or CLI files, and builds the map. A device's latest backup (WP4) joins here as
//!   another source of the same files. With `live`, it marks each control's position from
//!   the FC's `MSP_RC` (one short MSP exchange, no cue, the port released after it), else
//!   from the radio's joystick.
//! - `gear_radio` is one look at the radio in USB Joystick mode.
//! - `gear_radio_watch` starts or stops the stream the app's controls page draws from: a
//!   thread sends `radio-input` events through `Hooks::event`.

use super::Core;
use crate::api::{Event, RadioInput};
use crate::gear::bf::dump::Config;
use crate::gear::edgetx::card::Card;
use crate::gear::edgetx::{model as em, yaml::Doc};
use crate::gear::radio_hid::{self, HidSource, RadioSnapshot};
use crate::gear::switchmap::{self, Inputs, SwitchMap};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Files larger than this are not a dump or a model file.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// `gear_switch_map`: what to read, and whether to mark the live positions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct SwitchMapParams {
    /// An EdgeTX card (its mount or a copy of its folder), or one model file.
    #[serde(default)]
    pub radio: Option<PathBuf>,
    /// The model file on the card (`model01.yml`); default: the radio's selected model.
    #[serde(default)]
    pub model: Option<String>,
    /// Betaflight `dump all`, `diff all` or CLI-line files, read in order: a later file's
    /// lines win.
    #[serde(default)]
    pub fc: Vec<PathBuf>,
    /// An aircraft profile: its radio's and FC's latest backups. Not available until
    /// backups exist.
    #[serde(default)]
    pub aircraft: Option<String>,
    /// Mark where each control is now: the FC's channels (`MSP_RC`) when an FC is plugged
    /// in, else the radio's joystick.
    #[serde(default)]
    pub live: bool,
    /// The FC's port for `live`; omitted when one FC is plugged in.
    #[serde(default)]
    pub port: Option<String>,
    /// Channel values to mark instead (µs, CH1 first).
    #[serde(default)]
    pub channels: Vec<u16>,
}

/// `gear_radio`: how long to wait for a report.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct RadioParams {
    /// Milliseconds; default 500, at most 5000.
    #[serde(default)]
    pub wait_ms: Option<u32>,
}

/// `gear_radio_watch`: start or stop the stream.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct RadioWatchParams {
    pub on: bool,
}

/// The joystick source and the running watch.
pub struct RadioState {
    source: Mutex<Arc<dyn HidSource>>,
    watch: Mutex<Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>>,
}

impl Default for RadioState {
    fn default() -> Self {
        Self {
            source: Mutex::new(radio_hid::system()),
            watch: Mutex::new(None),
        }
    }
}

fn read_file(path: &Path, what: &str) -> Result<Vec<u8>> {
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        bail!("{} is not {what}.", path.display());
    }
    std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// The track names in `SOUNDS/<lang>/`, lower case, without extension.
fn sounds(root: &Path, lang: &str) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(root.join("SOUNDS").join(lang))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension()?.to_str()?.eq_ignore_ascii_case("wav"))
                        .then(|| p.file_stem()?.to_str().map(|s| s.to_ascii_lowercase()))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads the radio side: the model, its inputs and limits, the switch types, the sounds.
fn read_radio(path: &Path, model: Option<&str>, inputs: &mut Inputs) -> Result<()> {
    let (root, model_path) = if path.is_dir() {
        let card = Card::open(path)
            .with_context(|| format!("{} is not an EdgeTX card", path.display()))?;
        let file = match model.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => card.selected_model().ok_or_else(|| {
                anyhow::anyhow!(
                    "{} names no selected model; pass a model file name.",
                    path.display()
                )
            })?,
        };
        (Some(path.to_path_buf()), path.join("MODELS").join(file))
    } else {
        // MODELS/modelNN.yml on a card copy: radio.yml is two folders up.
        let root = path
            .parent()
            .and_then(Path::parent)
            .filter(|r| r.join("RADIO/radio.yml").is_file())
            .map(Path::to_path_buf);
        (root, path.to_path_buf())
    };
    let bytes = read_file(&model_path, "an EdgeTX model file")?;
    let name = file_name(&model_path);
    let doc = Doc::parse(&name, &bytes).map_err(|r| anyhow::anyhow!(r.reason))?;
    let view = em::view(&name, &doc).map_err(|r| anyhow::anyhow!(r.reason))?;
    inputs.expos = switchmap::expos(&doc);
    inputs.limits = switchmap::limits(&doc);
    inputs.model = Some(view);
    inputs.sources.push(name);
    if let Some(root) = root {
        let radio = root.join("RADIO/radio.yml");
        if let Ok(b) = read_file(&radio, "radio.yml") {
            if let Ok(rd) = Doc::parse("radio.yml", &b) {
                inputs.switches = switchmap::switch_types(&rd);
                inputs.sounds = Some(sounds(&root, &switchmap::tts_language(&rd)));
                inputs.sources.push("radio.yml".into());
            }
        }
    }
    Ok(())
}

impl Core {
    /// Replaces the joystick source (tests pass `FakeHid`).
    pub fn with_radio_hid(self, source: Arc<dyn HidSource>) -> Core {
        *self.radio.source.lock().unwrap() = source;
        self
    }

    fn radio_source(&self) -> Arc<dyn HidSource> {
        self.radio.source.lock().unwrap().clone()
    }

    /// The switch map from the files given; with `live`, the controls' positions now.
    /// Reads only.
    pub fn gear_switch_map(&self, p: &SwitchMapParams) -> Result<SwitchMap> {
        if let Some(a) = p.aircraft.as_deref().filter(|a| !a.trim().is_empty()) {
            bail!(
                "No backups to read for aircraft {a:?} yet: pass the radio's card or model file and the FC's dump. Device backups come in a later version."
            );
        }
        if p.radio.is_none() && p.fc.is_empty() {
            bail!("Pass an EdgeTX card or model file, a Betaflight dump, or both.");
        }
        let mut inputs = Inputs::default();
        if let Some(r) = &p.radio {
            read_radio(r, p.model.as_deref(), &mut inputs)?;
        }
        let mut texts = Vec::new();
        for f in &p.fc {
            let b = read_file(f, "a Betaflight dump, diff or CLI file")?;
            texts.push(String::from_utf8_lossy(&b).into_owned());
            inputs.sources.push(file_name(f));
        }
        if !texts.is_empty() {
            inputs.fc = Some(Config::parse(&texts.join("\n")));
        }
        let mut map = switchmap::build(&inputs);
        if !p.channels.is_empty() {
            map.live = Some(switchmap::live(&map, "given", &p.channels));
        } else if p.live {
            let (source, channels) = self.live_channels(p.port.as_deref())?;
            map.live = Some(switchmap::live(&map, source, &channels));
        }
        Ok(map)
    }

    /// The channels now: the FC's when one is plugged in (or a port is named), else the
    /// radio joystick's.
    fn live_channels(&self, port: Option<&str>) -> Result<(&'static str, Vec<u16>)> {
        let fc_named = port.is_some_and(|p| !p.trim().is_empty());
        if fc_named || !self.fc_ports().is_empty() {
            let c = self.gear_fc_pick(port)?;
            let handle = super::gear::link_handle(&c.link);
            if self.held_by_job(&handle) {
                bail!("The FC is busy with another job; try again when it ends.");
            }
            let _hold = self.gear_hold(&handle);
            let ch =
                crate::gear::bf::rc_channels(self.gear.ports.as_ref(), &handle, self.fc_timing())?;
            return Ok(("fc", ch));
        }
        let s = radio_hid::snapshot(self.radio_source().as_ref(), Duration::from_millis(500))?;
        match s.frame {
            Some(f) => Ok(("radio", f.channels)),
            None if s.connected => bail!(
                "Nothing to read live: no FC is plugged in, and the radio sent no report. Is USB Joystick mode on?"
            ),
            None => bail!(
                "Nothing to read live: no FC is plugged in, and no radio is in USB Joystick mode."
            ),
        }
    }

    /// One look at the radio in USB Joystick mode.
    pub fn gear_radio(&self, p: &RadioParams) -> Result<RadioSnapshot> {
        let wait = Duration::from_millis(p.wait_ms.unwrap_or(500).min(5000) as u64);
        radio_hid::snapshot(self.radio_source().as_ref(), wait)
    }

    /// Starts or stops the radio stream (`radio-input` events). True while it runs.
    pub fn gear_radio_watch(&self, p: &RadioWatchParams) -> Result<bool> {
        let mut w = self.radio.watch.lock().unwrap();
        if let Some((stop, h)) = w.take() {
            if p.on && !h.is_finished() {
                *w = Some((stop, h));
                return Ok(true);
            }
            stop.store(true, Ordering::SeqCst);
            let _ = h.join();
        }
        if !p.on {
            return Ok(false);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (src, hooks, s2) = (self.radio_source(), self.hooks.clone(), stop.clone());
        let h = std::thread::Builder::new()
            .name("radio-watch".into())
            .spawn(move || {
                radio_hid::watch(src.as_ref(), &s2, |e| {
                    hooks.event(Event::RadioInput(RadioInput(e)))
                })
            })?;
        *w = Some((stop, h));
        Ok(true)
    }
}
