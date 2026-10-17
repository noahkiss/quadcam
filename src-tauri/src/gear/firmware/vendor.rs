//! The vendor channel (design 6.4): an FC that runs its maker's own Betaflight build is
//! checked against the maker's newest file, not against official Betaflight.
//!
//! - **Source.** `bf::boards::vendor_source` names the maker's feed for an FC. BetaFPV
//!   publishes per-quad `.hex` and `.txt` pairs as attachments of a Zendesk Help Center
//!   article; the public JSON API lists them without a login. The `.txt` is a `diff all` whose
//!   version line is the build's true identity: the version label in the file name can be
//!   wrong (`2025_12_4` on a `2025.12.5-alpha` build).
//! - **Build identity.** Two builds are the same when version, git revision and build date
//!   agree (`same_build`). A comparison never uses the version number alone.
//! - **Detection** (`detect`): the person's choice; else a maker row and a cloud build key
//!   whose build date (build.betaflight.com's `submitted`) is one of the maker's file dates;
//!   else a row and no key, a maker's local build; else a row and a key with no date match,
//!   official; else official.
//!
//! Each reader is a pure function over downloaded bytes, so the tests use fixtures. The
//! network is reached only through `Fetch` (offline under cargo unless `QUADCAM_FETCH=real`).

use super::fetch_bytes;
use crate::gear::bf::boards::{BuildKind, Feed, VendorSource};
use crate::gear::bf::dump::parse_version_line;
use crate::gear::model::{FirmwareChannel, Identity};
use crate::modules::fetch::Fetch;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::path::Path;

/// The build service's answer for one key: what was built, and when.
pub fn build_url(key: &str) -> String {
    format!("https://build.betaflight.com/api/builds/{key}/json")
}

/// A Zendesk article's attachment list.
pub fn attachments_url(host: &str, article: &str) -> String {
    format!("https://{host}/api/v2/help_center/en-us/articles/{article}/attachments.json")
}

/// A build as its version line names it: `2025.12.5-alpha Jun 25 2026 / 03:24:50 (eb2bb5a33)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct BuildId {
    pub version: String,
    /// The build day, `YYYY-MM-DD`.
    #[serde(default)]
    pub date: Option<NaiveDate>,
    /// `HH:MM:SS`.
    #[serde(default)]
    pub time: Option<String>,
    /// The short git revision, or `norevision` for a local build.
    #[serde(default)]
    pub rev: Option<String>,
}

impl BuildId {
    /// True for a build made outside the cloud (git revision `norevision`).
    pub fn local(&self) -> bool {
        self.rev.as_deref() == Some("norevision")
    }

    /// `Jun 25 2026`.
    pub fn day_label(&self) -> Option<String> {
        self.date.map(|d| d.format("%b %-d %Y").to_string())
    }
}

/// A month, day and year as Betaflight prints them (`Jun 25 2026`, `Jan  1 2026`).
fn parse_day(words: &[&str]) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&words.join(" "), "%b %d %Y").ok()
}

/// The words of a build field: date, time and revision in any of the forms QuadCam keeps
/// (`Jun 25 2026 03:24:50 eb2bb5a` from MSP, `Jun 25 2026 / 03:24:50` from a version line,
/// `eb2bb5a33` alone from a dump).
fn date_time_rev(s: &str) -> (Option<NaiveDate>, Option<String>, Option<String>) {
    let words: Vec<&str> = s
        .split_whitespace()
        .map(|w| w.trim_matches(['(', ')']))
        .filter(|w| *w != "/" && !w.is_empty())
        .collect();
    let mut date = None;
    let mut time = None;
    let mut rev = None;
    let mut i = 0;
    while i < words.len() {
        if i + 3 <= words.len() {
            if let Some(d) = parse_day(&words[i..i + 3]) {
                date = Some(d);
                i += 3;
                continue;
            }
        }
        let w = words[i];
        if w.len() == 8 && w.as_bytes()[2] == b':' && w.as_bytes()[5] == b':' {
            time = Some(w.to_string());
        } else if w == "norevision" || (w.len() >= 7 && w.chars().all(|c| c.is_ascii_hexdigit())) {
            rev = Some(w.to_ascii_lowercase());
        }
        i += 1;
    }
    (date, time, rev)
}

