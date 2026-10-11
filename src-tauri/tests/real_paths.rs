//! A process started by cargo never reaches the real support folder: a core built without
//! `with_settings` refuses its default gear folder while `HOME` is the account's own. Its own
//! test binary, because it sets `HOME` for the process.

use quadcam_lib::core::{Core, NoHooks};
use quadcam_lib::paths;
use quadcam_lib::photos::Recorder;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

#[test]
fn a_core_without_settings_refuses_the_real_gear_folder() {
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    std::env::remove_var("QUADCAM_PATHS");
    let me = paths::account_home().expect("the account's home");
    std::env::set_var("HOME", &me);
    let work = tempfile::tempdir().unwrap();
    let core = Core::new(
        work.path().join("cache"),
        None,
        Arc::new(NoHooks),
        Arc::new(Recorder::default()),
    );

    let refused = catch_unwind(AssertUnwindSafe(|| core.gear_settings()))
        .expect_err("the real gear folder is refused");
    let why = refused
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(why.contains("QUADCAM_PATHS=real"), "{why}");
    assert!(paths::try_support_dir().is_err());
    for real in [paths::default_settings_file, paths::default_gear_dir] {
        assert!(catch_unwind(real).is_err());
    }

    // A test settings file keeps the gear folder beside it.
    let settings = work.path().join("support/settings.json");
    let core = core.with_settings(settings.clone());
    assert_eq!(
        core.gear_settings().gear_dir,
        work.path().join("support/gear")
    );

    // A temp HOME is never the real one.
    std::env::set_var("HOME", work.path());
    assert_eq!(
        paths::default_gear_dir(),
        work.path()
            .join("Library/Application Support/app.quadcam/gear")
    );
}
