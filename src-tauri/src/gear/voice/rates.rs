//! What ElevenLabs charges, in one table (design 7.4): USD per 1,000 characters for each model,
//! an optional promo rate with its last day, and the longest text one request takes. Every
//! estimate reads it. Credits come from the dollar rate: one credit is one character at the
//! base rate of the 0.08 models, so a model's credits per character are its base rate over
//! 0.08. That figure is an estimate. After a paid call, the `x-character-count` header gives
//! the real cost, and `Recorded` keeps it; the next estimate prefers it.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// USD per 1,000 characters that equals one credit per character.
pub const BASE_USD_PER_1K: f64 = 0.08;

/// One model's price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub model: &'static str,
    /// USD per 1,000 characters.
    pub usd_per_1k: f64,
    /// A temporary lower rate and its last day (`YYYY-MM-DD`, inclusive).
    pub promo: Option<(f64, &'static str)>,
    /// The longest text one request takes.
    pub max_chars: u64,
}

/// The pricing page of 2026-10-09. `eleven_v4` and `eleven_v4_turbo` have no limit on the page:
/// they take the v3 limit until the account says otherwise.
pub const RATES: &[Rate] = &[
    Rate {
        model: "eleven_v4",
        usd_per_1k: 0.08,
        promo: Some((0.022, "2026-10-12")),
        max_chars: 5000,
    },
    Rate {
        model: "eleven_v4_turbo",
        usd_per_1k: 0.04,
        promo: Some((0.011, "2026-10-12")),
        max_chars: 5000,
    },
    Rate {
        model: "eleven_v3",
        usd_per_1k: 0.08,
        promo: None,
        max_chars: 5000,
    },
    Rate {
        model: "eleven_v3_conversational",
        usd_per_1k: 0.04,
        promo: None,
        max_chars: 5000,
    },
    Rate {
        model: "eleven_multilingual_v2",
        usd_per_1k: 0.08,
        promo: None,
        max_chars: 10000,
    },
    Rate {
        model: "eleven_flash_v2_5",
        usd_per_1k: 0.04,
        promo: None,
        max_chars: 40000,
    },
    Rate {
        model: "eleven_turbo_v2_5",
        usd_per_1k: 0.04,
        promo: None,
        max_chars: 40000,
    },
];

pub fn rate(model: &str) -> Option<&'static Rate> {
    RATES.iter().find(|r| r.model == model)
}

impl Rate {
    /// The promo rate when `today` (`YYYY-MM-DD`) is on or before its last day.
    pub fn promo_on(&self, today: &str) -> Option<(f64, &'static str)> {
        self.promo.filter(|(_, until)| today <= *until)
    }

    /// USD per 1,000 characters on `today`.
    pub fn usd_on(&self, today: &str) -> f64 {
        self.promo_on(today).map_or(self.usd_per_1k, |(p, _)| p)
    }

    /// Credits one character is estimated to bill, from the base rate.
    pub fn credits_per_char(&self) -> f64 {
        self.usd_per_1k / BASE_USD_PER_1K
    }
}

/// Today's date, `YYYY-MM-DD`.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// The longest text to send in one request on `model`: nine tenths of the limit, so a batch
/// stays below it. 0 when no limit is known.
pub fn batch_limit(model: &str, api_limit: u64) -> usize {
    let max = rate(model).map(|r| r.max_chars).unwrap_or(api_limit);
    (max / 10 * 9) as usize
}

/// Costs the account billed, as credits per character by model, kept beside the batch cache.
pub struct Recorded {
    path: PathBuf,
}

impl Recorded {
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            path: cache_dir.join("voice").join("charcost.json"),
        }
    }

    fn load(&self) -> BTreeMap<String, f64> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Credits per character the account billed on `model`, from the last paid call.
    pub fn get(&self, model: &str) -> Option<f64> {
        self.load().get(model).copied().filter(|r| *r > 0.0)
    }

    /// Keeps what a call billed: `billed` characters for `sent` characters of text.
    pub fn record(&self, model: &str, billed: u64, sent: u64) {
        if sent == 0 || billed == 0 {
            return;
        }
        let mut all = self.load();
        all.insert(model.to_string(), billed as f64 / sent as f64);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, json!(all).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_the_pricing_page() {
        let v4 = rate("eleven_v4").unwrap();
        assert_eq!(v4.usd_on("2026-10-12"), 0.022);
        assert_eq!(v4.usd_on("2026-10-13"), 0.08);
        assert_eq!(v4.credits_per_char(), 1.0);
        let t = rate("eleven_v4_turbo").unwrap();
        assert_eq!(t.usd_on("2026-10-09"), 0.011);
        assert_eq!(t.credits_per_char(), 0.5);
        assert_eq!(rate("eleven_flash_v2_5").unwrap().credits_per_char(), 0.5);
        assert_eq!(rate("eleven_multilingual_v2").unwrap().max_chars, 10000);
        assert!(rate("eleven_turbo_v2").is_none());
    }

    #[test]
    fn a_batch_stays_below_the_limit() {
        assert_eq!(batch_limit("eleven_v3", 0), 4500);
        assert_eq!(batch_limit("eleven_flash_v2_5", 0), 36000);
        assert_eq!(batch_limit("other", 1000), 900);
        assert_eq!(batch_limit("other", 0), 0);
    }

    #[test]
    fn a_recorded_cost_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let r = Recorded::new(d.path());
        assert_eq!(r.get("eleven_v4"), None);
        r.record("eleven_v4", 50, 100);
        r.record("eleven_v4", 0, 100);
        assert_eq!(Recorded::new(d.path()).get("eleven_v4"), Some(0.5));
    }
}
