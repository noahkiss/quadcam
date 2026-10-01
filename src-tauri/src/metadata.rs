//! Per-clip metadata beyond the name and date: location, keywords, author, and the gear from
//! an aircraft profile. Profiles and saved places live in the app's settings file, never in
//! code. At export this becomes Apple QuickTime metadata (see `qtmeta`).

use crate::moments::{Moment, MomentKind};
use anyhow::{bail, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

/// A saved place, kept in the settings file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// One aircraft setup, kept in the settings file. Every field is optional. A clip dated
/// from a radio log picks the profile whose `edgetx_models` holds the log's model name.
/// A later digital video source (an MP4 with `.srt` or `.osd` sidecars) can hang off a
/// profile through `video_system`; only analog DVR files are read today.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    /// The aircraft, for example "5-inch freestyle".
    pub aircraft: String,
    /// The recorder: goggles or DVR maker and model. Written as the camera make and model.
    pub camera_make: String,
    pub camera_model: String,
    /// A label: analog, DJI, Walksnail, HDZero.
    pub video_system: String,
    pub keywords: Vec<String>,
    pub author: String,
    /// Name of a saved place used when a clip has no location.
    pub place: Option<String>,
    /// EdgeTX model names (the start of the log file name) that mean this profile.
    pub edgetx_models: Vec<String>,
}

/// A location in decimal degrees.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    /// The saved place it came from, if any.
    #[serde(default)]
    pub name: Option<String>,
}

impl Location {
    pub fn check(&self) -> Result<()> {
        if !(self.lat.is_finite() && (-90.0..=90.0).contains(&self.lat))
            || !(self.lon.is_finite() && (-180.0..=180.0).contains(&self.lon))
        {
            bail!(
                "location {}, {} is not a latitude (-90..90) and longitude (-180..180)",
                self.lat,
                self.lon
            );
        }
        Ok(())
    }

    /// ISO 6709, as Apple writes it: `+40.6892-074.0445/`.
    pub fn iso6709(&self) -> String {
        format!("{:+08.4}{:+09.4}/", self.lat, self.lon)
    }
}

/// What the person (or an agent) set on one clip. Missing values come from the profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default)]
pub struct ClipMeta {
    /// Profile name. None: the log's model, then the session default.
    pub profile: Option<String>,
    pub location: Option<Location>,
    /// Keywords on top of the profile's.
    pub keywords: Vec<String>,
    /// Overrides the profile's author.
    pub author: Option<String>,
}

/// Flight numbers from the radio log rows a clip claimed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct FlightStats {
    pub armed_s: f64,
    pub packs: usize,
    pub min_rx_bat_v: Option<f64>,
    pub min_lq: Option<f64>,
    pub min_rssi_db: Option<f64>,
    pub max_throttle: Option<f64>,
}

impl FlightStats {
    pub fn line(&self) -> String {
        let s = self.armed_s.round() as u64;
        let mut parts = vec![format!(
            "armed {}:{:02} over {} pack{}",
            s / 60,
            s % 60,
            self.packs,
            if self.packs == 1 { "" } else { "s" }
        )];
        if let Some(v) = self.min_rx_bat_v {
            parts.push(format!("min RxBt {v:.2} V"));
        }
        if let Some(v) = self.min_lq {
            parts.push(format!("min LQ {v:.0}%"));
        }
        if let Some(v) = self.min_rssi_db {
            parts.push(format!("min RSSI {v:.0} dB"));
        }
        if let Some(v) = self.max_throttle {
            parts.push(format!("max throttle {:.0}%", v * 100.0));
        }
        parts.join(", ")
    }
}

/// The metadata one export writes, after profile and defaults are applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Resolved {
    pub profile: Option<String>,
    pub location: Option<Location>,
    pub make: String,
    pub model: String,
    pub aircraft: String,
    pub video_system: String,
    pub keywords: Vec<String>,
    pub author: String,
    pub flight: Option<String>,
}

