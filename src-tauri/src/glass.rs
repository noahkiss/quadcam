//! The window's native chrome on macOS 26 and later: the title bar overlays the content, and
//! the sidebar sits on Liquid Glass (`NSGlassEffectView`), as an AppKit sidebar does. The web
//! view stops drawing its own background, so the glass shows where the page is transparent.
//! Older macOS keeps the standard title bar and an opaque page.

use tauri::{webview::WebviewWindowBuilder, Runtime, WebviewWindow};

/// The sidebar's width in points. `--sidebar-w` in `app/src/theme/tokens.css` must match.
pub const SIDEBAR_WIDTH: f64 = 232.0;

/// The toolbar's height in points, which the traffic lights centre in. `--bar-h` in
/// `app/src/theme/tokens.css` must match.
pub const BAR_HEIGHT: f64 = 52.0;

/// Tells the page it sits on glass: `native.ts` reads it before the first render.
const MARK: &str = "window.__QUADCAM_GLASS__ = true;";

/// Whether this Mac draws the glass chrome (macOS 26 or later).
pub fn supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2::available!(macos = 26.0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Sets up the overlay title bar and marks the page, on a Mac that draws the glass.
pub fn configure<'a, R: Runtime, M: tauri::Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
) -> WebviewWindowBuilder<'a, R, M> {
    if !supported() {
        return builder;
    }
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        // The traffic lights' left edge, and the extra title bar height that centres them in
        // the toolbar. tao makes the title bar the buttons' height plus `y`, and AppKit keeps
        // each button 9.5 pt above its bottom (measured on macOS 27), so the centre sits at
        // `y - 2.5` from the top.
        .traffic_light_position(tauri::LogicalPosition::new(20.0, BAR_HEIGHT / 2.0 + 2.5));
    builder.initialization_script(MARK)
}

/// Puts the glass behind the sidebar and makes the web view transparent. Runs on the main
/// thread once the window exists; does nothing where `supported` is false.
pub fn install<R: Runtime>(window: &WebviewWindow<R>) {
    if !supported() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let result = window.with_webview(|webview| {
            // SAFETY: `inner` is the window's WKWebView, alive while the window is, and
            // `with_webview` runs this on the main thread.
            unsafe { install_behind(webview.inner().cast()) }
        });
        if let Err(e) = result {
            eprintln!("quadcam: glass: {e}");
        }
    }
}

/// # Safety
/// `webview` must be a live WKWebView, and this must run on the main thread.
#[cfg(target_os = "macos")]
unsafe fn install_behind(webview: *mut objc2_app_kit::NSView) {
    use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSGlassEffectView, NSView, NSWindowOrderingMode,
    };
    use objc2_foundation::{ns_string, NSNumber, NSPoint, NSRect, NSSize};

    let Some(webview) = (unsafe { webview.as_ref() }) else {
        return;
    };
    let Some(parent) = (unsafe { webview.superview() }) else {
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // WKWebView has no public switch for a clear background; wry uses this key for its
    // transparent windows.
    let no = NSNumber::new_bool(false);
    let _: () =
        unsafe { msg_send![webview, setValue: &*no, forKey: ns_string!("drawsBackground")] };

    let height = parent.bounds().size.height;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIDEBAR_WIDTH, height));
    let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
    // An AppKit sidebar's glass: regular style, square corners, the window's full height.
    glass.setCornerRadius(0.0);
    glass.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewHeightSizable | NSAutoresizingMaskOptions::ViewMaxXMargin,
    );
    let glass: &NSView = &glass;
    parent.addSubview_positioned_relativeTo(glass, NSWindowOrderingMode::Below, Some(webview));
}

#[cfg(test)]
mod tests {
    /// The page lays the sidebar and toolbar out from these tokens; the glass and the traffic
    /// lights use the constants. They must agree.
    #[test]
    fn sizes_match_the_tokens() {
        let tokens = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/src/theme/tokens.css"),
        )
        .unwrap();
        assert!(tokens.contains(&format!("--sidebar-w: {}px;", super::SIDEBAR_WIDTH)));
        assert!(tokens.contains(&format!("--bar-h: {}px;", super::BAR_HEIGHT)));
    }
}
