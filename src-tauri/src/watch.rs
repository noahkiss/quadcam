//! Watches the files other processes write while the app runs: the settings file and the
//! library index (`<library>/.quadcam/index.json`). The CLI and a headless MCP server write
//! them directly, so this is how their changes show in the app at once.

use crate::core::Core;
use crate::library;
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Starts a thread that calls `on_settings` after the settings file changes (it has
/// already re-read it into `core`) and `on_library` after the library index changes. It
/// follows the library folder when the settings point somewhere new.
pub fn spawn(
    core: Arc<Core>,
    on_settings: impl Fn() + Send + 'static,
    on_library: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let Ok(settings) = core.settings_file().map(Path::to_path_buf) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut w) = notify::recommended_watcher(tx) else {
            return;
        };
        if let Some(dir) = settings.parent() {
            let _ = std::fs::create_dir_all(dir);
            if w.watch(dir, RecursiveMode::NonRecursive).is_err() {
                return;
            }
        }
        // The library folder, with its subfolders: the index folder may not exist yet.
        // Only index writes count; other files there change often during an import.
        let mut watched: Option<PathBuf> = None;
        let retarget = |w: &mut notify::RecommendedWatcher, watched: &mut Option<PathBuf>| {
            let want = core.library_root().ok().filter(|r| r.is_dir());
            if want != *watched {
                if let Some(old) = watched.take() {
                    let _ = w.unwatch(&old);
                }
                if let Some(new) = want {
                    if w.watch(&new, RecursiveMode::Recursive).is_ok() {
                        *watched = Some(new);
                    }
                }
            }
        };
        retarget(&mut w, &mut watched);
        let name = |p: &Path, n: &str| p.file_name().is_some_and(|f| f == n);
        let settings_name = settings
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        while let Ok(first) = rx.recv() {
            // Gather a burst of events (a rename makes several) into one reload.
            let mut events = vec![first];
            std::thread::sleep(Duration::from_millis(150));
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
            let paths: Vec<PathBuf> = events
                .into_iter()
                .filter_map(Result::ok)
                .flat_map(|e| e.paths)
                .collect();
            let settings_hit = paths.iter().any(|p| name(p, &settings_name));
            let library_hit = paths.iter().any(|p| {
                name(p, library::INDEX_FILE)
                    && p.parent().is_some_and(|d| name(d, library::INDEX_DIR))
            });
            if settings_hit {
                core.reload_settings();
                on_settings();
                retarget(&mut w, &mut watched);
            }
            if library_hit || settings_hit {
                on_library();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::core::{Core, NoHooks};
    use crate::photos::Recorder;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn wait_for(n: &AtomicUsize, at_least: usize) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if n.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// The app's core hears about the CLI's writes: a settings change, then a library
    /// index written by another core into a folder the settings just pointed at.
    #[test]
    fn writes_from_another_process_are_seen() {
        let d = tempfile::tempdir().unwrap();
        let settings = d.path().join("support/settings.json");
        let lib = d.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let mk = || {
            Arc::new(
                Core::new(
                    d.path().join("cache"),
                    None,
                    Arc::new(NoHooks),
                    Arc::new(Recorder::default()),
                )
                .with_settings(settings.clone()),
            )
        };
        let app = mk();
        let s = Arc::new(AtomicUsize::new(0));
        let l = Arc::new(AtomicUsize::new(0));
        let (s2, l2) = (s.clone(), l.clone());
        super::spawn(
            app.clone(),
            move || {
                s2.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                l2.fetch_add(1, Ordering::SeqCst);
            },
        );
        // FSEvents takes a moment to start delivering.
        std::thread::sleep(Duration::from_millis(1000));

        let cli = mk();
        cli.settings_set(&serde_json::from_value(serde_json::json!({"output_dir": lib})).unwrap())
            .unwrap();
        assert!(wait_for(&s, 1), "settings change seen");
        assert_eq!(app.defaults().output_dir.as_deref(), Some(lib.as_path()));

        let before = l.load(Ordering::SeqCst);
        cli.library_rebuild().unwrap(); // writes lib/.quadcam/index.json
        assert!(wait_for(&l, before + 1), "index change seen");
        // Later writes are seen too.
        std::thread::sleep(Duration::from_millis(300));
        let before = l.load(Ordering::SeqCst);
        cli.library_rebuild().unwrap();
        assert!(wait_for(&l, before + 1), "a second index write seen");
    }
}
