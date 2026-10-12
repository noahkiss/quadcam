//! The Map page's native map: an Apple MapKit `MKMapView` laid over the web view. The page
//! keeps a placeholder element where the map goes and reports its rectangle (`map_frame`); this
//! module puts the map exactly there, or hides it. The pins come from the page (`map_pins`), and
//! a click on one comes back as the `map-pick` event. No token and no network of QuadCam's own:
//! MapKit draws Apple's maps.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

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

/// The map's look: Apple Maps' Standard, Satellite and Hybrid.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MapStyle {
    #[default]
    Standard,
    Satellite,
    Hybrid,
}

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

/// One pin. `id` comes back in `map-pick` when the pin is clicked.
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
}

/// A pin was clicked: its id.
#[derive(Debug, Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct MapPick(pub String);

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
    impl Host {
        pub fn place(&mut self, _: Option<super::MapFrame>) {}
        pub fn set_pins(&mut self, _: &[super::MapPin], _: super::MapStyle, _: bool) {}
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{to_points, MapFrame, MapPick, MapPin, MapStyle, PinKind};
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{
        define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSView, NSWindowOrderingMode};
    use objc2_core_location::CLLocationCoordinate2D;
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
    use objc2_map_kit::{
        MKAnnotation, MKAnnotationView, MKClusterAnnotation, MKHybridMapConfiguration,
        MKImageryMapConfiguration, MKMapConfiguration, MKMapView, MKMapViewDelegate,
        MKMarkerAnnotationView, MKPointAnnotation, MKStandardMapConfiguration,
    };
    use std::cell::RefCell;
    use std::collections::HashMap;
    use tauri::AppHandle;
    use tauri_specta::Event;

    const REUSE: &str = "pin";
    const CLUSTER: &str = "qc";

    pub struct Host {
        map: Retained<MKMapView>,
        _delegate: Retained<Delegate>,
        pins: Vec<Retained<MKPointAnnotation>>,
        webview: usize,
    }

    /// A pin's id, kind and glyph.
    type Pin = (String, PinKind, Option<String>);

    thread_local! {
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
        /// Each pin's id, kind and glyph, by its annotation's address.
        static PINS: RefCell<HashMap<usize, Pin>> = RefCell::new(HashMap::new());
    }

    pub struct Ivars {
        app: AppHandle,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "QuadCamMapDelegate"]
        #[ivars = Ivars]
        pub struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl MKMapViewDelegate for Delegate {
            #[unsafe(method_id(mapView:viewForAnnotation:))]
            fn view_for_annotation(
                &self,
                map: &MKMapView,
                annotation: &ProtocolObject<dyn MKAnnotation>,
            ) -> Option<Retained<MKAnnotationView>> {
                view_for(self.mtm(), map, annotation)
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
                    let hit = PINS.with(|s| s.borrow().get(&key).map(|v| v.0.clone()));
                    map.deselectAnnotation_animated(Some(&annotation), false);
                    match hit {
                        Some(id) => {
                            let _ = MapPick(id).emit(&self.ivars().app);
                        }
                        None => {
                            // A cluster MapKit made: zoom in to its pins.
                            let obj: &NSObject =
                                &*(Retained::as_ptr(&annotation) as *const NSObject);
                            if obj.isKindOfClass(MKClusterAnnotation::class()) {
                                let cluster =
                                    &*(obj as *const NSObject as *const MKClusterAnnotation);
                                map.showAnnotations_animated(&cluster.memberAnnotations(), true);
                            }
                        }
                    }
                }
            }
        }
    );

    fn view_for(
        mtm: MainThreadMarker,
        map: &MKMapView,
        annotation: &ProtocolObject<dyn MKAnnotation>,
    ) -> Option<Retained<MKAnnotationView>> {
        let key = annotation as *const _ as *const () as usize;
        let Some((kind, glyph)) = PINS.with(|s| s.borrow().get(&key).map(|v| (v.1, v.2.clone())))
        else {
            // A cluster MapKit made: its own view.
            return None;
        };
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
            Some(Retained::into_super(view))
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
            let map = unsafe {
                MKMapView::initWithFrame(
                    MKMapView::alloc(mtm),
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)),
                )
            };
            let delegate: Retained<Delegate> = {
                let this = Delegate::alloc(mtm).set_ivars(Ivars { app });
                unsafe { msg_send![super(this), init] }
            };
            unsafe {
                map.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                map.setShowsZoomControls(true);
                map.setShowsCompass(true);
                map.setShowsScale(true);
            }
            map.setHidden(true);
            // The page lays the map out; between two reports the map keeps its margins, as
            // the page's own layout does while the window is resized.
            map.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            let view: &NSView = &map;
            parent.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Above, Some(web));
            Some(Host {
                map,
                _delegate: delegate,
                pins: Vec::new(),
                webview,
            })
        }

        fn webview(&self) -> &NSView {
            // SAFETY: the window's web view outlives the host, which `with` replaces when the
            // web view changes.
            unsafe { &*(self.webview as *const NSView) }
        }

        pub fn place(&mut self, frame: Option<MapFrame>) {
            let frame = frame.filter(|f| f.w > 0.0 && f.h > 0.0);
            let Some(f) = frame else {
                self.map.setHidden(true);
                return;
            };
            let web = self.webview();
            let b = web.bounds();
            let (x, y, w, h) = to_points(&f, b.size.width, b.size.height, web.isFlipped());
            let in_web = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
            let Some(parent) = (unsafe { self.map.superview() }) else {
                return;
            };
            let rect = web.convertRect_toView(in_web, Some(&parent));
            self.map.setFrame(rect);
            self.map.setHidden(false);
        }

        pub fn set_pins(&mut self, pins: &[MapPin], style: MapStyle, fit: bool) {
            unsafe {
                let config: Retained<MKMapConfiguration> = match style {
                    MapStyle::Standard => Retained::into_super(MKStandardMapConfiguration::new()),
                    MapStyle::Satellite => Retained::into_super(MKImageryMapConfiguration::new()),
                    MapStyle::Hybrid => Retained::into_super(MKHybridMapConfiguration::new()),
                };
                self.map.setPreferredConfiguration(&config);
                let old: Vec<&ProtocolObject<dyn MKAnnotation>> = self
                    .pins
                    .iter()
                    .map(|p| ProtocolObject::from_ref(&**p))
                    .collect();
                self.map.removeAnnotations(&NSArray::from_slice(&old));
            }
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
                    ids.insert(key, (p.id.clone(), p.kind, p.glyph.clone()));
                    a
                })
                .collect();
            PINS.with(|s| *s.borrow_mut() = ids);
            let all: Vec<&ProtocolObject<dyn MKAnnotation>> = self
                .pins
                .iter()
                .map(|p| ProtocolObject::from_ref(&**p))
                .collect();
            let all = NSArray::from_slice(&all);
            unsafe {
                self.map.addAnnotations(&all);
                if fit && !self.pins.is_empty() {
                    self.map.showAnnotations_animated(&all, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
