//! The macOS Share menu (`NSSharingServicePicker`) for library files: Messages, AirDrop,
//! Mail, Add to Photos and the other services the system offers for them.

use tauri::{AppHandle, Manager};

/// Shows the Share menu for `paths`, next to the rectangle `x`, `y`, `w`, `h` (CSS pixels,
/// from the top left of the window's content).
#[tauri::command]
pub fn share(
    app: AppHandle,
    paths: Vec<String>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Nothing to share.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let window = app
            .get_webview_window("main")
            .or_else(|| app.webview_windows().into_values().next())
            .ok_or("no window")?;
        let view = window.ns_view().map_err(|e| e.to_string())? as usize;
        app.run_on_main_thread(move || show(view, &paths, x, y, w, h))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, x, y, w, h);
        Err("Share needs macOS.".into())
    }
}

#[cfg(target_os = "macos")]
fn show(view: usize, paths: &[String], x: f64, y: f64, w: f64, h: f64) {
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_app_kit::{NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
    use std::cell::RefCell;

    thread_local! {
        // The picker stays alive while its menu shows.
        static OPEN: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
    }
    // SAFETY: `view` is the window's content view, which lives as long as the window, and
    // this runs on the main thread.
    let view: &NSView = unsafe { &*(view as *const NSView) };
    let urls: Vec<Retained<NSURL>> = paths
        .iter()
        .map(|p| NSURL::fileURLWithPath(&NSString::from_str(p)))
        .collect();
    let items = NSArray::from_retained_slice(&urls);
    // SAFETY: an array of NSURL is an array of objects, and NSURL conforms to
    // NSPasteboardWriting, which the picker needs of its items.
    let picker = unsafe {
        let items: Retained<NSArray> = Retained::cast_unchecked(items);
        NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items)
    };
    let top = if view.isFlipped() {
        y
    } else {
        view.bounds().size.height - y - h
    };
    let rect = NSRect::new(NSPoint::new(x, top), NSSize::new(w.max(1.0), h.max(1.0)));
    picker.showRelativeToRect_ofView_preferredEdge(rect, view, NSRectEdge::MinY);
    OPEN.with(|o| *o.borrow_mut() = Some(picker));
}
