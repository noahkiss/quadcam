//! Sun tags: where the sun stood when a clip was flown, from its place and time alone. No
//! network: the sun's position comes from NOAA's solar formulas (Meeus, "Astronomical
//! Algorithms"), good to about a minute between 1900 and 2100.
//!
//! A clip gets at most one tag, the first that fits:
//! - `sunrise` / `sunset`: within 15 minutes of the sun crossing the horizon (the centre
//!   0.833 degrees below it, which allows for refraction and the sun's radius).
//! - `golden_hour`: the sun up but at most 6 degrees high.
//! - `dawn` / `dusk`: the sun below the horizon by at most 6 degrees (civil twilight).
//!
//! A clip without a time of day or a location has no tag. A clip's time is the Mac's local
//! time, as the library shows it.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// Where the sun stood at a clip's time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SunTag {
    Dawn,
    Sunrise,
    GoldenHour,
    Sunset,
    Dusk,
}

impl SunTag {
    /// The words the library shows and searches.
    pub fn label(self) -> &'static str {
        match self {
            SunTag::Dawn => "dawn",
            SunTag::Sunrise => "sunrise",
            SunTag::GoldenHour => "golden hour",
            SunTag::Sunset => "sunset",
            SunTag::Dusk => "dusk",
        }
    }
}

/// The sun's centre at sunrise and sunset, in degrees: refraction and the sun's radius.
const HORIZON: f64 = -0.833;
/// The top of golden hour, in degrees.
const GOLDEN: f64 = 6.0;
/// The bottom of civil twilight, in degrees.
const CIVIL: f64 = -6.0;
/// How close to sunrise or sunset a clip counts as sunrise or sunset.
const WINDOW_MIN: i64 = 15;

fn julian_day(t: DateTime<Utc>) -> f64 {
    t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5
}

/// The sun's declination (degrees) and the equation of time (minutes) at `t`.
fn solar(t: DateTime<Utc>) -> (f64, f64) {
    let jc = (julian_day(t) - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + jc * (36000.76983 + jc * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + jc * (35999.05029 - 0.0001537 * jc);
    let e = 0.016708634 - jc * (0.000042037 + 0.0000001267 * jc);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914602 - jc * (0.004817 + 0.000014 * jc))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * jc)
        + (3.0 * mr).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * jc).to_radians();
    let lambda = (l0 + c - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let eps0 =
        23.0 + (26.0 + (21.448 - jc * (46.815 + jc * (0.00059 - jc * 0.001813))) / 60.0) / 60.0;
    let eps = (eps0 + 0.00256 * omega.cos()).to_radians();
    let decl = (eps.sin() * lambda.sin()).asin().to_degrees();
    let y = (eps / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eot = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    (decl, eot)
}

/// The sun's hour angle at `t` and longitude `lon`, in degrees: negative before solar noon.
fn hour_angle(t: DateTime<Utc>, lon: f64, eot: f64) -> f64 {
    let minutes = t.hour() as f64 * 60.0 + t.minute() as f64 + t.second() as f64 / 60.0;
    let solar_time = (minutes + eot + 4.0 * lon).rem_euclid(1440.0);
    solar_time / 4.0 - 180.0
}

/// The sun's elevation above the horizon at `t`, in degrees, without refraction.
pub fn elevation(t: DateTime<Utc>, lat: f64, lon: f64) -> f64 {
    let (decl, eot) = solar(t);
    let (lat, decl) = (lat.to_radians(), decl.to_radians());
    let ha = hour_angle(t, lon, eot).to_radians();
    let cos_zenith = lat.sin() * decl.sin() + lat.cos() * decl.cos() * ha.cos();
    90.0 - cos_zenith.clamp(-1.0, 1.0).acos().to_degrees()
}

fn minutes(m: f64) -> Duration {
    Duration::milliseconds((m * 60_000.0).round() as i64)
}

/// The UTC minute of solar noon (`noon`) or of the horizon crossing `sign` hour angles from
/// it, on the UTC day starting at `day`; None when the sun does not cross the horizon.
fn crossing(day: DateTime<Utc>, lat: f64, lon: f64, sign: f64) -> Option<DateTime<Utc>> {
    // Two passes: the first guess sets the time the sun's numbers are taken at.
    let mut at = day + minutes(720.0 - 4.0 * lon);
    for _ in 0..2 {
        let (decl, eot) = solar(at);
        let (latr, declr) = (lat.to_radians(), decl.to_radians());
        let cos_ha =
            HORIZON.to_radians().sin() / (latr.cos() * declr.cos()) - latr.tan() * declr.tan();
        if !(-1.0..=1.0).contains(&cos_ha) {
            return None;
        }
        let ha = cos_ha.acos().to_degrees();
        at = day + minutes(720.0 - 4.0 * lon - eot + sign * 4.0 * ha);
    }
    Some(at)
}

/// Sunrise and sunset of the solar day that holds `t`. None during polar day or night.
pub fn events(t: DateTime<Utc>, lat: f64, lon: f64) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    // The day by local solar time, so a place far from Greenwich gets its own day.
    let solar_day = (t + minutes(4.0 * lon)).date_naive();
    let day = Utc.from_utc_datetime(&solar_day.and_hms_opt(0, 0, 0)?);
    Some((
        crossing(day, lat, lon, -1.0)?,
        crossing(day, lat, lon, 1.0)?,
    ))
}

