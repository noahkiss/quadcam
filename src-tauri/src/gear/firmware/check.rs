//! The firmware check (design 6.4, 7.10): the newest release of each product against what
//! each saved device reports. It reads the network only when asked (`firmwareCheck` is
//! `manual` by default; `daily` checks when the last answer is a day old) and keeps the
//! answer in `<cache>/firmware/latest.json`.
//!
//! Sources: the EdgeTX and Betaflight GitHub releases and the ExpressLRS artifactory index.
//! Each is parsed by a pure function over the downloaded JSON, so the tests use fixtures.
//! An FC in the vendor channel (`vendor`) is checked against its maker's newest file
//! instead; official Betaflight then shows as information only.

use super::fetch_bytes;
use super::vendor::{self, BuildId, ChannelReason, CloudBuild, VendorFeed, VendorFile};
use crate::gear::bf::boards::{self, Feed, VendorSource};
use crate::gear::compat::{self, Product};
use crate::gear::model::{Device, DeviceKind, FirmwareChannel};
use crate::metadata::Profile;
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
    /// Makers' feeds as last read, for FCs in the vendor channel.
    #[serde(default)]
    pub vendor: Vec<VendorFeed>,
    /// The saved FCs' cloud builds as the build service reported them. A key's answer
    /// never changes, so each is read once.
    #[serde(default)]
    pub builds: Vec<CloudBuild>,
}

/// What a check reads beyond the three releases: the makers' feeds the saved FCs follow
/// (with the file prefixes wanted from each), and their cloud build keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wants {
    pub feeds: Vec<(String, Feed, Vec<String>)>,
    pub keys: Vec<String>,
}

/// The names an FC's aircraft goes by: the linked profile's name and its aircraft.
fn aircraft_names(d: &Device, profiles: &[Profile]) -> Vec<String> {
    let Some(a) = d.aircraft.as_deref() else {
        return Vec::new();
    };
    let mut names = vec![a.to_string()];
    if let Some(p) = profiles
        .iter()
        .find(|p| p.name.trim().eq_ignore_ascii_case(a.trim()))
    {
        names.push(p.aircraft.clone());
    }
    names
}

/// The maker's builds an FC can follow, from its identity and its aircraft.
pub fn source_of(d: &Device, profiles: &[Profile]) -> Option<VendorSource> {
    if d.kind != DeviceKind::Fc {
        return None;
    }
    let names = aircraft_names(d, profiles);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    boards::vendor_source(&d.identity, &names)
}

/// What a check must read for these devices.
pub fn wants(devices: &[Device], profiles: &[Profile]) -> Wants {
    let mut w = Wants::default();
    for d in devices.iter().filter(|d| d.kind == DeviceKind::Fc) {
        if let Some(src) = source_of(d, profiles) {
            match w.feeds.iter_mut().find(|(_, f, _)| *f == src.feed) {
                Some((_, _, prefixes)) if !prefixes.contains(&src.prefix) => {
                    prefixes.push(src.prefix)
                }
                Some(_) => {}
                None => w.feeds.push((src.vendor, src.feed, vec![src.prefix])),
            }
            if let Some(k) = d.identity.build_key() {
                if !w.keys.iter().any(|x| x == k) {
                    w.keys.push(k.to_string());
                }
            }
        }
    }
    w
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
    check_with(fetch, cache, now, &Wants::default())
}

