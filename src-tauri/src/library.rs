//! The library: the folder imports land in, and the clips in it.
//!
//! The files are the library. Every clip quadcam writes carries its details as QuickTime
//! metadata (see `qtmeta` and the `KEY_*` names here): date, name, note, place, aircraft,
//! moments, keep ranges, flight numbers, rating and flag. The index in
//! `<library>/.quadcam/index.json` is a cache of those details so the app opens fast;
//! `rebuild` makes it again from the files alone. The one thing only the index holds is a
//! clip's unsaved cut ranges.
//!
//! A clip's identity is the content fingerprint of its DVR source (`KEY_SOURCE`), never a
//! file name: DVRs restart at PICT0001 after every format. A file quadcam did not write
//! (an older export, adopted) is known by a hash of its first megabyte, which a metadata
//! rewrite never touches because `moov` sits at the end of the file.

use crate::media::{self, Tools};
use crate::metadata::{FlightStats, Location};
use crate::moments::{Moment, MomentKind, Source, Span};
use crate::qtmeta::{self, Item};
use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const INDEX_DIR: &str = ".quadcam";
pub const INDEX_FILE: &str = "index.json";
pub const INDEX_VERSION: u32 = 1;
/// Folder for kept DVR originals, inside each day folder.
pub const ORIGINALS: &str = "originals";

pub const KEY_SOURCE: &str = "app.quadcam.source";
pub const KEY_DVR: &str = "app.quadcam.dvr";
pub const KEY_IMPORT: &str = "app.quadcam.import";
pub const KEY_PLACE: &str = "app.quadcam.place";
pub const KEY_PROFILE: &str = "app.quadcam.profile";
pub const KEY_MOMENTS: &str = "app.quadcam.moments";
pub const KEY_KEEP: &str = "app.quadcam.keep";
pub const KEY_STATS: &str = "app.quadcam.stats";
pub const KEY_CUT: &str = "app.quadcam.cut";
pub const KEY_DETACHED: &str = "app.quadcam.detached";
pub const KEY_RATING: &str = "app.quadcam.rating";
pub const KEY_FLAG: &str = "app.quadcam.flag";
pub const KEY_PHOTOS: &str = "app.quadcam.photos";
pub const KEY_AIRCRAFT: &str = "app.quadcam.aircraft";
const QT: &str = "com.apple.quicktime.";

// ---------- layout ----------

/// Where a clip goes inside the library folder.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// `YYYY/YYYY-MM-DD/`
    #[default]
    YearDay,
    /// `YYYY-MM-DD/`
    Day,
    /// Every file in the library folder itself.
    Flat,
}

/// A place name made safe for a folder name: no slashes or colons, single spaces, at most
/// 60 characters.
pub fn folder_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == ':' || c == '\\' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let mut out = words.join(" ");
    out = out.trim_start_matches('.').trim().to_string();
    out.chars().take(60).collect::<String>().trim().to_string()
}

/// The folder for one day's clips. With `place`, the day folder gets the place name.
pub fn day_dir(root: &Path, layout: Layout, date: NaiveDate, place: Option<&str>) -> PathBuf {
    let mut day = date.format("%Y-%m-%d").to_string();
    if let Some(p) = place.map(folder_safe).filter(|p| !p.is_empty()) {
        day = format!("{day} {p}");
    }
    match layout {
        Layout::YearDay => root.join(format!("{:04}", date.year())).join(day),
        Layout::Day => root.join(day),
        Layout::Flat => root.to_path_buf(),
    }
}

/// The example under the layout setting: `2026/2026-09-27 Backyard/2026-09-27_name.mp4`.
pub fn example_path(
    layout: Layout,
    date: NaiveDate,
    place: Option<&str>,
    stem: &str,
    ext: &str,
) -> String {
    let dir = day_dir(Path::new(""), layout, date, place);
    dir.join(format!("{stem}.{ext}"))
        .to_string_lossy()
        .to_string()
}