/// The sun tag at `t` for a place, or None (daylight, night, or a place out of range).
pub fn tag(t: DateTime<Utc>, lat: f64, lon: f64) -> Option<SunTag> {
    if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
        return None;
    }
    let window = Duration::minutes(WINDOW_MIN);
    if let Some((rise, set)) = events(t, lat, lon) {
        if (t - rise).abs() <= window {
            return Some(SunTag::Sunrise);
        }
        if (t - set).abs() <= window {
            return Some(SunTag::Sunset);
        }
    }
    let e = elevation(t, lat, lon);
    let morning = hour_angle(t, lon, solar(t).1) < 0.0;
    if e > HORIZON && e <= GOLDEN {
        Some(SunTag::GoldenHour)
    } else if (CIVIL..=HORIZON).contains(&e) {
        Some(if morning { SunTag::Dawn } else { SunTag::Dusk })
    } else {
        None
    }
}

/// The moment a library clip was flown: its date and `HH:MM` time in the IANA zone `tz`
/// (its place's, `metadata::zone_for`), or on this Mac's clock without one.
pub fn local_instant(
    date: NaiveDate,
    time: Option<&str>,
    tz: Option<&str>,
) -> Option<DateTime<Utc>> {
    let t = NaiveTime::parse_from_str(time?.trim(), "%H:%M").ok()?;
    let at = date.and_time(t);
    match tz.and_then(|z| z.parse::<chrono_tz::Tz>().ok()) {
        Some(z) => z
            .from_local_datetime(&at)
            .earliest()
            .map(|d| d.with_timezone(&Utc)),
        None => Local
            .from_local_datetime(&at)
            .earliest()
            .map(|d| d.with_timezone(&Utc)),
    }
}

