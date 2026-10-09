//! The firmware check (design 6.4, 7.10): the newest release of each product against what
//! each saved device reports. It reads the network only when asked (`firmwareCheck` is
//! `manual` by default; `daily` checks when the last answer is a day old) and keeps the
//! answer in `<cache>/firmware/latest.json`.
//!
//! Sources: the EdgeTX and Betaflight GitHub releases and the ExpressLRS artifactory index.
//! Each is parsed by a pure function over the downloaded JSON, so the tests use fixtures.

use super::fetch_bytes;
use crate::gear::compat::{self, Product};
use crate::gear::model::{Device, DeviceKind};
use crate::modules::fetch::Fetch;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::cmp::Ordering;
use std::path::Path;

pub const EDGETX_RELEASES: &str = "https://api.github.com/repos/EdgeTX/edgetx/releases?per_page=30";
pub const BETAFLIGHT_RELEASES: &str =
    "https://api.github.com/repos/betaflight/betaflight/releases?per_page=30";
pub const ELRS_INDEX: &str = "https://artifactory.expresslrs.org/ExpressLRS/index.json";

/// A version as the firmware writes it: numbers, and a suffix after `-` or `+`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ver {
    nums: Vec<u64>,
    pre: bool,
}

impl Ver {
    /// `v2.12.4`, `2026.6.0-alpha`, `4.5.1`. None when it starts with no number.
    pub fn parse(s: &str) -> Option<Ver> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let (core, suffix) = match s.find(['-', '+', ' ']) {
            Some(i) => (&s[..i], &s[i..]),
            None => (s, ""),
        };
        let nums: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
        let nums = nums.filter(|n| !n.is_empty())?;
        Some(Ver {
            nums,
            pre: suffix.starts_with('-'),
        })
    }
}

impl Ord for Ver {
    fn cmp(&self, o: &Ver) -> Ordering {
        let n = self.nums.len().max(o.nums.len());
        for i in 0..n {
            let (a, b) = (
                self.nums.get(i).copied().unwrap_or(0),
                o.nums.get(i).copied().unwrap_or(0),
            );
            if a != b {
                return a.cmp(&b);
            }
        }
        // A release is newer than its own pre-release.
        o.pre.cmp(&self.pre)
    }
}

impl PartialOrd for Ver {
    fn partial_cmp(&self, o: &Ver) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// One asset of a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// `sha256:<hex>` when GitHub reports it.
    pub digest: Option<String>,
}

/// One GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag without a leading `v`.
    pub version: String,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

fn release_of(v: &Value) -> Option<Release> {
    if v["draft"].as_bool().unwrap_or(false) {
        return None;
    }
    let tag = v["tag_name"].as_str()?;
    Ver::parse(tag)?;
    Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        prerelease: v["prerelease"].as_bool().unwrap_or(false),
        assets: v["assets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some(Asset {
                            name: x["name"].as_str()?.to_string(),
                            url: x["browser_download_url"].as_str()?.to_string(),
                            size: x["size"].as_u64().unwrap_or(0),
                            digest: x["digest"].as_str().map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The releases in a GitHub release list.
pub fn parse_releases(json: &[u8]) -> Result<Vec<Release>> {
    let v: Value = serde_json::from_slice(json).context("The release list is not JSON")?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("The release list is not a list"))?;
    Ok(arr.iter().filter_map(release_of).collect())
}

/// The one release in a `releases/tags/<tag>` answer.
pub fn parse_release(json: &[u8]) -> Result<Release> {
    let v: Value = serde_json::from_slice(json).context("The release is not JSON")?;
    release_of(&v).ok_or_else(|| anyhow::anyhow!("The release answer has no tag"))
}

/// The highest version that is not a pre-release.
pub fn latest_stable(releases: &[Release]) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| !r.prerelease && Ver::parse(&r.version).is_some_and(|v| !v.pre))
        .max_by_key(|r| Ver::parse(&r.version))
}

/// The highest tag of the ELRS index that is not a release candidate.
pub fn parse_elrs_index(json: &[u8]) -> Result<Option<String>> {
    let v: Value = serde_json::from_slice(json).context("The ExpressLRS index is not JSON")?;
    let tags = v["tags"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("The ExpressLRS index has no tags"))?;
    Ok(tags
        .keys()
        .filter_map(|k| Ver::parse(k).filter(|v| !v.pre).map(|v| (v, k.clone())))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, k)| k.trim_start_matches(['v', 'V']).to_string()))
}