/// The profile for a clip: its own choice, else the one mapped to the log's EdgeTX model,
/// else the session default.
pub fn pick_profile<'a>(
    meta: &ClipMeta,
    log_model: Option<&str>,
    profiles: &'a [Profile],
    default: Option<&str>,
) -> Option<&'a Profile> {
    let by_name = |n: &str| {
        profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(n.trim()))
    };
    if let Some(n) = meta.profile.as_deref().filter(|n| !n.trim().is_empty()) {
        return by_name(n);
    }
    if let Some(m) = log_model {
        if let Some(p) = profiles.iter().find(|p| {
            p.edgetx_models
                .iter()
                .any(|x| x.trim().eq_ignore_ascii_case(m.trim()))
        }) {
            return Some(p);
        }
    }
    default.and_then(by_name)
}

/// Keywords without blanks or repeats (case-insensitive), in first-seen order.
pub fn clean_keywords<'a>(words: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words {
        let w = w.trim();
        if !w.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(w)) {
            out.push(w.to_string());
        }
    }
    out
}

/// Applies the profile and saved places to one clip's metadata. Keywords are "FPV", the
/// profile's, the clip's, and the kinds of radio-log moments scored 0.5 or more.
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    meta: &ClipMeta,
    log_model: Option<&str>,
    profiles: &[Profile],
    default_profile: Option<&str>,
    places: &[Place],
    moments: &[Moment],
    duration: f64,
    flight: Option<&FlightStats>,
) -> Resolved {
    let p = pick_profile(meta, log_model, profiles, default_profile);
    let place = || {
        let name = p?.place.as_deref()?;
        let pl = places
            .iter()
            .find(|x| x.name.eq_ignore_ascii_case(name.trim()))?;
        Some(Location {
            lat: pl.lat,
            lon: pl.lon,
            name: Some(pl.name.clone()),
        })
    };
    let kinds: Vec<&str> = moments
        .iter()
        .filter(|m| m.kind != MomentKind::DeadAir && m.score >= 0.5)
        .filter(|m| m.end > 0.0 && m.start < duration)
        .map(|m| m.kind.label())
        .collect();
    let profile_words: Vec<&str> = p
        .map(|p| p.keywords.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let keywords = clean_keywords(
        std::iter::once("FPV")
            .chain(profile_words)
            .chain(meta.keywords.iter().map(String::as_str))
            .chain(kinds),
    );
    Resolved {
        profile: p.map(|p| p.name.clone()),
        location: meta.location.clone().or_else(place),
        make: p.map(|p| p.camera_make.clone()).unwrap_or_default(),
        model: p.map(|p| p.camera_model.clone()).unwrap_or_default(),
        aircraft: p.map(|p| p.aircraft.clone()).unwrap_or_default(),
        video_system: p.map(|p| p.video_system.clone()).unwrap_or_default(),
        keywords,
        author: meta
            .author
            .clone()
            .filter(|a| !a.trim().is_empty())
            .or_else(|| p.map(|p| p.author.clone()))
            .unwrap_or_default(),
        flight: flight.map(FlightStats::line),
    }
}

/// `com.apple.quicktime.creationdate`: local time with its UTC offset, so Photos shows the
/// time of day the clip was flown.
pub fn creation_date(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local)
        .format("%Y-%m-%dT%H:%M:%S%z")
        .to_string()
}

