//! The new UI's `app/src/bindings.ts` comes from the `api` table through tauri-specta. These
//! tests fail when it is out of date; `QUADCAM_UPDATE_BINDINGS=1` writes it again.

use quadcam_lib::core::Core;
use std::path::PathBuf;

fn committed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../app/src/bindings.ts")
}

fn fresh() -> String {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("bindings.ts");
    quadcam_lib::specta_builder()
        .export(quadcam_lib::bindings_language(), &out)
        .unwrap();
    std::fs::read_to_string(out).unwrap()
}

#[test]
fn bindings_are_current() {
    let fresh = fresh();
    let path = committed();
    if std::env::var_os("QUADCAM_UPDATE_BINDINGS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &fresh).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == fresh,
        "app/src/bindings.ts is out of date: QUADCAM_UPDATE_BINDINGS=1 cargo test --test bindings"
    );
}

#[test]
fn every_method_has_a_typed_command() {
    let ts = fresh();
    for m in Core::METHODS {
        assert!(
            ts.contains(&format!("TAURI_INVOKE(\"{m}\"")),
            "{m} has no command in bindings.ts"
        );
    }
}