/// The newest stable version of each product, and when it was read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct Latest {
    #[serde(default)]
    pub edgetx: Option<String>,
    #[serde(default)]
    pub betaflight: Option<String>,
    #[serde(default)]
    pub elrs: Option<String>,
    #[serde(default)]
    pub checked_at: Option<DateTime<Utc>>,
    /// Sources that failed, one sentence each.
    #[serde(default)]
    pub errors: Vec<String>,
}

fn cache_file(cache: &Path) -> std::path::PathBuf {
    cache.join("firmware").join("latest.json")
}

/// The last answer, or an empty one.
pub fn cached(cache: &Path) -> Latest {
    std::fs::read(cache_file(cache))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// True when `daily` is set and the last answer is a day old or missing.
pub fn due(setting: &str, last: &Latest, now: DateTime<Utc>) -> bool {
    setting == "daily"
        && last
            .checked_at
            .is_none_or(|t| now - t >= Duration::hours(24))
}

/// Reads the three sources and keeps the answer. A source that fails keeps its old value
/// and adds an error.
pub fn check(fetch: &dyn Fetch, cache: &Path, now: DateTime<Utc>) -> Result<Latest> {
    let scratch = cache.join("firmware");
    let old = cached(cache);
    let mut out = Latest {
        checked_at: Some(now),
        ..old.clone()
    };
    out.errors.clear();
    let mut note = |what: &str, e: anyhow::Error| out.errors.push(format!("{what}: {e:#}"));
    let mut edgetx = None;
    let mut betaflight = None;
    let mut elrs = None;
    match fetch_bytes(fetch, EDGETX_RELEASES, &scratch).and_then(|b| parse_releases(&b)) {
        Ok(r) => edgetx = Some(latest_stable(&r).map(|r| r.version.clone())),
        Err(e) => note("EdgeTX", e),
    }
    match fetch_bytes(fetch, BETAFLIGHT_RELEASES, &scratch).and_then(|b| parse_releases(&b)) {
        Ok(r) => betaflight = Some(latest_stable(&r).map(|r| r.version.clone())),
        Err(e) => note("Betaflight", e),
    }
    match fetch_bytes(fetch, ELRS_INDEX, &scratch).and_then(|b| parse_elrs_index(&b)) {
        Ok(v) => elrs = Some(v),
        Err(e) => note("ExpressLRS", e),
    }
    if let Some(v) = edgetx {
        out.edgetx = v;
    }
    if let Some(v) = betaflight {
        out.betaflight = v;
    }
    if let Some(v) = elrs {
        out.elrs = v;
    }
    std::fs::create_dir_all(&scratch)?;
    let f = cache_file(cache);
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&out)?)?;
    std::fs::rename(&tmp, &f)?;
    Ok(out)
}

/// Where a device stands against the newest release.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareState {
    UpToDate,
    Update,
    /// The device reports no version, or the newest release is not known.
    Unknown,
}

/// One row of the Firmware page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Type)]
pub struct FirmwareStatus {
    pub device: String,
    pub kind: DeviceKind,
    pub name: String,
    /// `EdgeTX`, `Betaflight`, `ExpressLRS`.
    pub product: String,
    pub board: Option<String>,
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub state: FirmwareState,
    /// True when QuadCam can build and flash the newest version for this device.
    pub flashable: bool,
    /// What the person should know: why QuadCam does not flash it, or that the device is ahead.
    pub note: Option<String>,
}

fn product_of(kind: DeviceKind) -> Option<(&'static str, Option<Product>)> {
    match kind {
        DeviceKind::Fc => Some(("Betaflight", None)),
        DeviceKind::Radio => Some(("EdgeTX", Some(Product::Edgetx))),
        DeviceKind::ElrsTx | DeviceKind::ElrsRx => Some(("ExpressLRS", None)),
        DeviceKind::Goggles | DeviceKind::DvrCard => None,
    }
}