impl BuildId {
    /// The build an FC reports: its version and its build field.
    pub fn of_identity(id: &Identity) -> Option<BuildId> {
        let version = id.version.clone().filter(|v| !v.trim().is_empty())?;
        let (date, time, rev) = id.build.as_deref().map(date_time_rev).unwrap_or_default();
        Some(BuildId {
            version,
            date,
            time,
            rev,
        })
    }
}

/// The build a maker's `.txt` (a `diff all` or `dump all`) was made from: its first
/// `# Betaflight / ...` version line. The file name's version label is not read.
pub fn parse_txt(text: &str) -> Option<BuildId> {
    text.lines().find_map(|l| {
        let t = l.trim_end_matches('\r').trim();
        let v = parse_version_line(t.strip_prefix('#')?)?;
        let (date, time, _) = v
            .build_date
            .as_deref()
            .map(date_time_rev)
            .unwrap_or_default();
        Some(BuildId {
            version: v.version,
            date,
            time,
            rev: v.git.map(|g| g.to_ascii_lowercase()),
        })
    })
}

/// True when two builds are the same: the same version, and the same revision (one may be
/// shorter) and day where both are known. Two local builds (`norevision`) need the same day.
pub fn same_build(a: &BuildId, b: &BuildId) -> bool {
    if !a.version.trim().eq_ignore_ascii_case(b.version.trim()) {
        return false;
    }
    let rev = match (&a.rev, &b.rev) {
        (Some(x), Some(y)) => Some(x.starts_with(y.as_str()) || y.starts_with(x.as_str())),
        _ => None,
    };
    let day = match (a.date, b.date) {
        (Some(x), Some(y)) => Some(x == y),
        _ => None,
    };
    let local = a.local() || b.local();
    match (rev, day) {
        (Some(false), _) | (_, Some(false)) => false,
        (Some(true), Some(true)) => true,
        (Some(true), None) => !local,
        (None, Some(true)) => true,
        (None, None) => false,
    }
}

/// One file of a maker's feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct VendorFile {
    pub name: String,
    pub url: String,
    /// The `YYYYMMDD` at the end of the name.
    #[serde(default)]
    pub date: Option<NaiveDate>,
    /// From the `.txt`'s version line; None for a file QuadCam did not read.
    #[serde(default)]
    pub build: Option<BuildId>,
}

/// A maker's article as last read: its `.txt` files with their builds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct VendorFeed {
    pub feed: Feed,
    pub checked_at: DateTime<Utc>,
    pub files: Vec<VendorFile>,
}

/// One attachment in a Zendesk `attachments.json` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub url: String,
}

/// The attachments of a Zendesk article (`article_attachments`: `file_name`, `content_url`).
pub fn parse_attachments(json: &[u8]) -> Result<Vec<Attachment>> {
    let v: Value = serde_json::from_slice(json).context("The attachment list is not JSON")?;
    let list = v["article_attachments"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("The answer has no article_attachments"))?;
    Ok(list
        .iter()
        .filter_map(|a| {
            Some(Attachment {
                name: a["file_name"].as_str()?.to_string(),
                url: a["content_url"].as_str()?.to_string(),
            })
        })
        .collect())
}

/// The `YYYYMMDD` before a file's extension: `..._2025_12_4 20260625.txt` is 2026-06-25.
pub fn file_date(name: &str) -> Option<NaiveDate> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let last = stem.rsplit([' ', '_']).next()?;
    (last.len() == 8)
        .then(|| NaiveDate::parse_from_str(last, "%Y%m%d").ok())
        .flatten()
}

/// The newest of a source's files: the `.txt` files with its prefix and build kind, by the
/// build's day, then the file's date.
pub fn newest<'a>(src: &VendorSource, files: &'a [VendorFile]) -> Option<&'a VendorFile> {
    files
        .iter()
        .filter(|f| f.name.starts_with(&src.prefix))
        .filter(|f| {
            f.build.as_ref().is_some_and(|b| match src.build {
                BuildKind::Cloud => !b.local(),
                BuildKind::Local => b.local(),
            })
        })
        .max_by_key(|f| (f.build.as_ref().and_then(|b| b.date), f.date))
}

