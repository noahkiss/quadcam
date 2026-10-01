//! Place search: an address or a named place to latitude and longitude, for saved places.
//!
//! Providers, chosen by the `geocoder` setting:
//! - `apple` (default): MapKit's `MKLocalSearch`, which finds addresses and points of
//!   interest. Free, no key, no account. macOS only.
//! - `nominatim`: OpenStreetMap's public Nominatim server, also free and keyless. Its usage
//!   policy asks for a real User-Agent, at most one request a second, and no search per
//!   keystroke; quadcam searches only when asked and waits between requests.
//! - `census`: the US Census Bureau geocoder. Free and keyless, US street addresses only,
//!   and it finds rural addresses the others miss. When a keyless provider finds nothing,
//!   quadcam asks it too.
//! - `google`: Google Places API (New) text search. Off unless chosen; needs an API key
//!   (`QUADCAM_GOOGLE_PLACES_KEY`, else the `googlePlacesKey` setting) on a Google Cloud
//!   project with billing enabled.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One search hit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
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

/// The Google Places key: the environment first, then the setting.
pub fn google_key(setting: Option<&str>) -> Option<String> {
    std::env::var("QUADCAM_GOOGLE_PLACES_KEY")
        .ok()
        .or_else(|| setting.map(str::to_string))
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Searches with `provider` (`apple`, `nominatim`, `census` or `google`). When `apple` or
/// `nominatim` finds nothing, the Census geocoder gets a try (US street addresses).
pub fn search(
    provider: &str,
    query: &str,
    limit: usize,
    google_key: Option<&str>,
) -> Result<Vec<GeoResult>> {
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
        "census" => census(query)?,
        "google" => google(
            query,
            limit,
            google_key.context(
                "Google place search needs an API key: set QUADCAM_GOOGLE_PLACES_KEY or the google_places_key setting, or use another provider",
            )?,
        )?,
        p => bail!(
            "unknown place search provider {p:?}; use one of {}",
            crate::settings::GEOCODERS.join(", ")
        ),
    };
    if out.is_empty() && matches!(provider, "apple" | "nominatim") {
        out = census(query).unwrap_or_default();
    }
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
                        // MapKit's title puts each address line on a text line.
                        let address = pm
                            .title()
                            .map(|t| t.to_string())
                            .unwrap_or_default()
                            .lines()
                            .map(str::trim)
                            .filter(|l| !l.is_empty())
                            .collect::<Vec<_>>()
                            .join(", ");
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
    let stamp = crate::paths::cache_dir().join("nominatim.last");
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

/// A short name for an address: its first part, with the street when the first part is
/// only a house number (`200, East Main Street, ...` is `200 East Main Street`).
fn address_name(address: &str) -> String {
    let mut parts = address.split(',').map(str::trim);
    let first = parts.next().unwrap_or("").to_string();
    if !first.is_empty() && !first.chars().any(|c| c.is_alphabetic()) {
        if let Some(street) = parts.next() {
            return format!("{first} {street}");
        }
    }
    first
}

fn nominatim(query: &str, limit: usize) -> Result<Vec<GeoResult>> {
    #[derive(Deserialize, specta::Type)]
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
                name: h
                    .name
                    .filter(|n| n.chars().any(|c| c.is_alphabetic()))
                    .unwrap_or_else(|| address_name(&h.display_name)),
                address: h.display_name,
                lat: h.lat.parse().ok()?,
                lon: h.lon.parse().ok()?,
                provider: "nominatim".into(),
            })
        })
        .collect())
}

