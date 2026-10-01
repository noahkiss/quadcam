//! Output filenames: `YYYY-MM-DD_<name>.<ext>`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