/// Every day a source's builds carry: the files' name dates and their builds' days, of
/// either build kind. A cloud build made on one of these days is the maker's.
pub fn days(src: &VendorSource, files: &[VendorFile]) -> Vec<NaiveDate> {
    let mut d: Vec<NaiveDate> = files
        .iter()
        .filter(|f| f.name.starts_with(&src.prefix))
        .flat_map(|f| [f.date, f.build.as_ref().and_then(|b| b.date)])
        .flatten()
        .collect();
    d.sort();
    d.dedup();
    d
}

/// Reads a feed: the attachment list, then each `.txt` with one of `prefixes` that `old`
/// does not hold already (an attachment's URL names one file, which never changes).
pub fn read_feed(
    fetch: &dyn Fetch,
    feed: &Feed,
    prefixes: &[String],
    old: Option<&VendorFeed>,
    scratch: &Path,
    now: DateTime<Utc>,
) -> Result<VendorFeed> {
    let Feed::Zendesk { host, article } = feed;
    let list = parse_attachments(&fetch_bytes(
        fetch,
        &attachments_url(host, article),
        scratch,
    )?)?;
    let mut files = Vec::new();
    for a in list {
        if !a.name.to_ascii_lowercase().ends_with(".txt")
            || !prefixes.iter().any(|p| a.name.starts_with(p.as_str()))
        {
            continue;
        }
        let known = old
            .and_then(|o| o.files.iter().find(|f| f.url == a.url))
            .and_then(|f| f.build.clone());
        let build = match known {
            Some(b) => Some(b),
            None => {
                let bytes = fetch_bytes(fetch, &a.url, scratch)?;
                parse_txt(&String::from_utf8_lossy(&bytes))
            }
        };
        files.push(VendorFile {
            date: file_date(&a.name),
            name: a.name,
            url: a.url,
            build,
        });
    }
    Ok(VendorFeed {
        feed: feed.clone(),
        checked_at: now,
        files,
    })
}

/// A cloud build as the build service reports it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct CloudBuild {
    pub key: String,
    pub release: Option<String>,
    pub target: Option<String>,
    /// When the build was asked for: the build's day.
    pub submitted: Option<DateTime<Utc>>,
}

/// build.betaflight.com's `/api/builds/<key>/json`.
pub fn parse_build(key: &str, json: &[u8]) -> Result<CloudBuild> {
    let v: Value = serde_json::from_slice(json).context("The build answer is not JSON")?;
    if !v.is_object() {
        anyhow::bail!("The build answer is not an object");
    }
    Ok(CloudBuild {
        key: key.to_string(),
        release: v["request"]["release"].as_str().map(str::to_string),
        target: v["request"]["target"]
            .as_str()
            .or(v["config"]["target"].as_str())
            .map(str::to_string),
        submitted: v["submitted"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc)),
    })
}

/// How QuadCam came to an FC's channel.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChannelReason {
    /// The person set it on the device.
    Chosen,
    /// The FC's cloud build was made on a day the maker's files carry, or its version line
    /// is one of the maker's.
    MakersBuild,
    /// The maker publishes builds for this FC and the FC has no cloud build key.
    LocalBuild,
    /// The maker publishes builds for this FC; its files or the FC's build are not read yet.
    NotChecked,
    /// The FC runs a cloud build the maker did not publish.
    CloudBuild,
    /// QuadCam knows no maker's builds for this FC.
    NoSource,
}

/// An FC's channel and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub channel: FirmwareChannel,
    pub reason: ChannelReason,
}