/// The US Census Bureau's one-line address geocoder.
fn census(query: &str) -> Result<Vec<GeoResult>> {
    #[derive(Deserialize, specta::Type)]
    struct Coords {
        x: f64,
        y: f64,
    }
    #[derive(Deserialize, specta::Type)]
    struct Match {
        #[serde(rename = "matchedAddress")]
        matched: String,
        coordinates: Coords,
    }
    #[derive(Deserialize, specta::Type)]
    struct Res {
        #[serde(rename = "addressMatches", default)]
        matches: Vec<Match>,
    }
    #[derive(Deserialize, specta::Type)]
    struct Answer {
        result: Res,
    }
    let out = std::process::Command::new("/usr/bin/curl")
        .args(["--silent", "--show-error", "--fail", "--get"])
        .args(["--max-time", &TIMEOUT.as_secs().to_string()])
        .args(["--user-agent", &user_agent()])
        .args(["--data-urlencode", &format!("address={query}")])
        .args(["--data", "benchmark=Public_AR_Current"])
        .args(["--data", "format=json"])
        .arg("https://geocoding.geo.census.gov/geocoder/locations/onelineaddress")
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!(
            "Census address search failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let a: Answer = serde_json::from_slice(&out.stdout)
        .context("the Census geocoder sent an answer quadcam cannot read")?;
    Ok(a.result
        .matches
        .into_iter()
        .map(|m| GeoResult {
            name: m.matched.split(',').next().unwrap_or("").trim().to_string(),
            address: m.matched,
            lat: m.coordinates.y,
            lon: m.coordinates.x,
            provider: "census".into(),
        })
        .collect())
}

/// Google Places API (New) text search. The key goes to curl on stdin, never on its
/// command line.
fn google(query: &str, limit: usize, key: &str) -> Result<Vec<GeoResult>> {
    use std::io::Write;
    #[derive(Deserialize, specta::Type)]
    struct Text {
        text: String,
    }
    #[derive(Deserialize, specta::Type)]
    struct LatLng {
        latitude: f64,
        longitude: f64,
    }
    #[derive(Deserialize, specta::Type)]
    struct Place {
        #[serde(rename = "displayName")]
        name: Option<Text>,
        #[serde(rename = "formattedAddress", default)]
        address: String,
        location: Option<LatLng>,
    }
    #[derive(Deserialize, specta::Type)]
    struct Answer {
        #[serde(default)]
        places: Vec<Place>,
    }
    if key.contains(['\r', '\n']) {
        bail!("the Google Places key has a line break in it");
    }
    let body = serde_json::json!({"textQuery": query, "maxResultCount": limit}).to_string();
    let mut child = std::process::Command::new("/usr/bin/curl")
        .args([
            "--silent",
            "--show-error",
            "--max-time",
            &TIMEOUT.as_secs().to_string(),
        ])
        .args(["--user-agent", &user_agent()])
        .args(["--header", "Content-Type: application/json"])
        .args([
            "--header",
            "X-Goog-FieldMask: places.displayName,places.formattedAddress,places.location",
        ])
        .args(["--header", "@-"])
        .args(["--data", &body])
        .arg("https://places.googleapis.com/v1/places:searchText")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("running curl")?;
    child
        .stdin
        .take()
        .context("curl stdin")?
        .write_all(format!("X-Goog-Api-Key: {key}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "Google place search failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("Google sent an answer quadcam cannot read")?;
    if let Some(e) = v.get("error") {
        bail!(
            "Google place search refused: {} (check the key, and that the Places API (New) and billing are on for its project)",
            e.get("message").and_then(|m| m.as_str()).unwrap_or("error")
        );
    }
    let a: Answer = serde_json::from_value(v).context("reading Google's answer")?;
    Ok(a.places
        .into_iter()
        .filter_map(|p| {
            let l = p.location?;
            Some(GeoResult {
                name: p.name.map(|n| n.text).unwrap_or_else(|| p.address.clone()),
                address: p.address,
                lat: l.latitude,
                lon: l.longitude,
                provider: "google".into(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_and_unknown_are_refused() {
        assert!(super::search("apple", "  ", 5, None).is_err());
        let e = super::search("bing", "x", 5, None).unwrap_err();
        assert!(format!("{e:#}").contains("apple, nominatim, census, google"));
        let e = super::search("google", "x", 5, None).unwrap_err();
        assert!(
            format!("{e:#}").contains("QUADCAM_GOOGLE_PLACES_KEY"),
            "{e:#}"
        );
    }

    #[test]
    fn address_names() {
        assert_eq!(
            super::address_name("200, East Main Street, Bozeman"),
            "200 East Main Street"
        );
        assert_eq!(super::address_name("Tour Eiffel, 5, Avenue"), "Tour Eiffel");
    }

    #[test]
    fn user_agent_names_the_app() {
        assert!(super::user_agent().starts_with("quadcam/"));
        assert!(super::user_agent().contains("github.com"));
    }
}
