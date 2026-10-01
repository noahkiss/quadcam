//! The default output folder: ~/Movies/quadcam, resolved from $HOME at runtime and
//! created on first use. Its own test binary, because it changes $HOME for the process.

use quadcam_lib::media::{Encoder, Format};
use quadcam_lib::pipeline::{self, ImportSettings};

#[test]
fn default_output_dir_follows_home_and_is_created() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());

    let dir = pipeline::default_output_dir().unwrap();
    assert_eq!(dir, home.path().join("Movies/quadcam"));
    assert!(!dir.exists());

    let settings = ImportSettings {
        output_dir: dir.clone(),
        format: Format::Mp4,
        encoder: Encoder::Videotoolbox,
        keep_originals: false,
        add_time: false,
        default_name: "flight".into(),
        places: Vec::new(),
        profiles: Vec::new(),
        default_profile: None,
        layout: quadcam_lib::library::Layout::Flat,
        place_folders: false,
        import_id: String::new(),
    };
    pipeline::preflight(&settings, &[]).unwrap();
    assert!(
        dir.is_dir(),
        "the default folder and its parents are created on first import"
    );

    // A folder the user picked is never created for them.
    let picked = ImportSettings {
        output_dir: home.path().join("gone"),
        ..settings
    };
    assert!(pipeline::preflight(&picked, &[]).is_err());
    assert!(!home.path().join("gone").exists());
}
