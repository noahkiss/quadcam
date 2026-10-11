//! `Core`'s switch map (design 7.2) and the radio as a USB joystick.
//!
//! - `gear_switch_map` reads an EdgeTX card folder (or one model file) and Betaflight dump,
//!   diff or CLI files, or the latest backups of saved devices (an aircraft's radio and FC),
//!   and builds the map. With `live`, it marks each control's position from
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
use crate::gear::model::DeviceKind;
use crate::gear::radio_hid::{self, HidSource, RadioSnapshot};
use crate::gear::switchmap::{self, Inputs, SwitchMap};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    /// An aircraft profile: the latest backups of the saved devices linked to it (its radio
    /// and its FC). The radio's model is the one whose name the profile's EdgeTX models
    /// list, else the radio's selected model.
    #[serde(default)]
    pub aircraft: Option<String>,
    /// Saved device ids whose latest backups to read: an FC's dump, a radio's card.
    #[serde(default)]
    pub devices: Vec<String>,
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

/// The joystick source and its one reader (`radio_hid::Hub`): the page's stream and the
/// sim's unthrottled sinks.
pub struct RadioState {
    pub(crate) hub: Arc<radio_hid::Hub>,
}

impl Default for RadioState {
    fn default() -> Self {
        Self {
            hub: radio_hid::Hub::new(radio_hid::system()),
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

/// A card to read the radio side from: a folder, or a radio's backup.
pub(super) trait CardSource {
    fn read(&self, rel: &str) -> Option<Vec<u8>>;
    /// Every file's path from the card's root.
    fn files(&self) -> Vec<String>;
}

pub(super) struct Folder(pub(super) PathBuf);

impl CardSource for Folder {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        read_file(&self.0.join(rel), "a card file").ok()
    }
    fn files(&self) -> Vec<String> {
        crate::gear::backup::card_files(&self.0)
            .map(|v| v.into_iter().map(|(p, _, _)| p).collect())
            .unwrap_or_default()
    }
}

pub(super) struct InBackup<'a> {
    pub(super) snaps: &'a crate::gear::backup::Snapshots,
    pub(super) backup: crate::gear::model::Backup,
}

impl CardSource for InBackup<'_> {
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let f = self.backup.files.iter().find(|f| f.path == rel)?;
        self.snaps
            .blobs()
            .get(&crate::gear::backup::blob_of(f))
            .ok()
    }
    fn files(&self) -> Vec<String> {
        self.backup.files.iter().map(|f| f.path.clone()).collect()
    }
}

/// Reads one model file into the inputs.
fn read_model(name: &str, bytes: &[u8], inputs: &mut Inputs) -> Result<()> {
    let doc = Doc::parse(name, bytes).map_err(|r| anyhow::anyhow!(r.reason))?;
    let view = em::view(name, &doc).map_err(|r| anyhow::anyhow!(r.reason))?;
    inputs.expos = switchmap::expos(&doc);
    inputs.limits = switchmap::limits(&doc);
    inputs.model = Some(view);
    Ok(())
}

/// Reads `radio.yml` (switch types, sound language) and the sound files, when there.
fn read_radio_yml(card: &dyn CardSource, inputs: &mut Inputs) -> Option<Doc> {
    let b = card.read("RADIO/radio.yml")?;
    let rd = Doc::parse("radio.yml", &b).ok()?;
    inputs.switches = switchmap::switch_types(&rd);
    let dir = format!("SOUNDS/{}/", switchmap::tts_language(&rd));
    inputs.sounds = Some(
        card.files()
            .iter()
            .filter_map(|p| p.strip_prefix(&dir))
            .filter_map(|n| {
                let (stem, ext) = n.rsplit_once('.')?;
                (ext.eq_ignore_ascii_case("wav") && !stem.contains('/'))
                    .then(|| stem.to_ascii_lowercase())
            })
            .collect(),
    );
    Some(rd)
}

/// The radio side from a card: the model named, else the one whose header name is in
/// `names`, else the selected one.
fn read_card(
    card: &dyn CardSource,
    model: Option<&str>,
    names: &[String],
    label: &str,
    inputs: &mut Inputs,
) -> Result<()> {
    let rd = read_radio_yml(card, inputs);
    let models: Vec<String> = card
        .files()
        .into_iter()
        .filter(|p| p.starts_with("MODELS/") && p.ends_with(".yml"))
        .collect();
    let by_name = || {
        models.iter().find(|m| {
            card.read(m)
                .and_then(|b| Doc::parse(m, &b).ok())
                .and_then(|d| em::model_name(&d).ok().flatten())
                .is_some_and(|n| names.iter().any(|x| x.eq_ignore_ascii_case(&n)))
        })
    };
    let file = match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => format!("MODELS/{m}"),
        None => match by_name() {
            Some(m) => m.clone(),
            None => format!(
                "MODELS/{}",
                rd.as_ref()
                    .and_then(crate::gear::edgetx::card::selected_in)
                    .ok_or_else(|| anyhow::anyhow!(
                        "{label} names no selected model; pass a model file name."
                    ))?
            ),
        },
    };
    let bytes = card
        .read(&file)
        .ok_or_else(|| anyhow::anyhow!("{label} has no {file}."))?;
    let name = file.trim_start_matches("MODELS/").to_string();
    read_model(&name, &bytes, inputs)?;
    inputs.sources.push(format!("{label} {name}"));
    Ok(())
}

