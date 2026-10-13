//! What ElevenLabs charges, in one table (design 7.4): USD per 1,000 characters for each model,
//! an optional promo rate with its last day, and the longest text one request takes. Every
//! estimate reads it. Credits come from the dollar rate today (a live promo included) and the
//! price of one credit, `CREDIT_USD`. That figure is an estimate. After a paid call, the
//! `x-character-count` header gives the real cost, and `Recorded` keeps it when it is close to
//! the table's figure; the next estimate prefers it. Every reading goes to `charcost.log`,
//! kept or not.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// USD per 1,000 characters of a model the account bills at a multiplier of 1. A model the
/// table lacks is priced from its multiplier at this rate.
pub const BASE_USD_PER_1K: f64 = 0.08;

/// USD one credit costs. Measured on a pay-as-you-go account on 2026-10-13: $20 bought 54,945
/// credits, and two renders billed exactly this price (v4 Turbo at its $0.011 promo, 0.030
/// credits a character; Turbo v2.5 at $0.04, 0.110). A subscription plan may price a credit
/// differently, and no plan has been measured, so every credit figure stays an estimate.
pub const CREDIT_USD: f64 = 20.0 / 54_945.0;

/// Credits one character bills at `usd_per_1k`, to three decimals.
pub fn credits_per_char(usd_per_1k: f64) -> f64 {
    (usd_per_1k / 1000.0 / CREDIT_USD * 1000.0).round() / 1000.0
}

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
/// The prices match the page of 2026-10-13. The v4 promo ended on 2026-10-12; an expired promo
/// stays in the table and `promo_on` ignores it.
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

    /// Credits one character is estimated to bill on `today`, from the dollar rate then.
    pub fn credits_on(&self, today: &str) -> f64 {
        credits_per_char(self.usd_on(today))
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

    /// Credits per character the account billed on `model`, from the last paid call. A figure
    /// more than 5% from the table's figure for `today` is out of date (a promo began or
    /// ended) and is ignored.
    pub fn get(&self, model: &str, today: &str) -> Option<f64> {
        let got = self.load().get(model).copied().filter(|r| *r > 0.0)?;
        match rate(model) {
            Some(r) => {
                let m = r.credits_on(today);
                ((got - m).abs() <= m * 0.05).then_some(got)
            }
            None => Some(got),
        }
    }

    /// Keeps what a call billed: `billed` characters for `sent` characters of text. The rate
    /// is kept only within 5% of the model's own figure in the table on `today` (any figure
    /// there, promo or not, for a model the table lacks): the header may count plain
    /// characters, and a wrong rate would skew every later estimate. Both values are logged
    /// either way. Returns whether the rate was kept.
    pub fn record(&self, model: &str, billed: u64, sent: u64, today: &str) -> bool {
        if sent == 0 || billed == 0 {
            return false;
        }
        let got = billed as f64 / sent as f64;
        let known: Vec<f64> = match rate(model) {
            Some(r) => vec![r.credits_on(today)],
            None => RATES
                .iter()
                .flat_map(|r| {
                    std::iter::once(r.usd_per_1k)
                        .chain(r.promo.map(|(p, _)| p))
                        .map(credits_per_char)
                })
                .collect(),
        };
        let kept = known.iter().any(|m| (got - m).abs() <= m * 0.05);
        let line = format!(
            "{} {model}: billed {billed} for {sent} characters, {got:.3} credits a character; known {}; {}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
            known
                .iter()
                .map(|m| format!("{m}"))
                .collect::<Vec<_>>()
                .join(", "),
            if kept { "kept" } else { "not kept" }
        );
        eprintln!("quadcam: voice cost: {line}");
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("charcost.log"))
            {
                let _ = writeln!(f, "{line}");
            }
        }
        if !kept {
            return false;
        }
        let mut all = self.load();
        all.insert(model.to_string(), got);
        let _ = std::fs::write(&self.path, json!(all).to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_the_pricing_page() {
        let usd = |m: &str| rate(m).unwrap().usd_per_1k;
        for m in ["eleven_v4", "eleven_v3", "eleven_multilingual_v2"] {
            assert_eq!(usd(m), 0.08, "{m}");
        }
        for m in [
            "eleven_v4_turbo",
            "eleven_v3_conversational",
            "eleven_flash_v2_5",
            "eleven_turbo_v2_5",
        ] {
            assert_eq!(usd(m), 0.04, "{m}");
        }
        assert_eq!(rate("eleven_v3").unwrap().max_chars, 5000);
        assert_eq!(rate("eleven_multilingual_v2").unwrap().max_chars, 10000);
        assert_eq!(rate("eleven_flash_v2_5").unwrap().max_chars, 40000);
        assert!(rate("eleven_turbo_v2").is_none());
    }

    #[test]
    fn a_promo_applies_through_its_last_day_only() {
        let v4 = rate("eleven_v4").unwrap();
        assert_eq!(v4.usd_on("2026-10-12"), 0.022);
        assert_eq!(v4.promo_on("2026-10-12"), Some((0.022, "2026-10-12")));
        assert_eq!(v4.usd_on("2026-10-13"), 0.08);
        assert_eq!(v4.promo_on("2026-10-13"), None);
        let t = rate("eleven_v4_turbo").unwrap();
        assert_eq!(t.usd_on("2026-10-09"), 0.011);
        assert_eq!(rate("eleven_v3").unwrap().promo_on("2026-10-09"), None);
    }

    #[test]
    fn credits_come_from_the_dollar_rate_and_the_credit_price() {
        // The account's measurements: $20 for 54,945 credits; v4 Turbo at the $0.011 promo
        // billed 0.030 a character, Turbo v2.5 at $0.04 billed 0.110.
        assert_eq!(credits_per_char(0.08), 0.22);
        assert_eq!(credits_per_char(0.04), 0.11);
        let t = rate("eleven_v4_turbo").unwrap();
        assert_eq!(t.credits_on("2026-10-12"), 0.03);
        assert_eq!(t.credits_on("2026-10-13"), 0.11);
        assert_eq!(
            rate("eleven_turbo_v2_5").unwrap().credits_on("2026-10-13"),
            0.11
        );
        assert_eq!(rate("eleven_v4").unwrap().credits_on("2026-10-13"), 0.22);
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
        let day = "2026-10-13";
        assert_eq!(r.get("eleven_v4", day), None);
        assert!(r.record("eleven_v4", 219, 1000, day));
        assert!(!r.record("eleven_v4", 0, 100, day));
        assert_eq!(Recorded::new(d.path()).get("eleven_v4", day), Some(0.219));
    }

    #[test]
    fn a_rate_recorded_during_a_promo_is_ignored_after_it() {
        let d = tempfile::tempdir().unwrap();
        let r = Recorded::new(d.path());
        assert!(r.record("eleven_v4_turbo", 30, 1000, "2026-10-12"));
        assert_eq!(r.get("eleven_v4_turbo", "2026-10-12"), Some(0.03));
        assert_eq!(r.get("eleven_v4_turbo", "2026-10-13"), None);
    }

    #[test]
    fn a_rate_far_from_the_models_multiplier_is_logged_not_kept() {
        let d = tempfile::tempdir().unwrap();
        let r = Recorded::new(d.path());
        let day = "2026-10-13";
        // The header counted plain characters.
        assert!(!r.record("eleven_flash_v2_5", 100, 100, day));
        assert_eq!(r.get("eleven_flash_v2_5", day), None);
        assert!(r.record("eleven_flash_v2_5", 11, 100, day));
        // A model the table lacks takes any known figure.
        assert!(r.record("eleven_turbo_v2", 22, 100, day));
        assert!(!r.record("eleven_turbo_v2", 50, 100, day));
        let log = std::fs::read_to_string(d.path().join("voice/charcost.log")).unwrap();
        assert_eq!(log.lines().count(), 4);
        assert!(log.contains("eleven_flash_v2_5: billed 100 for 100 characters, 1.000 credits a character; known 0.11; not kept"), "{log}");
    }
}