// ---------- index ----------

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    #[default]
    None,
    Pick,
    Reject,
}

impl Flag {
    pub fn as_str(self) -> &'static str {
        match self {
            Flag::None => "",
            Flag::Pick => "pick",
            Flag::Reject => "reject",
        }
    }
    pub fn parse(s: &str) -> Flag {
        match s.trim() {
            "pick" => Flag::Pick,
            "reject" => Flag::Reject,
            _ => Flag::None,
        }
    }
}

/// A cut written as its own file next to its clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibCut {
    /// Relative to the library folder.
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub size: u64,
}

/// One clip in the library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibClip {
    pub id: String,
    /// Relative to the library folder.
    pub path: PathBuf,
    /// The short name; empty when the clip was imported without one.
    pub title: String,
    pub note: String,
    pub date: NaiveDate,
    /// `HH:MM`, when the creation date carries a time.
    #[serde(default)]
    pub time: Option<String>,
    pub duration: f64,
    pub size: u64,
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub flag: Flag,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
    /// Profile name, else the aircraft written in the file.
    #[serde(default)]
    pub aircraft: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// Radio-log moments, in clip seconds.
    #[serde(default)]
    pub moments: Vec<Moment>,
    /// Ranges with a picture; empty when the clip has no dead air (or was never scanned).
    #[serde(default)]
    pub keep: Vec<Span>,
    #[serde(default)]
    pub stats: Option<FlightStats>,
    #[serde(default)]
    pub cuts: Vec<LibCut>,
    /// Cut ranges set in the library but not written yet. Only the index holds these.
    #[serde(default)]
    pub pending_cuts: Vec<Span>,
    #[serde(default)]
    pub in_photos: bool,
    /// The kept DVR original, relative to the library folder.
    #[serde(default)]
    pub original: Option<PathBuf>,
    /// The DVR file name it came from.
    #[serde(default)]
    pub dvr: Option<String>,
    /// The import it came in with (`YYYYMMDD-HHMMSS`).
    #[serde(default)]
    pub import: Option<String>,
    /// Set for a cut file kept on its own after its cut was removed: (source id, range).
    #[serde(default)]
    pub cut_of: Option<(String, Span)>,
}

impl LibClip {
    /// The name to show: the title, else the file name without the date.
    pub fn display_name(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        let stem = self
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let rest = stem
            .get(11..)
            .filter(|_| stem.get(..10).and_then(parse_date).is_some())
            .unwrap_or(&stem);
        rest.replace(['-', '_'], " ").trim().to_string()
    }

    /// Every file of this clip: the clip, its cuts and its original.
    pub fn files(&self, root: &Path) -> Vec<PathBuf> {
        let mut v = vec![root.join(&self.path)];
        v.extend(self.cuts.iter().map(|c| root.join(&c.path)));
        v.extend(self.original.iter().map(|o| root.join(o)));
        v
    }

    pub fn bytes(&self) -> u64 {
        self.size + self.cuts.iter().map(|c| c.size).sum::<u64>()
    }