/// The Firmware page's rows: one per saved device that runs firmware QuadCam checks.
pub fn statuses(devices: &[Device], latest: &Latest) -> Vec<FirmwareStatus> {
    devices
        .iter()
        .filter_map(|d| {
            let (product, compat_product) = product_of(d.kind)?;
            let newest = match d.kind {
                DeviceKind::Fc => latest.betaflight.clone(),
                DeviceKind::Radio => latest.edgetx.clone(),
                _ => latest.elrs.clone(),
            };
            let installed = d.identity.version.clone().filter(|v| !v.trim().is_empty());
            let (state, mut note) = match (&installed, &newest) {
                (Some(i), Some(n)) => match (Ver::parse(i), Ver::parse(n)) {
                    (Some(a), Some(b)) if a < b => (FirmwareState::Update, None),
                    (Some(a), Some(b)) if a > b => (
                        FirmwareState::UpToDate,
                        Some("This version is newer than the newest release.".to_string()),
                    ),
                    (Some(_), Some(_)) => (FirmwareState::UpToDate, None),
                    _ => (
                        FirmwareState::Unknown,
                        Some("QuadCam cannot compare these versions.".into()),
                    ),
                },
                (None, _) => (
                    FirmwareState::Unknown,
                    Some("The device reported no version.".into()),
                ),
                (_, None) => (FirmwareState::Unknown, None),
            };
            let mut flashable = false;
            if let (Some(p), Some(n)) = (compat_product, &newest) {
                match compat::check_writable(p, d.identity.board.as_deref(), Some(n)) {
                    Ok(()) => flashable = true,
                    Err(r) if state == FirmwareState::Update => {
                        note = Some(format!("QuadCam will not flash it: {}", r.reason))
                    }
                    Err(_) => {}
                }
            } else if state == FirmwareState::Update {
                note = Some(format!(
                    "QuadCam checks {product} versions. It does not flash them."
                ));
            }
            Some(FirmwareStatus {
                device: d.id.clone(),
                kind: d.kind,
                name: d.display_name(),
                product: product.into(),
                board: d.identity.board.clone(),
                installed,
                latest: newest,
                state,
                flashable,
                note,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::firmware::FixtureFetch;
    use crate::gear::model::Identity;
    use serde_json::json;

    pub(crate) fn releases_json(tags: &[(&str, bool)]) -> Vec<u8> {
        let v: Vec<Value> = tags
            .iter()
            .map(|(t, pre)| {
                json!({"tag_name": t, "prerelease": pre, "draft": false, "assets": [
                    {"name": format!("edgetx-firmware-{t}.zip"),
                     "browser_download_url": format!("https://example.invalid/{t}.zip"),
                     "size": 10, "digest": "sha256:abc"}]})
            })
            .collect();
        serde_json::to_vec(&v).unwrap()
    }

    fn device(kind: DeviceKind, board: &str, version: Option<&str>) -> Device {
        Device {
            id: format!("{}-1", kind.id_prefix()),
            kind,
            name: String::new(),
            aircraft: None,
            identity: Identity {
                board: Some(board.into()),
                version: version.map(str::to_string),
                ..Identity::default()
            },
            last_seen: None,
            last_backup: None,
            last_space: None,
            aliases: Vec::new(),
            dfu_serial: None,
        }
    }

    #[test]
    fn versions_compare_by_number_and_prerelease() {
        let v = |s| Ver::parse(s).unwrap();
        assert!(v("2.12.4") > v("2.12.3"));
        assert!(v("2.12.10") > v("2.12.9"));
        assert!(v("v2.13.0") > v("2.12.99"));
        assert!(v("2026.6.0") > v("2025.12.5"));
        assert!(v("2026.6.0") > v("2026.6.0-alpha"));
        assert_eq!(v("4.5").cmp(&v("4.5.0")), Ordering::Equal);
        assert!(Ver::parse("beta").is_none());
        assert!(Ver::parse("").is_none());
        assert!(v("2026.6.0-alpha").pre);
    }

    #[test]
    fn the_newest_stable_release_wins_whatever_the_order() {
        let r = parse_releases(&releases_json(&[
            ("v2.12.3", false),
            ("v2.13.0-rc1", true),
            ("v2.12.4", false),
            ("v2.9.0", false),
        ]))
        .unwrap();
        assert_eq!(latest_stable(&r).unwrap().version, "2.12.4");
        assert_eq!(r[0].assets[0].digest.as_deref(), Some("sha256:abc"));
        assert!(latest_stable(&[]).is_none());
    }

    #[test]
    fn a_draft_and_a_bad_tag_are_skipped() {
        let j = json!([
            {"tag_name": "v9.9.9", "draft": true, "prerelease": false, "assets": []},
            {"tag_name": "nightly", "draft": false, "prerelease": false, "assets": []},
            {"tag_name": "v1.2.3", "draft": false, "prerelease": false, "assets": []}
        ]);
        let r = parse_releases(j.to_string().as_bytes()).unwrap();
        assert_eq!(r.len(), 1);
        assert!(parse_releases(b"{}").is_err());
        assert!(parse_releases(b"nope").is_err());
    }

    #[test]
    fn the_elrs_index_gives_the_highest_non_candidate_tag() {
        let j = json!({"tags": {"3.5.3": "a", "4.0.0": "b", "4.0.1-RC1": "c", "3.6.0": "d"}, "branches": {"main": "x"}});
        assert_eq!(
            parse_elrs_index(j.to_string().as_bytes())
                .unwrap()
                .as_deref(),
            Some("4.0.0")
        );
        assert!(parse_elrs_index(b"{}").is_err());
    }

    fn serve_all(f: &FixtureFetch) {
        f.serve(
            EDGETX_RELEASES,
            releases_json(&[("v2.12.4", false), ("v2.11.0", false)]),
        );
        f.serve(BETAFLIGHT_RELEASES, releases_json(&[("2026.6.0", false)]));
        f.serve(
            ELRS_INDEX,
            json!({"tags": {"3.5.3": "a"}}).to_string().into_bytes(),
        );
    }

    #[test]
    fn a_check_reads_three_sources_and_caches_the_answer() {
        let d = tempfile::tempdir().unwrap();
        let f = FixtureFetch::new();
        serve_all(&f);
        let now = Utc::now();
        let l = check(&f, d.path(), now).unwrap();
        assert_eq!(l.edgetx.as_deref(), Some("2.12.4"));
        assert_eq!(l.betaflight.as_deref(), Some("2026.6.0"));
        assert_eq!(l.elrs.as_deref(), Some("3.5.3"));
        assert!(l.errors.is_empty());
        assert_eq!(cached(d.path()), l);
        assert_eq!(f.requested.lock().unwrap().len(), 3);
        // The answer is stale only after a day, and only with `daily`.
        assert!(!due("manual", &l, now + Duration::days(9)));
        assert!(!due("daily", &l, now + Duration::hours(23)));
        assert!(due("daily", &l, now + Duration::hours(24)));
        assert!(due("daily", &Latest::default(), now));
    }

    #[test]
    fn a_failing_source_keeps_its_old_value_and_reports() {
        let d = tempfile::tempdir().unwrap();
        let f = FixtureFetch::new();
        serve_all(&f);
        check(&f, d.path(), Utc::now()).unwrap();
        let g = FixtureFetch::new();
        g.serve(EDGETX_RELEASES, releases_json(&[("v2.12.5", false)]));
        g.serve(ELRS_INDEX, b"garbage".to_vec());
        let l = check(&g, d.path(), Utc::now()).unwrap();
        assert_eq!(l.edgetx.as_deref(), Some("2.12.5"));
        assert_eq!(l.betaflight.as_deref(), Some("2026.6.0"), "kept");
        assert_eq!(l.elrs.as_deref(), Some("3.5.3"), "kept");
        assert_eq!(l.errors.len(), 2, "{:?}", l.errors);
        assert!(l.errors.iter().any(|e| e.starts_with("Betaflight")));
        assert!(l.errors.iter().any(|e| e.starts_with("ExpressLRS")));
    }

    #[test]
    fn statuses_say_update_current_or_unknown() {
        let latest = Latest {
            edgetx: Some("2.12.4".into()),
            betaflight: Some("2026.6.0".into()),
            elrs: Some("3.5.3".into()),
            ..Latest::default()
        };
        let devices = vec![
            device(DeviceKind::Radio, "pocket", Some("2.12.3")),
            device(DeviceKind::Radio, "tx16s", Some("2.11.0")),
            device(DeviceKind::Fc, "BETAFPVG473_V2", Some("2025.12.5")),
            device(DeviceKind::ElrsRx, "rx", None),
            device(DeviceKind::ElrsTx, "tx", Some("3.5.3")),
            device(DeviceKind::Radio, "pocket", Some("2.13.0")),
            device(DeviceKind::Goggles, "g", None),
        ];
        let s = statuses(&devices, &latest);
        assert_eq!(s.len(), 6, "goggles have no firmware row");
        assert_eq!(s[0].state, FirmwareState::Update);
        assert!(s[0].flashable, "pocket 2.12.4 is proven");
        assert_eq!(s[1].state, FirmwareState::Update);
        assert!(!s[1].flashable);
        assert!(
            s[1].note.as_deref().unwrap().contains("will not flash"),
            "{:?}",
            s[1].note
        );
        assert_eq!(s[2].state, FirmwareState::Update);
        assert!(!s[2].flashable);
        assert!(s[2].note.as_deref().unwrap().contains("does not flash"));
        assert_eq!(s[3].state, FirmwareState::Unknown);
        assert_eq!(s[4].state, FirmwareState::UpToDate);
        assert_eq!(s[5].state, FirmwareState::UpToDate);
        assert!(s[5].note.as_deref().unwrap().contains("newer"));
        assert_eq!(s[0].name, "Unnamed Radio");
        // Nothing checked yet: unknown, not an update.
        let none = statuses(&devices[..1], &Latest::default());
        assert_eq!(none[0].state, FirmwareState::Unknown);
    }
}