/// A library clip's sun tag, from its date, time and location, in the zone `tz`.
pub fn clip_tag(
    date: NaiveDate,
    time: Option<&str>,
    location: Option<&crate::metadata::Location>,
    tz: Option<&str>,
) -> Option<SunTag> {
    let l = location?;
    tag(local_instant(date, time, tz)?, l.lat, l.lon)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn near(got: DateTime<Utc>, want: &str, what: &str) {
        let off = (got - utc(want)).num_seconds().abs();
        assert!(off <= 120, "{what}: {got} is {off} s from {want}");
    }

    // Reference times: NOAA's solar calculator (gml.noaa.gov/grad/solcalc), to the minute.
    #[test]
    fn sunrise_and_sunset_match_reference_values() {
        // London, the June solstice: 04:43 and 21:21 BST.
        let (r, s) = events(utc("2024-06-21T12:00:00Z"), 51.5074, -0.1278).unwrap();
        near(r, "2024-06-21T03:43:00Z", "London sunrise");
        near(s, "2024-06-21T20:21:00Z", "London sunset");
        // New York, the December solstice: 07:16 and 16:32 EST.
        let (r, s) = events(utc("2024-12-21T17:00:00Z"), 40.7128, -74.0060).unwrap();
        near(r, "2024-12-21T12:16:00Z", "New York sunrise");
        near(s, "2024-12-21T21:32:00Z", "New York sunset");
        // Sydney, its summer: 05:41 and 20:05 AEDT (UTC+11), across the UTC date line.
        let (r, s) = events(utc("2024-12-21T02:00:00Z"), -33.8688, 151.2093).unwrap();
        near(r, "2024-12-20T18:41:00Z", "Sydney sunrise");
        near(s, "2024-12-21T09:05:00Z", "Sydney sunset");
    }

    #[test]
    fn polar_day_and_night_have_no_sunrise() {
        assert!(events(utc("2024-06-21T12:00:00Z"), 69.65, 18.96).is_none());
        assert!(events(utc("2024-12-21T12:00:00Z"), 69.65, 18.96).is_none());
    }

    #[test]
    fn noon_sun_stands_at_ninety_minus_the_latitude_off_the_declination() {
        // The March equinox: the sun is overhead at the equator at solar noon.
        let e = elevation(utc("2024-03-20T12:07:00Z"), 0.0, 0.0);
        assert!(e > 89.0, "{e}");
        // The June solstice, 40 N, solar noon at 74 W is near 16:58 UTC: 90 - (40 - 23.44).
        let e = elevation(utc("2024-06-21T16:58:00Z"), 40.0, -74.0);
        assert!((e - 73.44).abs() < 0.3, "{e}");
        // Midnight is below the horizon.
        assert!(elevation(utc("2024-06-21T04:58:00Z"), 40.0, -74.0) < 0.0);
    }

    #[test]
    fn tags_follow_the_sun() {
        let (lat, lon) = (51.5074, -0.1278);
        let (rise, set) = events(utc("2024-06-21T12:00:00Z"), lat, lon).unwrap();
        let at = |t: DateTime<Utc>| tag(t, lat, lon);
        assert_eq!(at(rise + Duration::minutes(10)), Some(SunTag::Sunrise));
        assert_eq!(at(set - Duration::minutes(14)), Some(SunTag::Sunset));
        assert_eq!(at(set + Duration::minutes(15)), Some(SunTag::Sunset));
        assert_eq!(at(set - Duration::minutes(40)), Some(SunTag::GoldenHour));
        assert_eq!(at(rise + Duration::minutes(40)), Some(SunTag::GoldenHour));
        assert_eq!(at(set + Duration::minutes(25)), Some(SunTag::Dusk));
        assert_eq!(at(rise - Duration::minutes(25)), Some(SunTag::Dawn));
        assert_eq!(at(utc("2024-06-21T12:00:00Z")), None, "midday");
        assert_eq!(at(utc("2024-06-21T00:00:00Z")), None, "night");
        assert_eq!(tag(rise, 95.0, 0.0), None, "no such place");
    }

    #[test]
    fn a_clip_time_is_read_in_its_places_zone() {
        let d = NaiveDate::from_ymd_opt(2024, 6, 21).unwrap();
        // British Summer Time and US Eastern Daylight Time.
        assert_eq!(
            local_instant(d, Some("18:30"), Some("Europe/London")),
            Some(utc("2024-06-21T17:30:00Z"))
        );
        assert_eq!(
            local_instant(d, Some("18:30"), Some("America/New_York")),
            Some(utc("2024-06-21T22:30:00Z"))
        );
        // Winter: London is on GMT.
        let w = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
        assert_eq!(
            local_instant(w, Some("18:30"), Some("Europe/London")),
            Some(utc("2024-01-10T18:30:00Z"))
        );
        // Sunset in London at 21:21 BST is a sunset tag there, whatever this Mac's zone.
        let loc = crate::metadata::Location {
            lat: 51.5,
            lon: -0.13,
            name: None,
        };
        assert_eq!(
            clip_tag(d, Some("21:21"), Some(&loc), Some("Europe/London")),
            Some(SunTag::Sunset)
        );
    }

    #[test]
    fn a_clip_needs_a_time_and_a_location() {
        let loc = crate::metadata::Location {
            lat: 51.5,
            lon: 0.0,
            name: None,
        };
        let d = NaiveDate::from_ymd_opt(2024, 6, 21).unwrap();
        assert_eq!(clip_tag(d, None, Some(&loc), None), None);
        assert_eq!(clip_tag(d, Some("12:00"), None, None), None);
        assert_eq!(clip_tag(d, Some("noon"), Some(&loc), None), None);
        assert!(local_instant(d, Some("18:30"), None).is_some());
        assert!(local_instant(d, Some("18:30"), Some("not a zone")).is_some());
        assert_eq!(SunTag::GoldenHour.label(), "golden hour");
        assert_eq!(
            serde_json::to_value(SunTag::GoldenHour).unwrap(),
            "golden_hour"
        );
    }
}