    /// Seconds with a picture: the keep ranges, else the whole clip.
    pub fn flying(&self) -> f64 {
        if self.keep.is_empty() {
            self.duration
        } else {
            self.keep.iter().map(Span::secs).sum()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Index {
    pub version: u32,
    /// The newest import's id; its clips are "Last import".
    #[serde(default)]
    pub last_import: Option<String>,
    pub clips: Vec<LibClip>,
}

pub fn index_path(root: &Path) -> PathBuf {
    root.join(INDEX_DIR).join(INDEX_FILE)
}

impl Index {
    pub fn load(root: &Path) -> Result<Option<Index>> {
        let p = index_path(root);
        if !p.is_file() {
            return Ok(None);
        }
        let ix: Index = serde_json::from_slice(&std::fs::read(&p)?)
            .with_context(|| format!("parsing {}", p.display()))?;
        if ix.version != INDEX_VERSION {
            return Ok(None);
        }
        Ok(Some(ix))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let p = index_path(root);
        std::fs::create_dir_all(p.parent().unwrap())?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&LibClip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut LibClip> {
        self.clips.iter_mut().find(|c| c.id == id)
    }

    /// Newest first: date, then time, then name.
    pub fn sort(&mut self) {
        self.clips.sort_by(|a, b| {
            b.date
                .cmp(&a.date)
                .then_with(|| b.time.cmp(&a.time))
                .then_with(|| a.path.cmp(&b.path))
        });
    }

    /// Puts `clip` in the index, replacing the entry with its id. Unsaved cuts survive.
    pub fn upsert(&mut self, mut clip: LibClip) {
        if let Some(old) = self.get_mut(&clip.id) {
            if clip.pending_cuts.is_empty() {
                clip.pending_cuts = std::mem::take(&mut old.pending_cuts);
            }
            *old = clip;
        } else {
            self.clips.push(clip);
        }
        self.sort();
    }
}

// ---------- reading files ----------

/// Moments as written into a file: kind, start, end, score.
#[derive(Serialize, Deserialize)]
struct MomentLite {
    k: MomentKind,
    s: f64,
    e: f64,
    c: f64,
}

pub fn moments_value(moments: &[Moment]) -> String {
    let lite: Vec<MomentLite> = moments
        .iter()
        .filter(|m| m.kind != MomentKind::DeadAir)
        .map(|m| MomentLite {
            k: m.kind,
            s: (m.start * 10.0).round() / 10.0,
            e: (m.end * 10.0).round() / 10.0,
            c: (m.score * 100.0).round() / 100.0,
        })
        .collect();
    if lite.is_empty() {
        return String::new();
    }
    serde_json::to_string(&lite).unwrap_or_default()
}

fn parse_moments(s: &str) -> Vec<Moment> {
    serde_json::from_str::<Vec<MomentLite>>(s)
        .unwrap_or_default()
        .into_iter()
        .map(|m| Moment {
            kind: m.k,
            start: m.s,
            end: m.e,
            score: m.c,
            source: Source::RadioLog,
            detail: String::new(),
        })
        .collect()
}

pub fn spans_value(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|s| format!("{:.1}-{:.1}", s.start, s.end))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_spans(s: &str) -> Vec<Span> {
    s.split(',')
        .filter_map(|p| {
            let (a, b) = p.trim().split_once('-')?;
            Some(Span {
                start: a.trim().parse().ok()?,
                end: b.trim().parse().ok()?,
            })
        })
        .collect()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}

/// A hash of the first megabyte before `moov`: the identity of a file quadcam did not
/// write. Metadata rewrites change only `moov`, so the identity holds.
pub fn head_id(path: &Path) -> Result<String> {
    use std::hash::{Hash, Hasher};
    let limit = qtmeta::moov_offset(path)
        .ok()
        .filter(|&o| o > 0)
        .unwrap_or(u64::MAX)
        .min(1 << 20);
    let mut f = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(limit as usize);
    (&mut f).take(limit).read_to_end(&mut buf)?;
    let mut h = std::hash::DefaultHasher::new();
    buf.hash(&mut h);
    Ok(format!("h{:016x}", h.finish()))
}

/// What one media file in the library is.
#[derive(Debug, Clone)]
pub enum Found {
    Clip(Box<LibClip>),
    /// A cut file: its source id (if written), range, and the clip stem it belongs to.
    Cut {
        source: Option<String>,
        parent_stem: String,
        cut: LibCut,
    },
}

fn is_media(p: &Path) -> bool {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    if name.starts_with('.') {
        return false;
    }
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| e == "mp4" || e == "mov")
}

/// The `_cutN` suffix of a stem: (the clip's stem, N).
fn cut_stem(stem: &str) -> Option<(&str, u32)> {
    let at = stem.rfind("_cut")?;
    let n: u32 = stem[at + 4..].split('-').next()?.parse().ok()?;
    Some((&stem[..at], n))
}

/// Reads one media file in the library. `rel` is its path relative to the library folder.
pub fn read_file(root: &Path, rel: &Path) -> Result<Found> {
    let path = root.join(rel);
    let (items, duration) = qtmeta::read_info(&path)?;
    let get = |k: &str| qtmeta::get(&items, k).map(str::to_string);
    let size = path.metadata()?.len();
    let stem = rel
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let source = get(KEY_SOURCE).filter(|s| !s.is_empty());
    let detached = get(KEY_DETACHED).is_some_and(|v| v == "1");
    let desc = get(&format!("{QT}description")).unwrap_or_default();
    // A cut: quadcam's own key, else the description an older quadcam wrote, else the name.
    let cut_range = get(KEY_CUT)
        .and_then(|v| parse_spans(&v).into_iter().next())
        .or_else(|| {
            let (_, r) = desc.rsplit_once("; cut ")?;
            parse_spans(r.trim_end_matches(" s").trim())
                .into_iter()
                .next()
        });
    if !detached {
        if let Some((parent, _)) = cut_stem(&stem) {
            let span = cut_range.unwrap_or(Span {
                start: 0.0,
                end: duration.unwrap_or(0.0),
            });
            return Ok(Found::Cut {
                source,
                parent_stem: parent.to_string(),
                cut: LibCut {
                    path: rel.to_path_buf(),
                    start: span.start,
                    end: span.end,
                    size,
                },
            });
        }
    }
    let created = get(&format!("{QT}creationdate")).unwrap_or_default();
    let mtime_date = || {
        path.metadata()
            .and_then(|m| m.modified())
            .ok()
            .map(|t| chrono::DateTime::<chrono::Local>::from(t).date_naive())
    };
    let date = parse_date(&created)
        .or_else(|| parse_date(&stem))
        .or_else(mtime_date)
        .unwrap_or_default();
    let time = created
        .get(11..16)
        .filter(|t| t.len() == 5 && t.as_bytes()[2] == b':')
        .map(str::to_string);
    let location = get(&format!("{QT}location.ISO6709"))
        .and_then(|v| media::parse_iso6709(&v))
        .map(|(lat, lon)| Location {
            lat,
            lon,
            name: get(KEY_PLACE).filter(|p| !p.is_empty()),
        });
    let dvr = get(KEY_DVR).or_else(|| {
        desc.strip_prefix("DVR ")
            .and_then(|r| r.split(';').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    let id = match &source {
        Some(s) if detached => match cut_range {
            Some(r) => format!("{s}@{:.1}-{:.1}", r.start, r.end),
            None => head_id(&path)?,
        },
        Some(s) => s.clone(),
        None => head_id(&path)?,
    };
    let original = path.parent().and_then(|dir| {
        let o = dir.join(ORIGINALS);
        ["avi", "AVI"]
            .iter()
            .map(|e| o.join(format!("{stem}.{e}")))
            .find(|p| p.is_file())
            .and_then(|p| p.strip_prefix(root).ok().map(Path::to_path_buf))
    });
    Ok(Found::Clip(Box::new(LibClip {
        id,
        path: rel.to_path_buf(),
        title: get(&format!("{QT}title")).unwrap_or_default(),
        note: get(&format!("{QT}comment")).unwrap_or_default(),
        date,
        time,
        duration: duration.unwrap_or(0.0),
        size,
        rating: get(KEY_RATING)
            .and_then(|r| r.trim().parse::<u8>().ok())
            .unwrap_or(0)
            .min(5),
        flag: Flag::parse(&get(KEY_FLAG).unwrap_or_default()),
        place: get(KEY_PLACE).filter(|p| !p.is_empty()),
        location,
        aircraft: get(KEY_PROFILE)
            .or_else(|| get(KEY_AIRCRAFT))
            .filter(|a| !a.is_empty()),
        keywords: get(&format!("{QT}keywords"))
            .map(|k| {
                k.split(',')
                    .map(|w| w.trim().to_string())
                    .filter(|w| !w.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        author: get(&format!("{QT}author")).filter(|a| !a.is_empty()),
        moments: get(KEY_MOMENTS)
            .map(|m| parse_moments(&m))
            .unwrap_or_default(),
        keep: get(KEY_KEEP).map(|k| parse_spans(&k)).unwrap_or_default(),
        stats: get(KEY_STATS).and_then(|s| serde_json::from_str(&s).ok()),
        cuts: Vec::new(),
        pending_cuts: Vec::new(),
        in_photos: get(KEY_PHOTOS).is_some_and(|v| v == "1"),
        original,
        dvr,
        import: get(KEY_IMPORT).filter(|i| !i.is_empty()),
        cut_of: if detached {
            source.clone().zip(cut_range)
        } else {
            None
        },
    })))
}

/// Media files under the library folder, relative to it. Skips hidden files and folders,
/// `originals/`, and half-written `.part` files.
pub fn media_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if name != ORIGINALS {
                    stack.push(p);
                }
            } else if ft.is_file() && is_media(&p) {
                if let Ok(rel) = p.strip_prefix(root) {
                    out.push(rel.to_path_buf());
                }
            }
        }
    }
    out.sort();
    out
}

/// True when the folder holds media files but no index: an existing export folder to adopt.
pub fn needs_scan(root: &Path) -> bool {
    !index_path(root).is_file() && !media_files(root).is_empty()
}

/// Builds the index from the files alone. Nothing is moved or written to the files.
/// `old` keeps its unsaved cuts. Files that cannot be read are listed in the second value.
pub fn rebuild(root: &Path, old: Option<&Index>) -> (Index, Vec<String>) {
    let mut clips: Vec<LibClip> = Vec::new();
    let mut cuts = Vec::new();
    let mut problems = Vec::new();
    for rel in media_files(root) {
        match read_file(root, &rel) {
            Ok(Found::Clip(c)) => clips.push(*c),
            Ok(Found::Cut {
                source,
                parent_stem,
                cut,
            }) => cuts.push((source, parent_stem, cut)),
            Err(e) => problems.push(format!("{}: {e:#}", rel.display())),
        }
    }
    // Two files with one id (a copy of a clip): keep the first, list the other.
    let mut seen = HashSet::new();
    clips.retain(|c| {
        let fresh = seen.insert(c.id.clone());
        if !fresh {
            problems.push(format!(
                "{}: the same clip as another file; left out",
                c.path.display()
            ));
        }
        fresh
    });
    for (source, parent_stem, cut) in cuts {
        let dir = cut.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let by_stem = |c: &LibClip| {
            c.path.parent().map(Path::to_path_buf).unwrap_or_default() == dir
                && c.path.file_stem().unwrap_or_default().to_string_lossy() == parent_stem
        };
        let at = clips
            .iter()
            .position(|c| source.as_deref() == Some(c.id.as_str()) && by_stem(c))
            .or_else(|| clips.iter().position(by_stem))
            .or_else(|| {
                source
                    .as_deref()
                    .and_then(|s| clips.iter().position(|c| c.id == s))
            });
        match at {
            Some(i) => clips[i].cuts.push(cut),
            None => problems.push(format!(
                "{}: a cut without its clip; left out",
                cut.path.display()
            )),
        }
    }
    for c in &mut clips {
        c.cuts.sort_by(|a, b| a.start.total_cmp(&b.start));
        if let Some(o) = old.and_then(|o| o.get(&c.id)) {
            c.pending_cuts = o.pending_cuts.clone();
        }
    }
    let last_import = clips.iter().filter_map(|c| c.import.clone()).max();
    let mut ix = Index {
        version: INDEX_VERSION,
        last_import,
        clips,
    };
    ix.sort();
    (ix, problems)
}

/// Re-reads one clip (and the cut files next to it) from disk.
pub fn reread(root: &Path, clip_rel: &Path, cut_rels: &[PathBuf]) -> Result<LibClip> {
    let Found::Clip(mut c) = read_file(root, clip_rel)? else {
        bail!("{} is a cut, not a clip", clip_rel.display());
    };
    for rel in cut_rels {
        if let Ok(Found::Cut { cut, .. }) = read_file(root, rel) {
            c.cuts.push(cut);
        }
    }
    c.cuts.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(*c)
}

// ---------- writing details into files ----------

/// Sets items in one file (an empty value removes the item). Keeps the file's mtime.
pub fn write_keys(path: &Path, set: &[(&str, String)]) -> Result<()> {
    qtmeta::update(path, |items: &mut Vec<Item>| {
        for (k, v) in set {
            qtmeta::set(items, k, v);
        }
    })
    .with_context(|| format!("writing details into {}", path.display()))
}

/// The library's own items for a new import, beside the Photos ones `metadata::qt_items`
/// writes.
#[allow(clippy::too_many_arguments)]
pub fn import_items(
    source: &str,
    dvr: &str,
    import: &str,
    place: Option<&str>,
    profile: Option<&str>,
    moments: &[Moment],
    keep: &[Span],
    stats: Option<&FlightStats>,
) -> Vec<Item> {
    let mut v = vec![
        (KEY_SOURCE.to_string(), source.to_string()),
        (KEY_DVR.to_string(), dvr.to_string()),
        (KEY_IMPORT.to_string(), import.to_string()),
        (KEY_PLACE.to_string(), place.unwrap_or("").to_string()),
        (KEY_PROFILE.to_string(), profile.unwrap_or("").to_string()),
        (KEY_MOMENTS.to_string(), moments_value(moments)),
        (KEY_KEEP.to_string(), spans_value(keep)),
        (
            KEY_STATS.to_string(),
            stats
                .and_then(|s| serde_json::to_string(s).ok())
                .unwrap_or_default(),
        ),
    ];
    v.retain(|(_, val)| !val.trim().is_empty());
    v
}

/// A thumbnail strip: `tiles` frames side by side, for scrubbing on hover.
pub const STRIP_TILES: u32 = 10;

pub fn make_strip(tools: &Tools, src: &Path, duration: f64, dst: &Path) -> Result<()> {
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d)?;
    }
    let rate = STRIP_TILES as f64 / duration.max(1.0);
    let tmp = dst.with_extension("part.jpg");
    let out = std::process::Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args([
            "-vf",
            &format!("fps={rate:.6},scale=192:-2,tile={STRIP_TILES}x1"),
            "-frames:v",
            "1",
            "-q:v",
            "5",
        ])
        .arg(&tmp)
        .output()
        .context("running ffmpeg for a thumbnail strip")?;
    if !out.status.success() || !tmp.is_file() {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "thumbnail strip failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// A file name for an id (ids may hold `@` and `.`).
pub fn id_file(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

// ---------- queries ----------

/// The smart groups and other ways to narrow the library.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Filter {
    /// Words that must all appear in the name, note, place, aircraft, keywords or DVR name.
    pub query: Option<String>,
    /// `all`, `last_import`, `moments`, `picks`, `rejected`, `not_in_photos`.
    pub group: Option<String>,
    pub day: Option<NaiveDate>,
    pub place: Option<String>,
    pub aircraft: Option<String>,
    pub min_rating: Option<u8>,
}

pub fn matches(ix: &Index, c: &LibClip, f: &Filter) -> bool {
    let ok_group = match f.group.as_deref().unwrap_or("all") {
        "last_import" => c.import.is_some() && c.import == ix.last_import,
        "moments" => !c.moments.is_empty(),
        "picks" => c.flag == Flag::Pick,
        "rejected" => c.flag == Flag::Reject,
        "not_in_photos" => !c.in_photos,
        _ => true,
    };
    let text = |s: &str| s.to_lowercase();
    let ok_query = f.query.as_deref().is_none_or(|q| {
        let hay = text(&format!(
            "{} {} {} {} {} {} {}",
            c.display_name(),
            c.note,
            c.place.as_deref().unwrap_or(""),
            c.aircraft.as_deref().unwrap_or(""),
            c.keywords.join(" "),
            c.dvr.as_deref().unwrap_or(""),
            c.path.display()
        ));
        q.split_whitespace().all(|w| hay.contains(&text(w)))
    });
    ok_group
        && ok_query
        && f.day.is_none_or(|d| c.date == d)
        && f.place.as_deref().is_none_or(|p| {
            c.place
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case(p))
        })
        && f.aircraft.as_deref().is_none_or(|a| {
            c.aircraft
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case(a))
        })
        && f.min_rating.is_none_or(|r| c.rating >= r)
}

/// Clip ids by source fingerprint, for "N new" on a card.
pub fn known_sources(ix: &Index) -> HashSet<String> {
    let mut s: HashSet<String> = ix.clips.iter().map(|c| c.id.clone()).collect();
    s.extend(
        ix.clips
            .iter()
            .filter_map(|c| c.cut_of.clone().map(|x| x.0)),
    );
    s
}

/// Totals for the footer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Totals {
    pub clips: usize,
    pub bytes: u64,
    pub seconds: f64,
    pub flying: f64,
}

pub fn totals(ix: &Index) -> Totals {
    Totals {
        clips: ix.clips.len(),
        bytes: ix.clips.iter().map(LibClip::bytes).sum(),
        seconds: ix.clips.iter().map(|c| c.duration).sum(),
        flying: ix.clips.iter().map(LibClip::flying).sum(),
    }
}

/// Counts by day, place and aircraft, for the sidebar.
pub fn groups(ix: &Index) -> HashMap<&'static str, Vec<(String, usize)>> {
    let mut by: HashMap<&'static str, HashMap<String, usize>> = HashMap::new();
    for c in &ix.clips {
        *by.entry("days")
            .or_default()
            .entry(c.date.to_string())
            .or_default() += 1;
        if let Some(p) = &c.place {
            *by.entry("places")
                .or_default()
                .entry(p.clone())
                .or_default() += 1;
        }
        if let Some(a) = &c.aircraft {
            *by.entry("aircraft")
                .or_default()
                .entry(a.clone())
                .or_default() += 1;
        }
    }
    by.into_iter()
        .map(|(k, m)| {
            let mut v: Vec<(String, usize)> = m.into_iter().collect();
            if k == "days" {
                v.sort_by(|a, b| b.0.cmp(&a.0));
            } else {
                v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            }
            (k, v)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn layout_paths() {
        let root = Path::new("/lib");
        let day = d("2026-09-27");
        assert_eq!(
            day_dir(root, Layout::YearDay, day, None),
            PathBuf::from("/lib/2026/2026-09-27")
        );
        assert_eq!(
            day_dir(root, Layout::YearDay, day, Some("Backyard")),
            PathBuf::from("/lib/2026/2026-09-27 Backyard")
        );
        assert_eq!(
            day_dir(root, Layout::Day, day, Some("Riverside park")),
            PathBuf::from("/lib/2026-09-27 Riverside park")
        );
        assert_eq!(
            day_dir(root, Layout::Flat, day, Some("Backyard")),
            PathBuf::from("/lib")
        );
        // A place name never makes a second folder level or a hidden folder.
        assert_eq!(
            day_dir(root, Layout::Day, day, Some("../a/b: c")),
            PathBuf::from("/lib/2026-09-27 a b c")
        );
        assert_eq!(
            day_dir(root, Layout::Day, day, Some("  ")),
            PathBuf::from("/lib/2026-09-27")
        );
        assert_eq!(
            example_path(
                Layout::YearDay,
                day,
                Some("Backyard"),
                "2026-09-27_loops",
                "mp4"
            ),
            "2026/2026-09-27 Backyard/2026-09-27_loops.mp4"
        );
        assert_eq!(
            example_path(Layout::Flat, day, None, "2026-09-27_loops", "mp4"),
            "2026-09-27_loops.mp4"
        );
    }

    #[test]
    fn cut_stems_and_values_round_trip() {
        assert_eq!(
            cut_stem("2026-09-27_loops_cut2"),
            Some(("2026-09-27_loops", 2))
        );
        assert_eq!(
            cut_stem("2026-09-27_loops_cut2-2"),
            Some(("2026-09-27_loops", 2))
        );
        assert_eq!(cut_stem("2026-09-27_loops"), None);
        let spans = vec![
            Span {
                start: 1.0,
                end: 2.5,
            },
            Span {
                start: 10.0,
                end: 20.0,
            },
        ];
        assert_eq!(parse_spans(&spans_value(&spans)), spans);
        let m = vec![Moment {
            kind: MomentKind::Flip,
            start: 61.04,
            end: 61.6,
            score: 0.923,
            source: Source::RadioLog,
            detail: "x".into(),
        }];
        let back = parse_moments(&moments_value(&m));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].kind, MomentKind::Flip);
        assert_eq!(back[0].start, 61.0);
        assert_eq!(back[0].score, 0.92);
    }

    #[test]
    fn filters_and_groups() {
        let clip = |id: &str, title: &str, day: &str| LibClip {
            id: id.into(),
            path: PathBuf::from(format!("{day}_{title}.mp4")),
            title: title.into(),
            note: String::new(),
            date: d(day),
            time: None,
            duration: 10.0,
            size: 100,
            rating: 0,
            flag: Flag::None,
            place: Some("Field".into()),
            location: None,
            aircraft: None,
            keywords: vec![],
            author: None,
            moments: vec![],
            keep: vec![],
            stats: None,
            cuts: vec![],
            pending_cuts: vec![],
            in_photos: false,
            original: None,
            dvr: Some("PICT0001.AVI".into()),
            import: Some("20260927-150000".into()),
            cut_of: None,
        };
        let mut ix = Index {
            version: INDEX_VERSION,
            last_import: Some("20260927-150000".into()),
            clips: vec![
                clip("a", "backyard loops", "2026-09-27"),
                LibClip {
                    flag: Flag::Reject,
                    import: Some("20260921-100000".into()),
                    ..clip("b", "windy one", "2026-09-21")
                },
            ],
        };
        ix.sort();
        let f = |g: &str| Filter {
            group: Some(g.into()),
            ..Default::default()
        };
        let ids = |f: &Filter| -> Vec<String> {
            ix.clips
                .iter()
                .filter(|c| matches(&ix, c, f))
                .map(|c| c.id.clone())
                .collect()
        };
        assert_eq!(ids(&f("last_import")), ["a"]);
        assert_eq!(ids(&f("rejected")), ["b"]);
        assert_eq!(ids(&f("all")), ["a", "b"]);
        let q = Filter {
            query: Some("BACKYARD loops".into()),
            ..Default::default()
        };
        assert_eq!(ids(&q), ["a"]);
        let q = Filter {
            query: Some("pict0001".into()),
            ..Default::default()
        };
        assert_eq!(ids(&q).len(), 2);
        let g = groups(&ix);
        assert_eq!(g["days"][0].0, "2026-09-27");
        assert_eq!(g["places"], vec![("Field".to_string(), 2)]);
        assert_eq!(totals(&ix).clips, 2);
    }
}
