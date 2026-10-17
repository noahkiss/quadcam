//! The map's callouts, shown by MapKit on a map in a window that never shows: every line sits
//! inside the view that clips it, and no two lines overlap. AppKit needs the main thread, so
//! this binary has no test harness (`harness = false`); it answers nextest's `--list` itself.

use quadcam_lib::mapview::{probe_callout, CalloutLayout, MapPin, PinKind, Sky};

const NAME: &str = "callout_lines_show_in_full";

fn pin(kind: PinKind, title: &str) -> MapPin {
    MapPin {
        id: "x".into(),
        kind,
        title: title.into(),
        subtitle: None,
        lat: 40.0,
        lon: -75.0,
        glyph: None,
        count: None,
        first: None,
        last: None,
        time: None,
        sky: None,
        is_day: None,
        thumb: None,
    }
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn check(what: &str, l: &CalloutLayout) {
    let w = l.wrapper;
    assert!(!l.lines.is_empty(), "{what}: no lines in {l:?}");
    for (text, r) in &l.lines {
        // A label's text box reaches 2 pt past its frame on each side, so x is not checked.
        assert!(
            r[1] >= w[1] - 0.5 && r[1] + r[3] <= w[1] + w[3] + 0.5,
            "{what}: {text:?} at {r:?} is cut by its wrapper {w:?}"
        );
        if let Some(t) = l.title {
            assert!(
                !overlaps(*r, t),
                "{what}: {text:?} at {r:?} overlaps the title at {t:?}"
            );
        }
    }
    for (i, (a, ra)) in l.lines.iter().enumerate() {
        for (b, rb) in &l.lines[i + 1..] {
            // A row of sky icon and word shares its line; only lines stacked apart must not meet.
            if (ra[1] - rb[1]).abs() > 0.5 {
                assert!(
                    !overlaps(*ra, *rb),
                    "{what}: {a:?} {ra:?} overlaps {b:?} {rb:?}"
                );
            }
        }
    }
}

fn run() {
    let place = MapPin {
        count: Some(15),
        first: Some("2026-08-29".into()),
        last: Some("2026-10-11".into()),
        glyph: Some("15".into()),
        ..pin(PinKind::Place, "Farm")
    };
    let long = MapPin {
        title: "A place with a very long name that runs past the callout's usual width".into(),
        ..place.clone()
    };
    let clip = MapPin {
        first: Some("2026-09-28".into()),
        last: Some("2026-09-28".into()),
        time: Some("18:30".into()),
        sky: Some(Sky::PartlyCloudy),
        is_day: Some(true),
        ..pin(PinKind::Clip, "river-dive")
    };
    let empty = pin(PinKind::Empty, "Home field");
    for (what, p, n) in [
        ("place", &place, 2),
        ("long name", &long, 2),
        ("clip", &clip, 2),
        ("empty", &empty, 1),
    ] {
        let Some(l) = probe_callout(p) else {
            println!("skipped: MapKit showed no callout here (no window server, or not macOS)");
            return;
        };
        assert_eq!(l.lines.len(), n, "{what}: {l:?}");
        check(what, &l);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("{NAME}: test");
        }
        return;
    }
    if let Some(f) = args.iter().skip(1).find(|a| !a.starts_with('-')) {
        if !NAME.contains(f.as_str()) {
            return;
        }
    }
    run();
    println!("test {NAME} ... ok");
}
