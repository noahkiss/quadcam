//! The Map page's native map: an Apple MapKit `MKMapView` laid over the web view. The page
//! keeps a placeholder element where the map goes and reports its rectangle (`map_frame`); this
//! module puts the map exactly there, or hides it. The pins come from the page (`map_pins`), and
//! a click on one comes back as the `map-pick` event. No token and no network of QuadCam's own:
//! MapKit draws Apple's maps.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

pub use crate::weather::Sky;
pub use native::{probe_callout, CalloutLayout};

/// Where the map goes, in CSS pixels from the top left of the page, and the page's width in
/// CSS pixels (which gives the zoom between CSS pixels and points).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq)]
pub struct MapFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub page_w: f64,
}

pub use crate::settings::MapStyle;

/// What a pin stands for, which sets its colour.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    /// A place with clips.
    Place,
    /// A saved place without clips.
    Empty,
    /// One clip with a location but no place.
    Clip,
}

/// One pin. A click shows its callout; the callout's button sends `id` back in `map-pick`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct MapPin {
    pub id: String,
    pub kind: PinKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub lat: f64,
    pub lon: f64,
    /// Shown inside the marker (the clip count of a place); empty shows the default glyph.
    pub glyph: Option<String>,
    /// A place's clip count.
    #[serde(default)]
    pub count: Option<u32>,
    /// The first and last day (`YYYY-MM-DD`) of a place's clips; a clip's day in both.
    #[serde(default)]
    pub first: Option<String>,
    #[serde(default)]
    pub last: Option<String>,
    /// A clip's time of day, `HH:MM`.
    #[serde(default)]
    pub time: Option<String>,
    /// A clip's sky at flight time, and whether the sun was up then.
    #[serde(default)]
    pub sky: Option<Sky>,
    #[serde(default)]
    pub is_day: Option<bool>,
    /// A clip's poster frame on disk.
    #[serde(default)]
    pub thumb: Option<String>,
}

/// A pin's callout button was clicked (or Return pressed while its callout shows): its id.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct MapPick(pub String);

/// What a pin's callout shows, and what VoiceOver reads for the pin. Pure, so it is tested
/// without AppKit.
#[derive(Debug, Clone, PartialEq)]
pub struct Callout {
    pub title: String,
    /// The lines under the title.
    pub lines: Vec<String>,
    /// The clip's sky: an SF Symbol name and the sky as words.
    pub sky: Option<(&'static str, String)>,
    /// The button's title (Show clips, Open); None for a place without clips.
    pub button: Option<&'static str>,
    /// The pin's VoiceOver label.
    pub label: String,
}

/// A day as `Sep 28, 2026`; text that is not a `YYYY-MM-DD` date stays as it is.
pub fn day(d: &str) -> String {
    match chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d") {
        Ok(d) => d.format("%b %-d, %Y").to_string(),
        Err(_) => d.to_string(),
    }
}

/// Two days as one range: `Sep 28, 2026`, `Sep 3 – Sep 28, 2026`, or
/// `Dec 30, 2025 – Jan 2, 2026`.
pub fn day_range(first: &str, last: &str) -> String {
    if first == last {
        return day(first);
    }
    let parse = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    match (parse(first), parse(last)) {
        (Some(a), Some(b)) if a.format("%Y").to_string() == b.format("%Y").to_string() => {
            format!("{} – {}", a.format("%b %-d"), day(last))
        }
        _ => format!("{} – {}", day(first), day(last)),
    }
}