/// `check`, and the makers' feeds and cloud builds `wants` names. A key the cache answers
/// is not read again.
pub fn check_with(
    fetch: &dyn Fetch,
    cache: &Path,
    now: DateTime<Utc>,
    wants: &Wants,
) -> Result<Latest> {
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
    for (name, feed, prefixes) in &wants.feeds {
        let old = out.vendor.iter().position(|f| f.feed == *feed);
        match vendor::read_feed(
            fetch,
            feed,
            prefixes,
            old.map(|i| &out.vendor[i]),
            &scratch,
            now,
        ) {
            Ok(f) => match old {
                Some(i) => out.vendor[i] = f,
                None => out.vendor.push(f),
            },
            Err(e) => out.errors.push(format!("{name}: {e:#}")),
        }
    }
    for key in &wants.keys {
        if out.builds.iter().any(|b| b.key == *key) {
            continue;
        }
        match fetch_bytes(fetch, &vendor::build_url(key), &scratch)
            .and_then(|b| vendor::parse_build(key, &b))
        {
            Ok(b) => out.builds.push(b),
            Err(e) => out.errors.push(format!("Betaflight build service: {e:#}")),
        }
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
    /// For an FC: where its firmware comes from. `latest` and `state` follow it.
    #[serde(default)]
    pub channel: Option<FirmwareChannel>,
    #[serde(default)]
    pub channel_reason: Option<ChannelReason>,
    /// For an FC: the maker whose builds it can follow (`BetaFPV`); None when QuadCam knows
    /// none for it.
    #[serde(default)]
    pub vendor: Option<String>,
    /// The installed build's day (`Jun 25 2026`), when the device reports it.
    #[serde(default)]
    pub installed_date: Option<String>,
    /// In the vendor channel: the maker's newest file for this quad.
    #[serde(default)]
    pub vendor_file: Option<VendorFile>,
    /// For an FC: the newest official Betaflight release.
    #[serde(default)]
    pub official: Option<String>,
    /// For an FC: what the channel means for it, when that needs saying.
    #[serde(default)]
    pub channel_note: Option<String>,
    /// For a radio: how to put it in DFU mode, one step each (`gear::radios`); empty for a
    /// board QuadCam has no profile for.
    #[serde(default)]
    pub dfu_steps: Vec<String>,
}

fn product_of(kind: DeviceKind) -> Option<(&'static str, Option<Product>)> {
    match kind {
        DeviceKind::Fc => Some(("Betaflight", None)),
        DeviceKind::Radio => Some(("EdgeTX", Some(Product::EdgetxFlash))),
        DeviceKind::ElrsTx | DeviceKind::ElrsRx => Some(("ExpressLRS", None)),
        DeviceKind::Goggles | DeviceKind::DvrCard => None,
    }
}

/// The Firmware page's rows: one per saved device that runs firmware QuadCam checks.
pub fn statuses(devices: &[Device], latest: &Latest) -> Vec<FirmwareStatus> {
    statuses_with(devices, latest, &[])
}

/// Where an FC's build stands against its maker's newest file, by build identity.
fn vendor_state(
    vendor: &str,
    installed: Option<&BuildId>,
    newest: Option<&BuildId>,
) -> (FirmwareState, Option<String>) {
    let (Some(own), Some(new)) = (installed, newest) else {
        return match installed {
            None => (
                FirmwareState::Unknown,
                Some("The device reported no version.".into()),
            ),
            Some(_) => (FirmwareState::Unknown, None),
        };
    };
    if vendor::same_build(own, new) {
        return (FirmwareState::UpToDate, None);
    }
    let order = match (own.date, new.date) {
        (Some(a), Some(b)) if a != b => Some(a.cmp(&b)),
        _ => match (Ver::parse(&own.version), Ver::parse(&new.version)) {
            (Some(a), Some(b)) if a != b => Some(a.cmp(&b)),
            _ => None,
        },
    };
    match order {
        Some(Ordering::Less) => (FirmwareState::Update, None),
        Some(_) => (
            FirmwareState::UpToDate,
            Some(format!("This build is newer than {vendor}'s newest file.")),
        ),
        None => (
            FirmwareState::Unknown,
            Some(format!(
                "QuadCam cannot compare this build with {vendor}'s file."
            )),
        ),
    }
}

/// `statuses`, with each FC's channel: its maker's builds (`profiles` give its aircraft)
/// or official Betaflight.
pub fn statuses_with(
    devices: &[Device],
    latest: &Latest,
    profiles: &[Profile],
) -> Vec<FirmwareStatus> {
    let mut rows = statuses_official(devices, latest);
    for (row, d) in rows
        .iter_mut()
        .zip(devices.iter().filter(|d| product_of(d.kind).is_some()))
    {
        if d.kind != DeviceKind::Fc {
            continue;
        }
        let src = source_of(d, profiles);
        let feed = src
            .as_ref()
            .and_then(|s| latest.vendor.iter().find(|f| f.feed == s.feed));
        let cloud = d
            .identity
            .build_key()
            .and_then(|k| latest.builds.iter().find(|b| b.key == k));
        let det = vendor::detect(
            d.firmware_channel,
            src.as_ref(),
            &d.identity,
            feed.map(|f| f.files.as_slice()),
            cloud,
        );
        let own = BuildId::of_identity(&d.identity);
        row.channel = Some(det.channel);
        row.channel_reason = Some(det.reason);
        row.vendor = src.as_ref().map(|s| s.vendor.clone());
        row.installed_date = own.as_ref().and_then(BuildId::day_label);
        row.official = latest.betaflight.clone();
        let (FirmwareChannel::Vendor, Some(src)) = (det.channel, &src) else {
            if det.reason == ChannelReason::CloudBuild {
                if let Some(v) = &row.vendor {
                    row.channel_note = Some(format!(
                        "This FC runs a Betaflight cloud build, not {v}'s file."
                    ));
                }
            }
            continue;
        };
        let newest = feed.and_then(|f| vendor::newest(src, &f.files)).cloned();
        let (state, note) = vendor_state(
            &src.vendor,
            own.as_ref(),
            newest.as_ref().and_then(|f| f.build.as_ref()),
        );
        row.latest = newest
            .as_ref()
            .and_then(|f| f.build.as_ref())
            .map(|b| b.version.clone());
        row.state = state;
        row.flashable = false;
        row.note = if state == FirmwareState::Update {
            Some(format!(
                "QuadCam checks {} builds. It does not flash them.",
                src.vendor
            ))
        } else {
            note
        };
        row.vendor_file = newest;
    }
    rows
}

fn statuses_official(devices: &[Device], latest: &Latest) -> Vec<FirmwareStatus> {
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
                channel: None,
                channel_reason: None,
                vendor: None,
                installed_date: None,
                vendor_file: None,
                official: None,
                channel_note: None,
                dfu_steps: if d.kind == DeviceKind::Radio {
                    crate::gear::radios::dfu_steps(d.identity.board.as_deref())
                } else {
                    Vec::new()
                },
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
    use std::path::Path;

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
            radio_aircraft: None,
            firmware_channel: None,
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

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/firmware")
                .join(name),
        )
        .unwrap()
    }

    const KEY: &str = "00112233445566778899aabbccddeeff";
    const M75_TXT: &str = "https://support.betafpv.com/hc/article_attachments/59863253804825";
    const A65_TXT: &str = "https://support.betafpv.com/hc/article_attachments/58300171193113";

    fn bf_fc(board: &str, version: &str, build: &str, key: Option<&str>, aircraft: &str) -> Device {
        let mut d = device(DeviceKind::Fc, board, Some(version));
        d.identity.build = Some(build.into());
        d.identity.build_key = key.map(str::to_string);
        d.aircraft = Some(aircraft.into());
        d
    }

    fn profiles() -> Vec<Profile> {
        ["Meteor75 Pro II", "Air65 II"]
            .iter()
            .map(|n| Profile {
                name: n.to_string(),
                ..Profile::default()
            })
            .collect()
    }

    /// A check with the BetaFPV feeds and one cloud build served from fixtures.
    fn checked(devices: &[Device], submitted: &str) -> (Latest, FixtureFetch) {
        let d = tempfile::tempdir().unwrap();
        let f = FixtureFetch::new();
        serve_all(&f);
        f.serve(BETAFLIGHT_RELEASES, releases_json(&[("2026.6.2", false)]));
        f.serve(
            &vendor::attachments_url("support.betafpv.com", "59855387002521"),
            fixture("zendesk-attachments-meteor75-o4.json"),
        );
        f.serve(M75_TXT, fixture("betafpv-m75-o4-elrs-20260625.txt"));
        f.serve(
            &vendor::attachments_url("support.betafpv.com", "54654422544409"),
            fixture("zendesk-attachments-air65-ii.json"),
        );
        f.serve(A65_TXT, fixture("betafpv-a65-f-20260515.txt"));
        f.serve(
            "https://support.betafpv.com/hc/article_attachments/58300189274265",
            fixture("betafpv-a65-f-20260421.txt"),
        );
        f.serve(
            &vendor::build_url(KEY),
            json!({"request": {"release": "2026.6.0-alpha", "target": "BETAFPVG473_V2"},
                   "submitted": submitted})
            .to_string()
            .into_bytes(),
        );
        let w = wants(devices, &profiles());
        let l = check_with(&f, d.path(), Utc::now(), &w).unwrap();
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        (l, f)
    }

    #[test]
    fn a_makers_build_is_checked_against_the_makers_file() {
        // The Meteor75 Pro II O4 on BetaFPV's file, saved before QuadCam read build keys.
        let meteor = bf_fc(
            "BETAFPVG473",
            "2025.12.5-alpha",
            "Jun 25 2026 03:24:50 eb2bb5a",
            None,
            "Meteor75 Pro II",
        );
        // The Air65 II on BetaFPV's cloud build of May 15.
        let air = bf_fc(
            "BETAFPVG473_V2",
            "2026.6.0-alpha",
            "May 15 2026 06:14:55 e92c10887",
            Some(KEY),
            "Air65 II",
        );
        let devices = vec![meteor.clone(), air];
        let (l, f) = checked(&devices, "2026-05-15T06:14:40Z");
        assert_eq!(l.vendor.len(), 2);
        assert_eq!(l.builds.len(), 1);
        let rows = statuses_with(&devices, &l, &profiles());
        let m = &rows[0];
        assert_eq!(m.channel, Some(FirmwareChannel::Vendor));
        assert_eq!(m.channel_reason, Some(ChannelReason::LocalBuild));
        assert_eq!(m.vendor.as_deref(), Some("BetaFPV"));
        assert_eq!(m.state, FirmwareState::UpToDate);
        assert_eq!(m.latest.as_deref(), Some("2025.12.5-alpha"));
        assert_eq!(m.installed_date.as_deref(), Some("Jun 25 2026"));
        assert_eq!(m.official.as_deref(), Some("2026.6.2"));
        assert!(!m.flashable);
        assert_eq!(m.note, None);
        assert_eq!(
            m.vendor_file.as_ref().unwrap().name,
            "M75 Pro II O4_GF 1811_ELRS_2025_12_4 20260625.txt"
        );
        let a = &rows[1];
        assert_eq!(a.channel_reason, Some(ChannelReason::MakersBuild));
        assert_eq!(a.state, FirmwareState::UpToDate);
        assert_eq!(a.latest.as_deref(), Some("2026.6.0-alpha"));
        // A second check reads the lists again, not the .txt files or the build.
        let w = wants(&devices, &profiles());
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("firmware")).unwrap();
        std::fs::write(
            d.path().join("firmware/latest.json"),
            serde_json::to_vec(&l).unwrap(),
        )
        .unwrap();
        let before = f.requested.lock().unwrap().len();
        check_with(&f, d.path(), Utc::now(), &w).unwrap();
        let asked: Vec<String> = f.requested.lock().unwrap()[before..].to_vec();
        assert!(
            !asked
                .iter()
                .any(|u| u == M75_TXT || u.contains("build.betaflight")),
            "{asked:?}"
        );
    }

    #[test]
    fn an_older_makers_build_has_an_update_and_a_choice_wins() {
        let old = bf_fc(
            "BETAFPVG473",
            "2025.12.3",
            "Mar 2 2026 10:00:00 1234567",
            None,
            "Meteor75 Pro II",
        );
        let (l, _) = checked(std::slice::from_ref(&old), "2026-05-15T06:14:40Z");
        let r = &statuses_with(std::slice::from_ref(&old), &l, &profiles())[0];
        assert_eq!(r.state, FirmwareState::Update);
        assert_eq!(r.latest.as_deref(), Some("2025.12.5-alpha"));
        assert!(
            r.note.as_deref().unwrap().contains("BetaFPV builds"),
            "{:?}",
            r.note
        );
        // The person picks official Betaflight: the newest release decides.
        let mut chosen = old.clone();
        chosen.firmware_channel = Some(FirmwareChannel::Official);
        let r = &statuses_with(std::slice::from_ref(&chosen), &l, &profiles())[0];
        assert_eq!(r.channel, Some(FirmwareChannel::Official));
        assert_eq!(r.channel_reason, Some(ChannelReason::Chosen));
        assert_eq!(r.latest.as_deref(), Some("2026.6.2"));
        assert_eq!(
            r.vendor.as_deref(),
            Some("BetaFPV"),
            "the choice stays offered"
        );
    }

    #[test]
    fn a_cloud_build_on_another_day_is_official_with_a_note() {
        let own = bf_fc(
            "BETAFPVG473_V2",
            "2026.6.0-alpha",
            "Jul 30 2026 01:00:00 abcdef1",
            Some(KEY),
            "Air65 II",
        );
        let (l, _) = checked(std::slice::from_ref(&own), "2026-07-30T01:00:00Z");
        let r = &statuses_with(std::slice::from_ref(&own), &l, &profiles())[0];
        assert_eq!(r.channel, Some(FirmwareChannel::Official));
        assert_eq!(r.channel_reason, Some(ChannelReason::CloudBuild));
        assert_eq!(r.state, FirmwareState::Update);
        assert_eq!(r.latest.as_deref(), Some("2026.6.2"));
        assert!(r
            .channel_note
            .as_deref()
            .unwrap()
            .contains("not BetaFPV's file"));
        // An FC with no maker row: official, no maker.
        let other = bf_fc("STM32F411", "4.5.1", "x", None, "Air65 II");
        let r = &statuses_with(std::slice::from_ref(&other), &l, &profiles())[0];
        assert_eq!(r.channel_reason, Some(ChannelReason::NoSource));
        assert_eq!(r.vendor, None);
    }
}