/// Reads the radio side from a card folder or one model file.
fn read_radio(path: &Path, model: Option<&str>, inputs: &mut Inputs) -> Result<()> {
    if path.is_dir() {
        Card::open(path).with_context(|| format!("{} is not an EdgeTX card", path.display()))?;
        return read_card(
            &Folder(path.to_path_buf()),
            model,
            &[],
            &file_name(path),
            inputs,
        );
    }
    let bytes = read_file(path, "an EdgeTX model file")?;
    let name = file_name(path);
    read_model(&name, &bytes, inputs)?;
    inputs.sources.push(name);
    // MODELS/modelNN.yml on a card copy: radio.yml is two folders up.
    if let Some(root) = path
        .parent()
        .and_then(Path::parent)
        .filter(|r| r.join("RADIO/radio.yml").is_file())
    {
        if read_radio_yml(&Folder(root.to_path_buf()), inputs).is_some() {
            inputs.sources.push("radio.yml".into());
        }
    }
    Ok(())
}

impl Core {
    /// Replaces the joystick source (tests pass `FakeHid`).
    pub fn with_radio_hid(self, source: Arc<dyn HidSource>) -> Core {
        self.radio.hub.set_source(source);
        self
    }

    fn radio_source(&self) -> Arc<dyn HidSource> {
        self.radio.hub.source()
    }

    /// Every radio report, unthrottled and stamped on arrival, until the subscription
    /// drops (the sim's input ring, the calibration).
    pub fn radio_subscribe(&self, sink: radio_hid::Sink) -> Result<radio_hid::Subscription> {
        self.radio.hub.subscribe(sink)
    }

    /// The switch map from the files given; with `live`, the controls' positions now.
    /// Reads only.
    pub fn gear_switch_map(&self, p: &SwitchMapParams) -> Result<SwitchMap> {
        let mut devices = p.devices.clone();
        let mut names = Vec::new();
        let mut model = p.model.clone();
        if let Some(a) = p
            .aircraft
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            // The aircraft's devices: its profile's FC and radio, and every device linked
            // to it (an FC names its one aircraft; a radio's aircraft name the radio).
            let profile = self
                .profiles()?
                .0
                .into_iter()
                .find(|x| x.name.trim().eq_ignore_ascii_case(a));
            let mut linked: Vec<String> = profile
                .iter()
                .flat_map(|pr| [pr.gear.fc.clone(), pr.gear.radio.clone()])
                .flatten()
                .collect();
            for d in self.gear_store().devices()? {
                if d.kind != DeviceKind::Radio
                    && d.aircraft
                        .as_deref()
                        .is_some_and(|x| x.eq_ignore_ascii_case(a))
                    && !linked.contains(&d.id)
                {
                    linked.push(d.id);
                }
            }
            if linked.is_empty() {
                bail!("No saved device is linked to aircraft {a:?} (quadcam-cli gear devices save <id> --aircraft {a:?}).");
            }
            devices.extend(linked.into_iter().filter(|id| !p.devices.contains(id)));
            if let Some(pr) = profile {
                names = pr.edgetx_models;
                model = model.or(pr.gear.edgetx_model);
            }
        }
        if p.radio.is_none() && p.fc.is_empty() && devices.is_empty() {
            bail!("Pass an EdgeTX card or model file, a Betaflight dump, a device or an aircraft.");
        }
        let mut inputs = Inputs::default();
        let mut texts = Vec::new();
        let snaps = crate::gear::backup::Snapshots::new(self.gear_store());
        for id in &devices {
            let Some(b) = snaps.latest(id) else {
                bail!("No backup of {id:?} yet: back it up, or pass its files.");
            };
            let date = b.taken_at.format("%Y-%m-%d %H:%M").to_string();
            if let Some(f) = ["dump all", "diff all"]
                .iter()
                .find_map(|c| b.files.iter().find(|f| f.path == *c))
            {
                let bytes = snaps.blobs().get(&crate::gear::backup::blob_of(f))?;
                texts.push(String::from_utf8_lossy(&bytes).into_owned());
                inputs.sources.push(format!("{id} {} ({date})", f.path));
            } else if b.files.iter().any(|f| f.path == "RADIO/radio.yml") && p.radio.is_none() {
                let label = format!("{id} ({date})");
                read_card(
                    &InBackup {
                        snaps: &snaps,
                        backup: b,
                    },
                    model.as_deref(),
                    &names,
                    &label,
                    &mut inputs,
                )?;
            }
        }
        if let Some(r) = &p.radio {
            read_radio(r, p.model.as_deref(), &mut inputs)?;
        }
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
        let hooks = self.hooks.clone();
        self.radio.hub.watch(
            p.on,
            Arc::new(move |e| hooks.event(Event::RadioInput(RadioInput(e)))),
        )
    }
}