/// The SF Symbol for a sky; a clear or partly cloudy night shows the moon, as the app's own
/// icons do (`app/src/lib/weather.ts` `skyIcon`).
pub fn sky_symbol(sky: Sky, is_day: Option<bool>) -> &'static str {
    let night = is_day == Some(false);
    match sky {
        Sky::Clear if night => "moon.stars",
        Sky::Clear => "sun.max",
        Sky::PartlyCloudy if night => "cloud.moon",
        Sky::PartlyCloudy => "cloud.sun",
        Sky::Cloudy => "cloud",
        Sky::Fog => "cloud.fog",
        Sky::Drizzle => "cloud.drizzle",
        Sky::Rain => "cloud.rain",
        Sky::HeavyRain => "cloud.heavyrain",
        Sky::Sleet => "cloud.sleet",
        Sky::Snow => "cloud.snow",
        Sky::Thunderstorm => "cloud.bolt.rain",
        Sky::Hail => "cloud.hail",
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The callout of a pin: a place's clip count and date range with Show clips; a clip's day
/// and time and its sky with Open.
pub fn callout(p: &MapPin) -> Callout {
    let mut lines = Vec::new();
    let mut sky = None;
    let button;
    let kind;
    let dates = match (p.first.as_deref(), p.last.as_deref()) {
        (Some(a), Some(b)) => Some(day_range(a, b)),
        (Some(a), None) | (None, Some(a)) => Some(day(a)),
        (None, None) => None,
    };
    match p.kind {
        PinKind::Place | PinKind::Empty => {
            kind = "place";
            let n = p.count.unwrap_or(0);
            lines.push(match n {
                0 => "No clips".to_string(),
                1 => "1 clip".to_string(),
                n => format!("{n} clips"),
            });
            lines.extend(dates);
            button = (n > 0).then_some("Show clips");
        }
        PinKind::Clip => {
            kind = "clip";
            lines.extend(
                dates.map(|d| match p.time.as_deref().filter(|t| !t.is_empty()) {
                    Some(t) => format!("{d} at {t}"),
                    None => d,
                }),
            );
            sky = p.sky.map(|s| (sky_symbol(s, p.is_day), capital(s.word())));
            button = Some("Open");
        }
    }
    let mut label = vec![p.title.clone(), kind.to_string()];
    label.extend(lines.iter().cloned());
    label.extend(sky.as_ref().map(|s| s.1.clone()));
    Callout {
        title: p.title.clone(),
        lines,
        sky,
        button,
        label: label.join(", "),
    }
}

/// The map's rectangle in its parent view's points, from the page's CSS rectangle.
/// `bounds_w` and `bounds_h` are the web view's size in points; a web view that is not
/// flipped counts y from the bottom.
pub fn to_points(
    f: &MapFrame,
    bounds_w: f64,
    bounds_h: f64,
    flipped: bool,
) -> (f64, f64, f64, f64) {
    let s = if f.page_w > 0.0 {
        bounds_w / f.page_w
    } else {
        1.0
    };
    let (x, y, w, h) = (f.x * s, f.y * s, f.w * s, f.h * s);
    let y = if flipped { y } else { bounds_h - y - h };
    (x, y, w, h)
}

/// Shows the map at `frame`, or hides it when `frame` is None or empty.
#[tauri::command]
#[specta::specta]
pub fn map_frame(app: AppHandle, frame: Option<MapFrame>) -> Result<(), String> {
    with_host(&app, move |host| host.place(frame))
}

/// Replaces the pins and the map's look. `fit` zooms the map to show every pin.
#[tauri::command]
#[specta::specta]
pub fn map_pins(
    app: AppHandle,
    pins: Vec<MapPin>,
    style: MapStyle,
    fit: bool,
) -> Result<(), String> {
    with_host(&app, move |host| host.set_pins(&pins, style, fit))
}

#[cfg(target_os = "macos")]
fn with_host(
    app: &AppHandle,
    f: impl FnOnce(&mut native::Host) + Send + 'static,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next())
        .ok_or("no window")?;
    let handle = app.clone();
    window
        .with_webview(move |webview| {
            let view = webview.inner() as usize;
            // SAFETY: `inner` is the window's WKWebView, alive while the window is, and
            // `with_webview` runs this on the main thread.
            unsafe { native::with(view, handle, f) }
        })
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn with_host(
    _: &AppHandle,
    _: impl FnOnce(&mut native::Host) + Send + 'static,
) -> Result<(), String> {
    Err("The map needs macOS.".into())
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub struct Host;
    pub type Rect = [f64; 4];
    #[derive(Debug)]
    pub struct CalloutLayout {
        pub title: Option<Rect>,
        pub wrapper: Rect,
        pub lines: Vec<(String, Rect)>,
    }
    pub fn probe_callout(_: &super::MapPin) -> Option<CalloutLayout> {
        None
    }
    impl Host {
        pub fn place(&mut self, _: Option<super::MapFrame>) {}
        pub fn set_pins(&mut self, _: &[super::MapPin], _: super::MapStyle, _: bool) {}
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{callout, to_points, Callout, MapFrame, MapPick, MapPin, MapStyle, PinKind};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::Message;
    use objc2::{
        define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadMarker,
        MainThreadOnly,
    };
    use objc2_app_kit::{
        NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSColor, NSEvent, NSImage,
        NSImageScaling, NSImageView, NSLayoutAttribute, NSLayoutConstraint, NSResponder,
        NSStackView, NSTextField, NSUserInterfaceLayoutOrientation, NSView, NSWindowOrderingMode,
    };
    use objc2_core_location::CLLocationCoordinate2D;
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
    use objc2_map_kit::{
        MKAnnotation, MKAnnotationView, MKClusterAnnotation, MKHybridMapConfiguration,
        MKImageryMapConfiguration, MKMapConfiguration, MKMapView, MKMapViewDelegate,
        MKMarkerAnnotationView, MKPointAnnotation, MKStandardMapConfiguration,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use tauri::AppHandle;
    use tauri_specta::Event;

    const REUSE: &str = "pin";
    const CLUSTER: &str = "qc";
    /// Key codes: Return, the keypad's Enter, Escape.
    const RETURN: u16 = 36;
    const ENTER: u16 = 76;
    const ESCAPE: u16 = 53;
    /// The poster frame's size in a clip's callout, in points (16:9).
    const THUMB: (f64, f64) = (96.0, 54.0);

    pub struct Host {
        map: Retained<Map>,
        delegate: Retained<Delegate>,
        pins: Vec<Retained<MKPointAnnotation>>,
        webview: usize,
    }

    /// What a pin's annotation carries: its id, kind, glyph, callout and poster frame.
    struct Pin {
        id: String,
        kind: PinKind,
        glyph: Option<String>,
        callout: Callout,
        thumb: Option<String>,
    }

    thread_local! {
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
        /// Each pin, by its annotation's address.
        static PINS: RefCell<HashMap<usize, Pin>> = RefCell::new(HashMap::new());
        /// The annotation whose callout shows, by address.
        static SELECTED: Cell<Option<usize>> = const { Cell::new(None) };
    }

    pub struct Ivars {
        app: AppHandle,
    }

    define_class!(
        /// The map view: Return opens the shown callout's pin and Escape closes the callout.
        #[unsafe(super(MKMapView, NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "QuadCamMapView"]
        pub struct Map;

        impl Map {
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool {
                true
            }

            #[unsafe(method(keyDown:))]
            fn key_down(&self, event: &NSEvent) {
                let shown = SELECTED.with(Cell::get).is_some();
                match event.keyCode() {
                    RETURN | ENTER if shown => open_selected(),
                    ESCAPE if shown => close_selected(),
                    // SAFETY: the superclass handles every other key.
                    _ => unsafe { msg_send![super(self), keyDown: event] },
                }
            }
        }
    );

    define_class!(
        /// A callout's lines. MapKit's detail wrapper takes this view's fitting size and clips
        /// to it, but never places the view inside it, so a plain stack could sit part way out
        /// and lose its first line. This one pins its edges to the wrapper.
        #[unsafe(super(NSStackView, NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "QuadCamCalloutDetail"]
        pub struct Detail;

        impl Detail {
            #[unsafe(method(viewDidMoveToSuperview))]
            fn did_move(&self) {
                // SAFETY: the superclass's own handling first.
                unsafe { msg_send![super(self), viewDidMoveToSuperview] }
                let Some(parent) = (unsafe { self.superview() }) else {
                    return;
                };
                self.setTranslatesAutoresizingMaskIntoConstraints(false);
                let edges = [
                    self.topAnchor().constraintEqualToAnchor(&parent.topAnchor()),
                    self.bottomAnchor().constraintEqualToAnchor(&parent.bottomAnchor()),
                    self.leadingAnchor().constraintEqualToAnchor(&parent.leadingAnchor()),
                    self.trailingAnchor().constraintEqualToAnchor(&parent.trailingAnchor()),
                ];
                NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&edges));
            }
        }
    );

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "QuadCamMapDelegate"]
        #[ivars = Ivars]
        pub struct Delegate;

        impl Delegate {
            /// The callout's button.
            #[unsafe(method(openPin:))]
            fn open_pin(&self, _sender: Option<&AnyObject>) {
                open_selected();
            }
        }

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl MKMapViewDelegate for Delegate {
            #[unsafe(method_id(mapView:viewForAnnotation:))]
            fn view_for_annotation(
                &self,
                map: &MKMapView,
                annotation: &ProtocolObject<dyn MKAnnotation>,
            ) -> Option<Retained<MKAnnotationView>> {
                view_for(self, map, annotation)
            }

            #[unsafe(method(mapViewDidFinishLoadingMap:))]
            fn did_load(&self, _map: &MKMapView) {
                #[cfg(debug_assertions)]
                eprintln!("quadcam: map: loaded");
            }

            #[unsafe(method(mapViewDidFailLoadingMap:withError:))]
            fn did_fail(&self, _map: &MKMapView, error: &objc2_foundation::NSError) {
                eprintln!("quadcam: map: {}", error.localizedDescription());
            }

            #[unsafe(method(mapViewDidFinishRenderingMap:fullyRendered:))]
            fn did_render(&self, _map: &MKMapView, full: bool) {
                #[cfg(debug_assertions)]
                eprintln!("quadcam: map: rendered (full {full})");
            }

            #[unsafe(method(mapView:didSelectAnnotationView:))]
            fn did_select(&self, map: &MKMapView, view: &MKAnnotationView) {
                // SAFETY: MapKit calls this on the main thread with a live map and view.
                unsafe {
                    let Some(annotation) = view.annotation() else {
                        return;
                    };
                    let key = &*annotation as *const _ as *const () as usize;
                    if PINS.with(|s| s.borrow().contains_key(&key)) {
                        // MapKit shows the callout; the map takes the keys for it.
                        SELECTED.with(|s| s.set(Some(key)));
                        if let Some(w) = map.window() {
                            w.makeFirstResponder(Some(map));
                        }
                        return;
                    }
                    // A cluster MapKit made: zoom in to its pins.
                    map.deselectAnnotation_animated(Some(&annotation), false);
                    let obj: &NSObject = &*(Retained::as_ptr(&annotation) as *const NSObject);
                    if obj.isKindOfClass(MKClusterAnnotation::class()) {
                        let cluster = &*(obj as *const NSObject as *const MKClusterAnnotation);
                        map.showAnnotations_animated(&cluster.memberAnnotations(), true);
                    }
                }
            }

            #[unsafe(method(mapView:didDeselectAnnotationView:))]
            fn did_deselect(&self, _map: &MKMapView, view: &MKAnnotationView) {
                // SAFETY: MapKit calls this on the main thread with a live view.
                let key = unsafe { view.annotation() }
                    .map(|a| &*a as *const _ as *const () as usize);
                SELECTED.with(|s| {
                    if s.get() == key {
                        s.set(None);
                    }
                });
            }
        }
    );

    /// Sends the shown callout's pin to the page (`map-pick`) and closes the callout.
    fn open_selected() {
        let Some(key) = SELECTED.with(Cell::get) else {
            return;
        };
        let Some(id) = PINS.with(|s| s.borrow().get(&key).map(|p| p.id.clone())) else {
            return;
        };
        let app = HOST.with(|h| {
            let h = h.borrow();
            let host = h.as_ref()?;
            host.deselect(key);
            Some(host.delegate.ivars().app.clone())
        });
        if let Some(app) = app {
            let _ = MapPick(id).emit(&app);
        }
    }

    /// Closes the shown callout.
    fn close_selected() {
        if let Some(key) = SELECTED.with(Cell::get) {
            HOST.with(|h| {
                if let Some(host) = h.borrow().as_ref() {
                    host.deselect(key);
                }
            });
        }
    }

    fn label(text: &str, mtm: MainThreadMarker, secondary: bool) -> Retained<NSView> {
        let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        if secondary {
            l.setTextColor(Some(&NSColor::secondaryLabelColor()));
        }
        Retained::into_super(Retained::into_super(l))
    }

    fn stack(
        views: &[Retained<NSView>],
        vertical: bool,
        mtm: MainThreadMarker,
    ) -> Retained<NSStackView> {
        let refs: Vec<&NSView> = views.iter().map(|v| &**v).collect();
        let refs = NSArray::from_slice(&refs);
        let s = if vertical {
            // SAFETY: `Detail` is an `NSStackView`, so the class method makes one of it.
            let d: Retained<Detail> =
                unsafe { msg_send![Detail::class(), stackViewWithViews: &*refs] };
            Retained::into_super(d)
        } else {
            NSStackView::stackViewWithViews(&refs, mtm)
        };
        if vertical {
            s.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
            s.setAlignment(NSLayoutAttribute::Leading);
            s.setSpacing(2.0);
        } else {
            s.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
            s.setAlignment(NSLayoutAttribute::CenterY);
            s.setSpacing(4.0);
        }
        s
    }

    /// The callout's lines and the clip's sky, under the title.
    fn detail(c: &Callout, mtm: MainThreadMarker) -> Retained<NSView> {
        let mut rows: Vec<Retained<NSView>> = c.lines.iter().map(|l| label(l, mtm, true)).collect();
        if let Some((symbol, word)) = &c.sky {
            let word_ns = NSString::from_str(word);
            let mut row = Vec::new();
            if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(&word_ns),
            ) {
                let v = NSImageView::imageViewWithImage(&img, mtm);
                v.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                // The word beside it says the same.
                v.setAccessibilityElement(false);
                row.push(Retained::into_super(Retained::into_super(v)));
            }
            row.push(label(word, mtm, true));
            rows.push(Retained::into_super(stack(&row, false, mtm)));
        }
        Retained::into_super(stack(&rows, true, mtm))
    }

    /// A clip's poster frame, when it is on disk.
    fn thumb(path: &str, title: &str, mtm: MainThreadMarker) -> Option<Retained<NSView>> {
        let img = NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path))?;
        let v = NSImageView::imageViewWithImage(&img, mtm);
        v.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        v.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(THUMB.0, THUMB.1),
        ));
        v.setAccessibilityLabel(Some(&NSString::from_str(&format!("{title}, poster frame"))));
        Some(Retained::into_super(Retained::into_super(v)))
    }

    fn view_for(
        delegate: &Delegate,
        map: &MKMapView,
        annotation: &ProtocolObject<dyn MKAnnotation>,
    ) -> Option<Retained<MKAnnotationView>> {
        let mtm = delegate.mtm();
        let key = annotation as *const _ as *const () as usize;
        let (kind, glyph, c, poster) = PINS.with(|s| {
            s.borrow()
                .get(&key)
                .map(|p| (p.kind, p.glyph.clone(), p.callout.clone(), p.thumb.clone()))
        })?;
        // A cluster MapKit made has no entry and gets its own view (the `?` above).
        // SAFETY: MapKit calls this on the main thread with a live map and annotation.
        unsafe {
            let view =
                match map.dequeueReusableAnnotationViewWithIdentifier(&NSString::from_str(REUSE)) {
                    Some(v) => {
                        v.setAnnotation(Some(annotation));
                        Retained::cast_unchecked::<MKMarkerAnnotationView>(v)
                    }
                    None => MKMarkerAnnotationView::initWithAnnotation_reuseIdentifier(
                        MKMarkerAnnotationView::alloc(mtm),
                        Some(annotation),
                        Some(&NSString::from_str(REUSE)),
                    ),
                };
            view.setClusteringIdentifier(Some(&NSString::from_str(CLUSTER)));
            view.setGlyphText(glyph.as_deref().map(NSString::from_str).as_deref());
            let tint = match kind {
                PinKind::Place => NSColor::systemRedColor(),
                PinKind::Empty => NSColor::systemGrayColor(),
                PinKind::Clip => NSColor::systemBlueColor(),
            };
            view.setMarkerTintColor(Some(&tint));
            // A click shows the callout; its button does what the click used to.
            view.setCanShowCallout(true);
            view.setDetailCalloutAccessoryView(Some(&detail(&c, mtm)));
            let left = poster.as_deref().and_then(|p| thumb(p, &c.title, mtm));
            view.setLeftCalloutAccessoryView(left.as_deref());
            let button = c.button.map(|title| {
                let b = NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(delegate),
                    Some(sel!(openPin:)),
                    mtm,
                );
                b.setKeyEquivalent(&NSString::from_str("\r"));
                b.setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "{title}: {}",
                    c.title
                ))));
                b
            });
            view.setRightCalloutAccessoryView(button.as_deref().map(|b| &***b));
            view.setAccessibilityLabel(Some(&NSString::from_str(&c.label)));
            Some(Retained::into_super(view))
        }
    }

    /// A rectangle in window points: x, y, width, height.
    pub type Rect = [f64; 4];

    /// Where a shown callout put its parts, for `tests/map_callout.rs`.
    #[derive(Debug)]
    pub struct CalloutLayout {
        pub title: Option<Rect>,
        /// The view MapKit holds the lines in, which clips them.
        pub wrapper: Rect,
        pub lines: Vec<(String, Rect)>,
    }

    fn in_window(v: &NSView) -> Rect {
        let r = v.convertRect_toView(v.bounds(), None);
        [r.origin.x, r.origin.y, r.size.width, r.size.height]
    }

    /// Every text field under `v`, with its text.
    fn texts(v: &NSView, out: &mut Vec<(String, Rect)>) {
        if let Some(t) = v.downcast_ref::<NSTextField>() {
            out.push((t.stringValue().to_string(), in_window(v)));
            return;
        }
        for s in v.subviews() {
            texts(&s, out);
        }
    }

    fn find_detail(v: &NSView) -> Option<Retained<NSView>> {
        if v.isKindOfClass(Detail::class()) {
            return Some(v.retain());
        }
        v.subviews().iter().find_map(|s| find_detail(&s))
    }

    /// Shows `pin`'s callout, built as the map builds it, on a map in a window that never
    /// shows, and reads back where MapKit laid it out. Must run on the main thread; None when
    /// MapKit showed no callout (no window server).
    pub fn probe_callout(pin: &MapPin) -> Option<CalloutLayout> {
        use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
        use objc2_foundation::{NSDate, NSRunLoop};
        let mtm = MainThreadMarker::new()?;
        let spin = |s: f64| {
            NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(s))
        };
        let c = callout(pin);
        // SAFETY: main thread; every object lives to the end of this function.
        unsafe {
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(700.0, 500.0));
            let win = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            );
            win.setReleasedWhenClosed(false);
            let map = MKMapView::initWithFrame(MKMapView::alloc(mtm), rect);
            win.contentView()?.addSubview(&map);
            let a = MKPointAnnotation::new();
            a.setCoordinate(CLLocationCoordinate2D {
                latitude: pin.lat,
                longitude: pin.lon,
            });
            a.setTitle(Some(&NSString::from_str(&c.title)));
            let a = ProtocolObject::from_ref(&*a);
            map.addAnnotation(a);
            map.setCenterCoordinate_animated(a.coordinate(), false);
            spin(0.3);
            let view = map.viewForAnnotation(a)?;
            view.setCanShowCallout(true);
            view.setDetailCalloutAccessoryView(Some(&detail(&c, mtm)));
            let button = c.button.map(|t| {
                NSButton::buttonWithTitle_target_action(&NSString::from_str(t), None, None, mtm)
            });
            view.setRightCalloutAccessoryView(button.as_deref().map(|b| &***b));
            map.selectAnnotation_animated(a, false);
            spin(0.7);
            let root = win.contentView()?.superview()?;
            let detail = find_detail(&root)?;
            let wrapper = detail.superview()?;
            let mut lines = Vec::new();
            texts(&detail, &mut lines);
            let mut all = Vec::new();
            texts(&root, &mut all);
            let title = all.iter().find(|(t, _)| *t == c.title).map(|(_, r)| *r);
            win.close();
            Some(CalloutLayout {
                title,
                wrapper: in_window(&wrapper),
                lines,
            })
        }
    }

    /// # Safety
    /// `webview` must be the window's live WKWebView, and this must run on the main thread.
    pub unsafe fn with(webview: usize, app: AppHandle, f: impl FnOnce(&mut Host)) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        HOST.with(|h| {
            let mut h = h.borrow_mut();
            if h.as_ref().is_some_and(|host| host.webview != webview) {
                *h = None;
            }
            if h.is_none() {
                // SAFETY: the caller's promise.
                *h = unsafe { Host::new(webview, app, mtm) };
            }
            if let Some(host) = h.as_mut() {
                f(host);
            }
        });
    }

    impl Host {
        unsafe fn new(webview: usize, app: AppHandle, mtm: MainThreadMarker) -> Option<Host> {
            let web: &NSView = unsafe { (webview as *const NSView).as_ref() }?;
            let parent = web.superview()?;
            let map: Retained<Map> = unsafe {
                msg_send![
                    Map::alloc(mtm),
                    initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0))
                ]
            };
            let delegate: Retained<Delegate> = {
                let this = Delegate::alloc(mtm).set_ivars(Ivars { app });
                unsafe { msg_send![super(this), init] }
            };
            let mk: &MKMapView = &map;
            unsafe {
                mk.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                mk.setShowsZoomControls(true);
                mk.setShowsCompass(true);
                mk.setShowsScale(true);
            }
            mk.setHidden(true);
            // The page lays the map out; between two reports the map keeps its margins, as
            // the page's own layout does while the window is resized.
            mk.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            let view: &NSView = mk;
            parent.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Above, Some(web));
            Some(Host {
                map,
                delegate,
                pins: Vec::new(),
                webview,
            })
        }

        fn mk(&self) -> &MKMapView {
            &self.map
        }

        fn webview(&self) -> &NSView {
            // SAFETY: the window's web view outlives the host, which `with` replaces when the
            // web view changes.
            unsafe { &*(self.webview as *const NSView) }
        }

        /// Closes the callout of the pin at `key`.
        fn deselect(&self, key: usize) {
            if let Some(a) = self
                .pins
                .iter()
                .find(|p| Retained::as_ptr(p) as *const () as usize == key)
            {
                // SAFETY: on the main thread (the callers run there) with a live annotation.
                unsafe {
                    self.mk()
                        .deselectAnnotation_animated(Some(ProtocolObject::from_ref(&**a)), true)
                };
            }
            SELECTED.with(|s| s.set(None));
        }

        /// Gives the keys back to the page when the map held them.
        fn release_keys(&self) {
            let mk: &NSView = self.mk();
            let Some(w) = mk.window() else {
                return;
            };
            let holds = w
                .firstResponder()
                .is_some_and(|r| Retained::as_ptr(&r) as *const () == mk as *const _ as *const ());
            if holds {
                let web: &NSResponder = self.webview();
                w.makeFirstResponder(Some(web));
            }
        }

        pub fn place(&mut self, frame: Option<MapFrame>) {
            let frame = frame.filter(|f| f.w > 0.0 && f.h > 0.0);
            let Some(f) = frame else {
                if let Some(key) = SELECTED.with(Cell::get) {
                    self.deselect(key);
                }
                self.release_keys();
                self.mk().setHidden(true);
                return;
            };
            let web = self.webview();
            let b = web.bounds();
            let (x, y, w, h) = to_points(&f, b.size.width, b.size.height, web.isFlipped());
            let in_web = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
            let Some(parent) = (unsafe { self.mk().superview() }) else {
                return;
            };
            let rect = web.convertRect_toView(in_web, Some(&parent));
            self.mk().setFrame(rect);
            self.mk().setHidden(false);
        }

        pub fn set_pins(&mut self, pins: &[MapPin], style: MapStyle, fit: bool) {
            let held = self.map.clone();
            let map: &MKMapView = &held;
            unsafe {
                let config: Retained<MKMapConfiguration> = match style {
                    MapStyle::Standard => Retained::into_super(MKStandardMapConfiguration::new()),
                    MapStyle::Satellite => Retained::into_super(MKImageryMapConfiguration::new()),
                    MapStyle::Hybrid => Retained::into_super(MKHybridMapConfiguration::new()),
                };
                map.setPreferredConfiguration(&config);
            }
            // The same pins again (a style change, a library reload with nothing new on the
            // map) keep their annotations, so a shown callout stays open.
            let same = PINS.with(|s| {
                let s = s.borrow();
                self.pins.len() == pins.len()
                    && self.pins.iter().zip(pins).all(|(a, p)| {
                        let key = Retained::as_ptr(a) as *const () as usize;
                        s.get(&key).is_some_and(|q| {
                            q.id == p.id && q.callout == callout(p) && q.thumb == p.thumb
                        }) && {
                            // SAFETY: a live annotation on the main thread.
                            let c = unsafe { a.coordinate() };
                            c.latitude == p.lat && c.longitude == p.lon
                        }
                    })
            });
            if !same {
                unsafe {
                    let old: Vec<&ProtocolObject<dyn MKAnnotation>> = self
                        .pins
                        .iter()
                        .map(|p| ProtocolObject::from_ref(&**p))
                        .collect();
                    map.removeAnnotations(&NSArray::from_slice(&old));
                }
                SELECTED.with(|s| s.set(None));
                let mut ids = HashMap::new();
                self.pins = pins
                    .iter()
                    .map(|p| {
                        let a = unsafe { MKPointAnnotation::new() };
                        unsafe {
                            a.setCoordinate(CLLocationCoordinate2D {
                                latitude: p.lat,
                                longitude: p.lon,
                            });
                            a.setTitle(Some(&NSString::from_str(&p.title)));
                            a.setSubtitle(p.subtitle.as_deref().map(NSString::from_str).as_deref());
                        }
                        let key = Retained::as_ptr(&a) as *const () as usize;
                        ids.insert(
                            key,
                            Pin {
                                id: p.id.clone(),
                                kind: p.kind,
                                glyph: p.glyph.clone(),
                                callout: callout(p),
                                thumb: p.thumb.clone(),
                            },
                        );
                        a
                    })
                    .collect();
                PINS.with(|s| *s.borrow_mut() = ids);
            }
            let all: Vec<&ProtocolObject<dyn MKAnnotation>> = self
                .pins
                .iter()
                .map(|p| ProtocolObject::from_ref(&**p))
                .collect();
            let all = NSArray::from_slice(&all);
            unsafe {
                if !same {
                    map.addAnnotations(&all);
                }
                if fit && !self.pins.is_empty() {
                    map.showAnnotations_animated(&all, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(kind: PinKind) -> MapPin {
        MapPin {
            id: "x".into(),
            kind,
            title: "Home field".into(),
            subtitle: None,
            lat: 0.0,
            lon: 0.0,
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

    #[test]
    fn days_and_ranges_read_as_words() {
        assert_eq!(day("2026-09-28"), "Sep 28, 2026");
        assert_eq!(day("2026-09-03"), "Sep 3, 2026");
        assert_eq!(day("soon"), "soon");
        assert_eq!(day_range("2026-09-28", "2026-09-28"), "Sep 28, 2026");
        assert_eq!(
            day_range("2026-09-03", "2026-09-28"),
            "Sep 3 – Sep 28, 2026"
        );
        assert_eq!(
            day_range("2025-12-30", "2026-01-02"),
            "Dec 30, 2025 – Jan 2, 2026"
        );
    }

    #[test]
    fn a_place_callout_counts_its_clips_and_their_days() {
        let p = MapPin {
            count: Some(3),
            first: Some("2026-09-03".into()),
            last: Some("2026-09-28".into()),
            ..pin(PinKind::Place)
        };
        let c = callout(&p);
        assert_eq!(c.title, "Home field");
        assert_eq!(c.lines, ["3 clips", "Sep 3 – Sep 28, 2026"]);
        assert_eq!(c.button, Some("Show clips"));
        assert_eq!(c.sky, None);
        assert_eq!(c.label, "Home field, place, 3 clips, Sep 3 – Sep 28, 2026");

        let one = callout(&MapPin {
            count: Some(1),
            first: Some("2026-09-28".into()),
            last: Some("2026-09-28".into()),
            ..pin(PinKind::Place)
        });
        assert_eq!(one.lines, ["1 clip", "Sep 28, 2026"]);
    }

    #[test]
    fn a_place_without_clips_has_no_button() {
        let c = callout(&pin(PinKind::Empty));
        assert_eq!(c.lines, ["No clips"]);
        assert_eq!(c.button, None);
        assert_eq!(c.label, "Home field, place, No clips");
    }

    #[test]
    fn a_clip_callout_shows_its_day_time_and_sky() {
        let p = MapPin {
            title: "river-dive".into(),
            first: Some("2026-09-28".into()),
            last: Some("2026-09-28".into()),
            time: Some("18:30".into()),
            sky: Some(Sky::PartlyCloudy),
            is_day: Some(false),
            thumb: Some("/lib/.quadcam/posters/x.jpg".into()),
            ..pin(PinKind::Clip)
        };
        let c = callout(&p);
        assert_eq!(c.lines, ["Sep 28, 2026 at 18:30"]);
        assert_eq!(c.sky, Some(("cloud.moon", "Partly cloudy".to_string())));
        assert_eq!(c.button, Some("Open"));
        assert_eq!(
            c.label,
            "river-dive, clip, Sep 28, 2026 at 18:30, Partly cloudy"
        );
        // No time, no weather: the day alone.
        let bare = callout(&MapPin {
            time: None,
            sky: None,
            ..p
        });
        assert_eq!(bare.lines, ["Sep 28, 2026"]);
        assert_eq!(bare.sky, None);
    }

    #[test]
    fn every_sky_has_a_symbol_and_nights_show_the_moon() {
        use Sky::*;
        let all = [
            Clear,
            PartlyCloudy,
            Cloudy,
            Fog,
            Drizzle,
            Rain,
            HeavyRain,
            Sleet,
            Snow,
            Thunderstorm,
            Hail,
        ];
        let day: std::collections::HashSet<_> =
            all.iter().map(|s| sky_symbol(*s, Some(true))).collect();
        assert_eq!(day.len(), all.len());
        assert_eq!(sky_symbol(Clear, None), "sun.max");
        assert_eq!(sky_symbol(Clear, Some(false)), "moon.stars");
        assert_eq!(sky_symbol(Rain, Some(false)), "cloud.rain");
    }

    #[test]
    fn a_frame_maps_to_points() {
        let f = MapFrame {
            x: 232.0,
            y: 52.0,
            w: 568.0,
            h: 500.0,
            page_w: 800.0,
        };
        // A flipped web view at zoom 1: the same numbers.
        assert_eq!(
            to_points(&f, 800.0, 600.0, true),
            (232.0, 52.0, 568.0, 500.0)
        );
        // Not flipped: y from the bottom.
        assert_eq!(
            to_points(&f, 800.0, 600.0, false),
            (232.0, 48.0, 568.0, 500.0)
        );
        // Page zoom 2 (400 CSS px across 800 pt): every value doubles.
        let z = MapFrame { page_w: 400.0, ..f };
        assert_eq!(
            to_points(&z, 800.0, 600.0, true),
            (464.0, 104.0, 1136.0, 1000.0)
        );
    }
}
