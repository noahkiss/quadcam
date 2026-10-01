//! Place search: an address or a named place to latitude and longitude, for saved places.
//!
//! Providers, chosen by the `geocoder` setting:
//! - `apple` (default): MapKit's `MKLocalSearch`, which finds addresses and points of
//!   interest. Free, no key, no account. macOS only.
//! - `nominatim`: OpenStreetMap's public Nominatim server, also free and keyless. Its usage
//!   policy asks for a real User-Agent, at most one request a second, and no search per
//!   keystroke; quadcam searches only when asked and waits between requests.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One search hit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeoResult {
    pub name: String,
    pub address: String,
    pub lat: f64,
    pub lon: f64,
    pub provider: String,
}

/// The most results one search returns.
pub const MAX_RESULTS: usize = 10;
const TIMEOUT: Duration = Duration::from_secs(20);

/// Searches with `provider` (`apple` or `nominatim`).
pub fn search(provider: &str, query: &str, limit: usize) -> Result<Vec<GeoResult>> {
    let query = query.trim();
    if query.is_empty() {
        bail!("Type an address or a place name to search for.");
    }
    if query.chars().count() > 200 {
        bail!("The search is too long; use at most 200 characters.");
    }
    let limit = limit.clamp(1, MAX_RESULTS);
    let mut out = match provider {
        "apple" => apple(query)?,
        "nominatim" => nominatim(query, limit)?,
        p => bail!(
            "unknown place search provider {p:?}; use one of {}",
            crate::settings::GEOCODERS.join(", ")
        ),
    };
    out.retain(|r| {
        r.lat.is_finite() && r.lon.is_finite() && r.lat.abs() <= 90.0 && r.lon.abs() <= 180.0
    });
    out.truncate(limit);
    Ok(out)
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // MKMapItem.placemark: its replacements need macOS 26; the app runs on 13.
fn apple(query: &str) -> Result<Vec<GeoResult>> {
    use block2::RcBlock;
    use objc2::AnyThread;
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSError, NSRunLoop, NSString, NSThread};
    use objc2_map_kit::{MKAnnotation, MKLocalSearch, MKLocalSearchRequest, MKLocalSearchResponse};
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<Result<Vec<GeoResult>, String>>();
    let block = RcBlock::new(move |resp: *mut MKLocalSearchResponse, err: *mut NSError| {
        // SAFETY: MapKit passes a valid response or a valid error (or null).
        let out = unsafe {
            match (resp.as_ref(), err.as_ref()) {
                (Some(r), _) => Ok(r
                    .mapItems()
                    .iter()
                    .map(|item| {
                        let pm = item.placemark();
                        let c = pm.coordinate();
                        let name = item.name().map(|n| n.to_string()).unwrap_or_default();
                        let address = pm.title().map(|t| t.to_string()).unwrap_or_default();
                        GeoResult {
                            name: if name.is_empty() {
                                address.clone()
                            } else {
                                name
                            },
                            address,
                            lat: c.latitude,
                            lon: c.longitude,
                            provider: "apple".into(),
                        }
                    })
                    .collect()),
                (None, Some(e)) => Err(e.localizedDescription().to_string()),
                (None, None) => Ok(Vec::new()),
            }
        };
        let _ = tx.send(out);
    });
    // SAFETY: plain MapKit calls with valid objects; the block outlives the search because
    // MapKit copies it.
    let search = unsafe {
        let req = MKLocalSearchRequest::new();
        req.setNaturalLanguageQuery(Some(&NSString::from_str(query)));
        let search = MKLocalSearch::initWithRequest(MKLocalSearch::alloc(), &req);
        search.startWithCompletionHandler(RcBlock::as_ptr(&block));
        search
    };
    // MapKit answers on the main queue. On the main thread (the CLI and the MCP server) the
    // run loop has to turn for that; in the app the main thread is already running it.
    let started = std::time::Instant::now();
    let answer = if NSThread::isMainThread_class() {
        let run_loop = NSRunLoop::currentRunLoop();
        loop {
            if let Ok(a) = rx.try_recv() {
                break Some(a);
            }
            if started.elapsed() > TIMEOUT {
                break None;
            }
            // SAFETY: NSDefaultRunLoopMode is a static NSString.
            run_loop.runMode_beforeDate(
                unsafe { NSDefaultRunLoopMode },
                &NSDate::dateWithTimeIntervalSinceNow(0.05),
            );
        }
    } else {
        rx.recv_timeout(TIMEOUT).ok()
    };
    match answer {
        Some(Ok(v)) => Ok(v),
        Some(Err(e)) if e.contains("MKErrorDomain error 4") || e.contains("placemark") => {
            Ok(Vec::new())
        }
        Some(Err(e)) => bail!("Apple place search failed: {e}"),
        None => {
            // SAFETY: cancelling a running search is always allowed.
            unsafe { search.cancel() };
            bail!(
                "Apple place search did not answer within {} s; check the network",
                TIMEOUT.as_secs()
            )
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn apple(_query: &str) -> Result<Vec<GeoResult>> {
    bail!("Apple place search needs macOS; set geocoder to nominatim")
}

/// The User-Agent Nominatim's policy asks for: the app and where to reach its maintainers.
fn user_agent() -> String {
    format!(
        "quadcam/{} (+{})",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    )
}

/// Waits until a second has passed since the last Nominatim request from any quadcam
/// process (a stamp file in the cache folder records it).
fn nominatim_throttle() {
    let stamp = crate::core::cache_dir().join("nominatim.last");
    let gap = Duration::from_millis(1100);
    if let Ok(t) = stamp.metadata().and_then(|m| m.modified()) {
        if let Ok(since) = t.elapsed() {
            if since < gap {
                std::thread::sleep(gap - since);
            }
        }
    }
    let _ = std::fs::create_dir_all(stamp.parent().unwrap());
    let _ = std::fs::write(&stamp, b"");
}

fn nominatim(query: &str, limit: usize) -> Result<Vec<GeoResult>> {
    #[derive(Deserialize)]
    struct Hit {
        name: Option<String>,
        display_name: String,
        lat: String,
        lon: String,
    }
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    nominatim_throttle();
    let out = std::process::Command::new("/usr/bin/curl")
        .args(["--silent", "--show-error", "--fail", "--get"])
        .args(["--max-time", &TIMEOUT.as_secs().to_string()])
        .args(["--user-agent", &user_agent()])
        .args(["--data-urlencode", &format!("q={query}")])
        .args(["--data", "format=jsonv2"])
        .args(["--data", &format!("limit={limit}")])
        .arg("https://nominatim.openstreetmap.org/search")
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!(
            "Nominatim search failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let hits: Vec<Hit> = serde_json::from_slice(&out.stdout)
        .context("Nominatim sent an answer quadcam cannot read")?;
    Ok(hits
        .into_iter()
        .filter_map(|h| {
            Some(GeoResult {
                name: h.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
                    h.display_name
                        .split(',')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                }),
                address: h.display_name,
                lat: h.lat.parse().ok()?,
                lon: h.lon.parse().ok()?,
                provider: "nominatim".into(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_and_unknown_are_refused() {
        assert!(super::search("apple", "  ", 5).is_err());
        let e = super::search("bing", "x", 5).unwrap_err();
        assert!(format!("{e:#}").contains("apple, nominatim"));
    }

    #[test]
    fn user_agent_names_the_app() {
        assert!(super::user_agent().starts_with("quadcam/"));
        assert!(super::user_agent().contains("github.com"));
    }
}