/// The QuickTime items for one file. Empty values are left out.
pub fn qt_items(r: &Resolved, base: &crate::media::Meta) -> Vec<(String, String)> {
    let q = |k: &str| format!("com.apple.quicktime.{k}");
    let mut v = vec![
        (q("creationdate"), creation_date(base.creation_time)),
        (
            q("software"),
            format!("QuadCam {}", env!("CARGO_PKG_VERSION")),
        ),
        (q("title"), base.title.clone()),
        (q("description"), base.description.clone()),
        (q("comment"), base.comment.clone()),
        (q("make"), r.make.clone()),
        (q("model"), r.model.clone()),
        (q("author"), r.author.clone()),
        (q("keywords"), r.keywords.join(",")),
        ("app.quadcam.aircraft".into(), r.aircraft.clone()),
        ("app.quadcam.video_system".into(), r.video_system.clone()),
        (
            "app.quadcam.flight".into(),
            r.flight.clone().unwrap_or_default(),
        ),
    ];
    if let Some(l) = &r.location {
        v.insert(0, (q("location.ISO6709"), l.iso6709()));
    }
    v.retain(|(_, val)| !val.trim().is_empty());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moments::Source;

    fn profiles() -> Vec<Profile> {
        vec![
            Profile {
                name: "Whoop".into(),
                camera_make: "Maker".into(),
                camera_model: "Goggles".into(),
                keywords: vec!["tinywhoop".into(), "fpv".into()],
                author: "Pilot".into(),
                place: Some("Field".into()),
                edgetx_models: vec!["AIR65 II".into()],
                ..Default::default()
            },
            Profile {
                name: "Five".into(),
                aircraft: "5-inch".into(),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn profile_choice_order() {
        let ps = profiles();
        let m = ClipMeta::default();
        assert_eq!(
            pick_profile(&m, Some("air65 ii"), &ps, Some("Five"))
                .unwrap()
                .name,
            "Whoop"
        );
        assert_eq!(
            pick_profile(&m, Some("Other"), &ps, Some("Five"))
                .unwrap()
                .name,
            "Five"
        );
        assert!(pick_profile(&m, None, &ps, None).is_none());
        let own = ClipMeta {
            profile: Some("five".into()),
            ..Default::default()
        };
        assert_eq!(
            pick_profile(&own, Some("AIR65 II"), &ps, None)
                .unwrap()
                .name,
            "Five"
        );
    }

    #[test]
    fn resolves_keywords_place_and_items() {
        let places = vec![Place {
            name: "Field".into(),
            lat: 40.68919,
            lon: -74.04449,
        }];
        let roll = Moment {
            kind: MomentKind::Roll,
            start: 3.0,
            end: 3.5,
            score: 0.8,
            source: Source::RadioLog,
            detail: String::new(),
        };
        let weak = Moment {
            kind: MomentKind::Crash,
            score: 0.3,
            ..roll.clone()
        };
        let meta = ClipMeta {
            keywords: vec!["park".into(), "Roll".into()],
            ..Default::default()
        };
        let stats = FlightStats {
            armed_s: 252.0,
            packs: 2,
            min_rx_bat_v: Some(3.52),
            min_lq: Some(71.0),
            min_rssi_db: None,
            max_throttle: Some(1.0),
        };
        let r = resolve(
            &meta,
            Some("AIR65 II"),
            &profiles(),
            None,
            &places,
            &[roll, weak],
            10.0,
            Some(&stats),
        );
        assert_eq!(r.keywords, ["FPV", "tinywhoop", "park", "Roll"]);
        assert_eq!(r.location.as_ref().unwrap().iso6709(), "+40.6892-074.0445/");
        assert_eq!(r.author, "Pilot");
        assert_eq!(
            r.flight.as_deref(),
            Some("armed 4:12 over 2 packs, min RxBt 3.52 V, min LQ 71%, max throttle 100%")
        );
        let base = crate::media::Meta {
            title: "Loops".into(),
            comment: String::new(),
            creation_time: "2026-09-28T14:00:00Z".parse().unwrap(),
            date: "2026-09-28".into(),
            description: "DVR PICT0001.AVI; date source: radio log".into(),
        };
        let items = qt_items(&r, &base);
        let keys: Vec<&str> = items.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys[0], "com.apple.quicktime.location.ISO6709");
        assert!(keys.contains(&"com.apple.quicktime.make"));
        assert!(
            !keys.contains(&"com.apple.quicktime.comment"),
            "empty values are left out"
        );
        assert!(!keys.contains(&"app.quadcam.aircraft"));
        let cd = &items
            .iter()
            .find(|(k, _)| k.ends_with("creationdate"))
            .unwrap()
            .1;
        assert_eq!(
            chrono::DateTime::parse_from_str(cd, "%Y-%m-%dT%H:%M:%S%z")
                .unwrap()
                .with_timezone(&Utc),
            base.creation_time,
            "local time with offset, same instant"
        );
        assert!(Location {
            lat: 91.0,
            lon: 0.0,
            name: None
        }
        .check()
        .is_err());
        assert_eq!(
            Location {
                lat: -3.5,
                lon: 120.25,
                name: None
            }
            .iso6709(),
            "-03.5000+120.2500/"
        );
    }
}