/// The channel of an FC (see the module docs). `files` is the source's feed as last read
/// (None before the first check); `cloud` the FC's build key as the build service reported
/// it (None when it has no key or it was not read).
pub fn detect(
    chosen: Option<FirmwareChannel>,
    src: Option<&VendorSource>,
    fc: &Identity,
    files: Option<&[VendorFile]>,
    cloud: Option<&CloudBuild>,
) -> Detected {
    let d = |channel, reason| Detected { channel, reason };
    match (chosen, src) {
        (Some(FirmwareChannel::Vendor), None) => {
            return d(FirmwareChannel::Official, ChannelReason::NoSource)
        }
        (Some(c), _) => return d(c, ChannelReason::Chosen),
        (None, None) => return d(FirmwareChannel::Official, ChannelReason::NoSource),
        (None, Some(_)) => {}
    }
    let src = src.expect("matched above");
    if fc.build_key().is_none() {
        return d(FirmwareChannel::Vendor, ChannelReason::LocalBuild);
    }
    let Some(files) = files else {
        return d(FirmwareChannel::Vendor, ChannelReason::NotChecked);
    };
    let own = BuildId::of_identity(fc);
    let ours = |f: &VendorFile| f.name.starts_with(&src.prefix);
    if own.as_ref().is_some_and(|o| {
        files
            .iter()
            .filter(|f| ours(f))
            .filter_map(|f| f.build.as_ref())
            .any(|b| same_build(o, b))
    }) {
        return d(FirmwareChannel::Vendor, ChannelReason::MakersBuild);
    }
    match cloud.and_then(|c| c.submitted) {
        Some(t) if days(src, files).contains(&t.date_naive()) => {
            d(FirmwareChannel::Vendor, ChannelReason::MakersBuild)
        }
        Some(_) => d(FirmwareChannel::Official, ChannelReason::CloudBuild),
        None => d(FirmwareChannel::Vendor, ChannelReason::NotChecked),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::bf::boards::vendor_source;
    use crate::gear::firmware::FixtureFetch;

    pub(crate) const KEY: &str = "00112233445566778899aabbccddeeff";

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/firmware")
                .join(name),
        )
        .unwrap()
    }

    fn m75_fc(build: &str, key: Option<&str>) -> Identity {
        Identity {
            board: Some("BETAFPVG473".into()),
            version: Some("2025.12.5-alpha".into()),
            build: Some(build.into()),
            manufacturer_id: Some("BEFH".into()),
            build_key: key.map(str::to_string),
            ..Identity::default()
        }
    }

    fn m75_src() -> VendorSource {
        vendor_source(&m75_fc("", None), &["Meteor75 Pro II"]).unwrap()
    }

    fn m75_files() -> Vec<VendorFile> {
        vec![VendorFile {
            name: "M75 Pro II O4_GF 1811_ELRS_2025_12_4 20260625.txt".into(),
            url: "https://example.invalid/1".into(),
            date: NaiveDate::from_ymd_opt(2026, 6, 25),
            build: parse_txt(
                &String::from_utf8(fixture("betafpv-m75-o4-elrs-20260625.txt")).unwrap(),
            ),
        }]
    }

    fn cloud(day: &str) -> CloudBuild {
        CloudBuild {
            key: KEY.into(),
            release: Some("2025.12.4".into()),
            target: Some("BETAFPVG473".into()),
            submitted: Some(format!("{day}T03:24:39Z").parse().unwrap()),
        }
    }

    #[test]
    fn the_txt_names_the_build_not_the_file_label() {
        let b = parse_txt(&String::from_utf8(fixture("betafpv-m75-o4-elrs-20260625.txt")).unwrap())
            .unwrap();
        // The file says 2025_12_4; the build is 2025.12.5-alpha.
        assert_eq!(b.version, "2025.12.5-alpha");
        assert_eq!(b.date, NaiveDate::from_ymd_opt(2026, 6, 25));
        assert_eq!(b.time.as_deref(), Some("03:24:50"));
        assert_eq!(b.rev.as_deref(), Some("eb2bb5a33"));
        assert_eq!(b.day_label().as_deref(), Some("Jun 25 2026"));
        assert!(!b.local());
        let l =
            parse_txt(&String::from_utf8(fixture("betafpv-a65-f-20260421.txt")).unwrap()).unwrap();
        assert_eq!(l.version, "2026.6.0-alpha");
        assert_eq!(l.date, NaiveDate::from_ymd_opt(2026, 4, 15));
        assert!(l.local());
        assert!(parse_txt("no version here\n# diff all\n").is_none());
        assert_eq!(
            file_date("M75 Pro II O4_GF 1811_ELRS_2025_12_4 20260625.txt"),
            NaiveDate::from_ymd_opt(2026, 6, 25)
        );
        assert_eq!(file_date("主图.jpg"), None);
    }

    #[test]
    fn an_fc_build_reads_in_every_form() {
        let msp = BuildId::of_identity(&m75_fc("Jun 25 2026 03:24:50 eb2bb5a", None)).unwrap();
        assert_eq!(msp.date, NaiveDate::from_ymd_opt(2026, 6, 25));
        assert_eq!(msp.rev.as_deref(), Some("eb2bb5a"));
        let padded = BuildId::of_identity(&m75_fc("Jan  1 2026 00:00:00 abcdef0", None)).unwrap();
        assert_eq!(padded.date, NaiveDate::from_ymd_opt(2026, 1, 1));
        let dump = BuildId::of_identity(&m75_fc("eb2bb5a33", None)).unwrap();
        assert_eq!((dump.date, dump.rev.as_deref()), (None, Some("eb2bb5a33")));
        let file = m75_files()[0].build.clone().unwrap();
        assert!(same_build(&msp, &file));
        assert!(
            same_build(&dump, &file),
            "revision alone, not a local build"
        );
        let other = BuildId::of_identity(&m75_fc("Jul 30 2026 01:00:00 1234567", None)).unwrap();
        assert!(!same_build(&other, &file));
        let mut newer = file.clone();
        newer.version = "2026.6.2".into();
        assert!(!same_build(&msp, &newer), "the version must agree");
        // Two local builds need the day.
        let local = BuildId {
            version: "2026.6.0-alpha".into(),
            rev: Some("norevision".into()),
            ..BuildId::default()
        };
        assert!(!same_build(&local, &local.clone()));
        let dated = BuildId {
            date: NaiveDate::from_ymd_opt(2026, 4, 15),
            ..local.clone()
        };
        assert!(same_build(&dated, &dated.clone()));
    }

    #[test]
    fn the_zendesk_answer_lists_its_files() {
        let a = parse_attachments(&fixture("zendesk-attachments-meteor75-o4.json")).unwrap();
        assert_eq!(a.len(), 5);
        assert!(a.iter().any(
            |x| x.name == "M75 Pro II O4_GF 1811_ELRS_2025_12_4 20260625.txt"
                && x.url == "https://support.betafpv.com/hc/article_attachments/59863253804825"
        ));
        let b = parse_attachments(&fixture("zendesk-attachments-air65-ii.json")).unwrap();
        assert_eq!(b.iter().filter(|x| x.name.ends_with(".txt")).count(), 6);
        assert!(parse_attachments(b"{}").is_err());
        assert!(parse_attachments(b"<html>").is_err());
    }

    #[test]
    fn a_feed_reads_only_its_own_txt_files_once() {
        let d = tempfile::tempdir().unwrap();
        let f = FixtureFetch::new();
        let feed = Feed::Zendesk {
            host: "support.betafpv.com".into(),
            article: "54654422544409".into(),
        };
        f.serve(
            &attachments_url("support.betafpv.com", "54654422544409"),
            fixture("zendesk-attachments-air65-ii.json"),
        );
        f.serve(
            "https://support.betafpv.com/hc/article_attachments/58300171193113",
            fixture("betafpv-a65-f-20260515.txt"),
        );
        f.serve(
            "https://support.betafpv.com/hc/article_attachments/58300189274265",
            fixture("betafpv-a65-f-20260421.txt"),
        );
        let prefixes = vec!["A65 II F_0702_Freestyle".to_string()];
        let now = Utc::now();
        let r = read_feed(&f, &feed, &prefixes, None, d.path(), now).unwrap();
        assert_eq!(r.files.len(), 2, "{:?}", r.files);
        let air = |key: Option<&str>| {
            vendor_source(
                &Identity {
                    board: Some("BETAFPVG473_V2".into()),
                    build_key: key.map(str::to_string),
                    ..Identity::default()
                },
                &["Air65 II"],
            )
            .unwrap()
        };
        let cloud_src = air(Some(KEY));
        let n = newest(&cloud_src, &r.files).unwrap();
        assert_eq!(n.date, NaiveDate::from_ymd_opt(2026, 5, 15));
        assert_eq!(n.build.as_ref().unwrap().rev.as_deref(), Some("e92c10887"));
        let local = newest(&air(None), &r.files).unwrap();
        assert!(local.build.as_ref().unwrap().local());
        assert_eq!(
            days(&cloud_src, &r.files),
            vec![
                NaiveDate::from_ymd_opt(2026, 4, 15).unwrap(),
                NaiveDate::from_ymd_opt(2026, 4, 21).unwrap(),
                NaiveDate::from_ymd_opt(2026, 5, 15).unwrap()
            ]
        );
        // A second read takes the builds it knows from the old answer.
        let again = read_feed(&f, &feed, &prefixes, Some(&r), d.path(), now).unwrap();
        assert_eq!(again.files, r.files);
        assert_eq!(
            f.requested.lock().unwrap().len(),
            4,
            "the list twice, each .txt once"
        );
    }

    #[test]
    fn the_build_service_answer() {
        let j = serde_json::json!({
            "request": {"release": "2025.12.4", "target": "BETAFPVG473"},
            "config": {"target": "BETAFPVG473", "manufacturer": "BEFH", "configuration": []},
            "submitted": "2026-06-25T03:24:39.7400567Z"
        });
        let b = parse_build(KEY, j.to_string().as_bytes()).unwrap();
        assert_eq!(b.release.as_deref(), Some("2025.12.4"));
        assert_eq!(b.target.as_deref(), Some("BETAFPVG473"));
        assert_eq!(
            b.submitted.unwrap().date_naive(),
            NaiveDate::from_ymd_opt(2026, 6, 25).unwrap()
        );
        assert!(parse_build(KEY, b"[]").is_err());
        assert!(parse_build(KEY, b"nope").is_err());
    }

    #[test]
    fn detection_takes_five_paths() {
        let src = m75_src();
        let files = m75_files();
        let other_build = "Jul 30 2026 01:00:00 1234567";
        let keyed = m75_fc(other_build, Some(KEY));
        // 1. The person's choice wins; vendor without a source is official.
        let d = detect(
            Some(FirmwareChannel::Official),
            Some(&src),
            &keyed,
            Some(&files),
            None,
        );
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Official, ChannelReason::Chosen)
        );
        let d = detect(Some(FirmwareChannel::Vendor), None, &keyed, None, None);
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Official, ChannelReason::NoSource)
        );
        // 2. A cloud build made on a day the maker's files carry.
        let d = detect(
            None,
            Some(&src),
            &keyed,
            Some(&files),
            Some(&cloud("2026-06-25")),
        );
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Vendor, ChannelReason::MakersBuild)
        );
        // ... or the FC's version line is the maker's own, without the build service.
        let same = m75_fc("Jun 25 2026 03:24:50 eb2bb5a", Some(KEY));
        let d = detect(None, Some(&src), &same, Some(&files), None);
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Vendor, ChannelReason::MakersBuild)
        );
        // 3. No build key: the maker's local build.
        for key in [None, Some("")] {
            let d = detect(
                None,
                Some(&src),
                &m75_fc(other_build, key),
                Some(&files),
                None,
            );
            assert_eq!(
                (d.channel, d.reason),
                (FirmwareChannel::Vendor, ChannelReason::LocalBuild)
            );
        }
        // 4. A cloud build on another day: official.
        let d = detect(
            None,
            Some(&src),
            &keyed,
            Some(&files),
            Some(&cloud("2026-07-30")),
        );
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Official, ChannelReason::CloudBuild)
        );
        // Not read yet: the maker's, to be checked.
        let d = detect(None, Some(&src), &keyed, None, None);
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Vendor, ChannelReason::NotChecked)
        );
        let d = detect(None, Some(&src), &keyed, Some(&files), None);
        assert_eq!(d.reason, ChannelReason::NotChecked);
        // 5. No row: official.
        let d = detect(None, None, &keyed, None, None);
        assert_eq!(
            (d.channel, d.reason),
            (FirmwareChannel::Official, ChannelReason::NoSource)
        );
    }
}
