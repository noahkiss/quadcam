//! Output filenames: `YYYY-MM-DD_<name>.<ext>`, or `YY.MM.DD_<name>.<ext>` with the
//! short date format.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How the date starts a file name.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum DateFormat {
    /// `2026-09-25`
    #[default]
    #[serde(rename = "YYYY-MM-DD")]
    Long,
    /// `26.09.25`
    #[serde(rename = "YY.MM.DD")]
    Short,
}

impl DateFormat {
    pub const NAMES: &[&str] = &["YYYY-MM-DD", "YY.MM.DD"];

    pub fn format(self, d: NaiveDate) -> String {
        match self {
            DateFormat::Long => d.format("%Y-%m-%d").to_string(),
            DateFormat::Short => d.format("%y.%m.%d").to_string(),
        }
    }
}

/// The date a file name starts with, in either format, and the rest of the name. The date
/// must be followed by `_`, `.` or nothing.
pub fn split_date(stem: &str) -> Option<(NaiveDate, &str)> {
    let try_one = |len: usize, fmt: &str| {
        let head = stem.get(..len)?;
        let rest = &stem[len..];
        if !(rest.is_empty() || rest.starts_with('_') || rest.starts_with('.')) {
            return None;
        }
        NaiveDate::parse_from_str(head, fmt).ok().map(|d| (d, rest))
    };
    try_one(10, "%Y-%m-%d").or_else(|| {
        let head = stem.get(..8)?;
        // `%y` alone would also take one digit; insist on the exact shape.
        if head.as_bytes().iter().enumerate().all(|(i, b)| {
            if i == 2 || i == 5 {
                *b == b'.'
            } else {
                b.is_ascii_digit()
            }
        }) {
            try_one(8, "%y.%m.%d")
        } else {
            None
        }
    })
}

/// Default short name when the name field is empty.
pub const DEFAULT_NAME: &str = "flight";

/// Name slug, lowercased: anything outside `[a-z0-9_\-. ]` becomes `_`,
/// whitespace runs become `_`, max 80 chars. Empty falls back to `default_name`.
pub fn slug(name: &str, default_name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let mut out = String::new();
    let mut in_space = false;
    for c in lowered.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push('_');
            }
            in_space = true;
            continue;
        }
        in_space = false;
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let out: String = out.chars().take(80).collect();
    // A name made only of dots would make a hidden or relative filename.
    if out.is_empty() || out.chars().all(|c| c == '.') {
        if default_name == name {
            return DEFAULT_NAME.to_string();
        }
        return slug(default_name, DEFAULT_NAME);
    }
    out
}

/// Date clean-up: drop anything outside `[a-z0-9_\-.]`, max 40 chars.
pub fn clean_date(date: &str) -> String {
    date.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
        .take(40)
        .collect()
}

/// `YYYY-MM-DD_<slug>` or, with a time, `YYYY-MM-DD_HHMM_<slug>`.
pub fn stem(date: &str, time_hhmm: Option<&str>, name: &str, default_name: &str) -> String {
    let date = clean_date(date);
    let slug = slug(name, default_name);
    match time_hhmm.map(clean_date).filter(|t| !t.is_empty()) {
        Some(t) => format!("{date}_{t}_{slug}"),
        None if date.is_empty() => slug,
        None => format!("{date}_{slug}"),
    }
}

/// Plans unique paths inside one output folder: `-2`, `-3` before the extension when the
/// target exists on disk or another clip in this batch already took the name. Never overwrite.
#[derive(Default)]
pub struct NamePlanner {
    taken: HashSet<PathBuf>,
}

impl NamePlanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn claim(&mut self, dir: &Path, stem: &str, ext: &str) -> PathBuf {
        let mut n = 1;
        loop {
            let file = if n == 1 {
                format!("{stem}.{ext}")
            } else {
                format!("{stem}-{n}.{ext}")
            };
            let path = dir.join(file);
            if !self.taken.contains(&path) && !path.exists() {
                self.taken.insert(path.clone());
                return path;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_rules() {
        assert_eq!(slug("wake-up", "flight"), "wake-up");
        assert_eq!(slug("Backyard Loops", "flight"), "backyard_loops");
        assert_eq!(slug("  a   b  ", "flight"), "a_b");
        assert_eq!(slug("Dive/Park!", "flight"), "dive_park_");
        assert_eq!(slug("", "flight"), "flight");
        assert_eq!(slug("   ", "flight"), "flight");
        assert_eq!(slug("..", "flight"), "flight");
        assert_eq!(slug("", ""), "flight");
        assert_eq!(slug("", "My Default"), "my_default");
        assert_eq!(slug(&"x".repeat(100), "flight").len(), 80);
        assert_eq!(slug("Ünïcode", "flight"), "_n_code");
    }

    #[test]
    fn date_formats_round_trip() {
        let d = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        assert_eq!(DateFormat::Short.format(d), "26.09.25");
        assert_eq!(DateFormat::Long.format(d), "2026-09-25");
        assert_eq!(
            split_date("26.09.25_home-first-flight"),
            Some((d, "_home-first-flight"))
        );
        assert_eq!(
            split_date("2026-09-25_loops_cut1"),
            Some((d, "_loops_cut1"))
        );
        assert_eq!(split_date("2026-09-25"), Some((d, "")));
        assert_eq!(split_date("26.9.25_x"), None);
        assert_eq!(split_date("2026-09-25x"), None);
        assert_eq!(split_date("PICT0001"), None);
        assert_eq!(
            serde_json::to_value(DateFormat::Short).unwrap(),
            serde_json::json!("YY.MM.DD")
        );
    }

    #[test]
    fn stem_rules() {
        assert_eq!(
            stem("2026-10-04", None, "backyard-loops", "flight"),
            "2026-10-04_backyard-loops"
        );
        assert_eq!(
            stem("2026-10-04", Some("1403"), "x", "flight"),
            "2026-10-04_1403_x"
        );
        assert_eq!(stem("2026/10/04", None, "", "flight"), "20261004_flight");
    }

    #[test]
    fn planner_suffixes_in_batch_and_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = NamePlanner::new();
        let a = p.claim(dir.path(), "2026-09-30_flight", "mp4");
        let b = p.claim(dir.path(), "2026-09-30_flight", "mp4");
        let c = p.claim(dir.path(), "2026-09-30_flight", "mp4");
        assert!(a.ends_with("2026-09-30_flight.mp4"));
        assert!(b.ends_with("2026-09-30_flight-2.mp4"));
        assert!(c.ends_with("2026-09-30_flight-3.mp4"));

        std::fs::write(dir.path().join("2026-09-30_x.mp4"), b"").unwrap();
        let mut p2 = NamePlanner::new();
        assert!(p2
            .claim(dir.path(), "2026-09-30_x", "mp4")
            .ends_with("2026-09-30_x-2.mp4"));
    }
}
