//! Profile fitting (sim-design 6.3; S3 acceptance) on the CI fixtures: the fit finds the
//! example whoops' hover and roll lag, never validates worse than its base, and repeats
//! exactly. The decoder against `blackbox_decode`'s CSV runs on a local flash image only.

use std::path::Path;

use quadcam_sim::fit::{fit, FitInputs};
use quadcam_sim::log::{read_folder, LogData, Scales};
use quadcam_sim::preset;
use quadcam_sim::validate::validate;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixtures(id: &str) -> Vec<LogData> {
    read_folder(&Path::new(FIXTURES).join(id), Scales::default()).unwrap()
}

fn inputs(id: &str) -> FitInputs {
    FitInputs {
        id: format!("{id}-fitted"),
        label: format!("{id} (fitted)"),
        ..FitInputs::default()
    }
}

fn measured(r: &quadcam_sim::fit::FitReport, name: &str) -> f64 {
    r.measured
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .value
}

#[test]
fn the_fit_finds_the_example_whoops_hover() {
    for id in ["meteor75", "air65ii"] {
        let base = preset(id).unwrap();
        let logs = fixtures(id);
        let r = fit(&base, &inputs(id), &logs).unwrap();
        print!("{}", r.table());
        let rf = &base.reference;
        let cmd = measured(&r, "hover_command");
        let rpm = measured(&r, "hover_speed");
        assert!(
            (cmd - rf.hover_cmd.unwrap()).abs() <= 0.005,
            "{id}: hover {cmd}"
        );
        assert!(
            (rpm / rf.hover_rpm.unwrap() - 1.0).abs() <= 0.02,
            "{id}: hover {rpm} rpm"
        );
        let kt = r.values.iter().find(|v| v.name == "prop.kt").unwrap();
        assert!(kt.residual.is_some() && kt.samples > 0, "{id}: {kt:?}");
        assert!(
            r.estimates.iter().any(|e| e.starts_with("frame.mass")),
            "{id}: an unweighed quad's mass is an estimate"
        );
    }
    // The spike's roll lag for the 75 mm whoop; the 65 mm's 19.3 ms mixed in other moves.
    let r = fit(
        &preset("meteor75").unwrap(),
        &inputs("meteor75"),
        &fixtures("meteor75"),
    )
    .unwrap();
    assert!((measured(&r, "roll_lag") - 13.7).abs() <= 1.0);
}

#[test]
fn a_fitted_profile_validates_no_worse_than_its_base() {
    for id in ["meteor75", "air65ii"] {
        let base = preset(id).unwrap();
        let logs = fixtures(id);
        let r = fit(&base, &inputs(id), &logs).unwrap();
        let p = r.profile.clone().unwrap();
        assert_eq!(p.id, format!("{id}-fitted"));
        let v = validate(&p, &logs);
        assert!(v.passed(), "{id}: {:?}", v.failures());
    }
}

#[test]
fn the_fit_repeats_exactly() {
    let base = preset("air65ii").unwrap();
    let logs = fixtures("air65ii");
    let a = fit(&base, &inputs("air65ii"), &logs)
        .unwrap()
        .profile
        .unwrap();
    let b = fit(&base, &inputs("air65ii"), &logs)
        .unwrap()
        .profile
        .unwrap();
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn a_weighed_mass_and_a_diff_all_are_used() {
    let base = preset("meteor75").unwrap();
    let diff = "# diff all\nset motor_poles = 14\nset dshot_idle_value = 700\n";
    let mut i = inputs("meteor75");
    i.mass = Some(0.0581);
    i.diff = Some(diff.into());
    let r = fit(&base, &i, &fixtures("meteor75")).unwrap();
    let p = r.profile.unwrap();
    assert_eq!(p.frame.mass.value, 0.0581);
    assert_eq!(p.motor.poles.value, 14.0);
    assert!(!r.estimates.iter().any(|e| e.starts_with("frame.mass")));
}

#[test]
fn no_hover_is_an_error() {
    let mut logs = fixtures("meteor75");
    logs.retain(|l| !l.name.contains("hover"));
    for l in &mut logs {
        for a in &mut l.acc {
            a[2] = 2.0;
        }
    }
    let e = fit(&preset("meteor75").unwrap(), &inputs("meteor75"), &logs).unwrap_err();
    assert!(e.contains("hover"), "{e}");
}

/// QuadCam's decoder against the CSV `blackbox_decode` wrote: every fixture is a run of
/// rows in one of the flash image's logs, field by field. Set `QUADCAM_BBL_DIR` to a folder
/// holding `meteor75-msp.bbl` and `air65ii-msp.bbl` (the example whoops' 2026-10-07
/// pulls); without it the test does nothing. One stretch differs: in `air65ii/punch.csv`
/// `blackbox_decode` dropped 13 P frames before an I frame, which QuadCam's decoder keeps
/// (their loop iterations and values run straight into the I frame).
#[test]
fn the_decoder_matches_blackbox_decode_on_the_fixtures() {
    let Ok(dir) = std::env::var("QUADCAM_BBL_DIR") else {
        return;
    };
    for id in ["meteor75", "air65ii"] {
        let image = std::fs::read(Path::new(&dir).join(format!("{id}-msp.bbl"))).unwrap();
        let logs: Vec<_> = quadcam_sim::bbl::decode_image(&image)
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        for f in std::fs::read_dir(Path::new(FIXTURES).join(id)).unwrap() {
            let f = f.unwrap().path();
            let rows: Vec<Vec<i64>> = std::fs::read_to_string(&f)
                .unwrap()
                .lines()
                .filter(|l| l.starts_with(|c: char| c.is_ascii_digit() || c == '-'))
                .map(|l| {
                    l.split(',')
                        // Volts and amps (two decimals) as the log's hundredths.
                        .map(|x| match x.contains('.') {
                            true => (x.parse::<f64>().unwrap() * 100.0).round() as i64,
                            false => x.parse().unwrap(),
                        })
                        .collect()
                })
                .collect();
            let cols: Vec<String> = quadcam_sim::log::FIXTURE_COLUMNS
                .iter()
                .map(|c| c.split(" (").next().unwrap().to_string())
                .collect();
            let found = logs.iter().any(|l| {
                let ix: Vec<usize> = cols.iter().map(|c| l.field(c).unwrap()).collect();
                let row = |k: usize| -> Vec<i64> { ix.iter().map(|&i| l.frames[k][i]).collect() };
                (0..l.frames.len()).any(|s| {
                    let r0 = row(s);
                    if r0[1..] != rows[0][1..] {
                        return false;
                    }
                    let mut k = s;
                    let mut skipped = 0;
                    for want in &rows {
                        // The one stretch `blackbox_decode` dropped: skip ahead to its row.
                        while k < l.frames.len() && row(k)[0] - r0[0] != want[0] && skipped < 13 {
                            k += 1;
                            skipped += 1;
                        }
                        if k >= l.frames.len()
                            || row(k)[0] - r0[0] != want[0]
                            || row(k)[1..] != want[1..]
                        {
                            return false;
                        }
                        k += 1;
                    }
                    skipped == 0 || f.ends_with("air65ii/punch.csv")
                })
            });
            assert!(found, "{}: no matching rows in {id}-msp.bbl", f.display());
        }
    }
}
