//! This Mac's location, from Location Services (CoreLocation), so a place can be saved where
//! the person stands. One fix per request, to about 100 m; nothing keeps tracking. The first
//! request shows macOS's permission prompt, with the usage string in `Info.plist`; the
//! hardened runtime needs the `personal-information.location` entitlement for it.
//!
//! A process started by cargo never asks Location Services (no prompt in a test run) unless
//! `QUADCAM_LOCATION=real`.

use anyhow::{bail, Result};
use std::time::Duration;

/// One location fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    /// The radius it is good to, in metres.
    pub accuracy_m: f64,
}

/// How long to wait for a fix, the permission prompt included.
const TIMEOUT: Duration = Duration::from_secs(60);

/// True when this process may ask Location Services: not started by cargo, or
/// `QUADCAM_LOCATION=real`.
pub fn allowed(env: Option<&str>, under_cargo: bool) -> bool {
    env == Some("real") || !under_cargo
}

/// The text for a refused permission.
pub const DENIED: &str = "QuadCam may not use Location Services. Allow it in System Settings > Privacy & Security > Location Services.";

/// This Mac's location now. Waits for the permission prompt on the first call.
pub fn current() -> Result<Fix> {
    if !allowed(
        std::env::var("QUADCAM_LOCATION").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        bail!("Location Services are off in tests (QUADCAM_LOCATION=real turns them on)");
    }
    imp::current()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Fix, DENIED, TIMEOUT};
    use anyhow::{bail, Result};
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
    use objc2_core_location::{
        kCLLocationAccuracyHundredMeters, CLAuthorizationStatus, CLLocation, CLLocationManager,
        CLLocationManagerDelegate,
    };
    use objc2_foundation::{
        NSArray, NSDate, NSDefaultRunLoopMode, NSError, NSObject, NSObjectProtocol, NSRunLoop,
    };
    use std::cell::RefCell;

    type Answer = std::result::Result<Fix, String>;

    #[derive(Default)]
    struct Ivars {
        answer: RefCell<Option<Answer>>,
    }

    fn authorized(s: CLAuthorizationStatus) -> bool {
        s == CLAuthorizationStatus::AuthorizedAlways
            || s == CLAuthorizationStatus::AuthorizedWhenInUse
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class has no Drop.
        #[unsafe(super(NSObject))]
        #[name = "QuadCamLocationDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl CLLocationManagerDelegate for Delegate {
            #[unsafe(method(locationManager:didUpdateLocations:))]
            fn did_update(&self, _m: &CLLocationManager, locations: &NSArray<CLLocation>) {
                if let Some(l) = locations.lastObject() {
                    // SAFETY: reading a location's values.
                    let (c, acc) = unsafe { (l.coordinate(), l.horizontalAccuracy()) };
                    self.answer(Ok(Fix {
                        lat: c.latitude,
                        lon: c.longitude,
                        accuracy_m: acc,
                    }));
                }
            }

            #[unsafe(method(locationManager:didFailWithError:))]
            fn did_fail(&self, _m: &CLLocationManager, error: &NSError) {
                // kCLErrorLocationUnknown (0) passes: Core Location keeps trying.
                if error.code() != 0 {
                    self.answer(Err(error.localizedDescription().to_string()));
                }
            }

            #[unsafe(method(locationManagerDidChangeAuthorization:))]
            fn did_change(&self, m: &CLLocationManager) {
                // SAFETY: plain Core Location calls on a live manager.
                let s = unsafe { m.authorizationStatus() };
                if authorized(s) {
                    unsafe { m.requestLocation() };
                } else if s == CLAuthorizationStatus::Denied
                    || s == CLAuthorizationStatus::Restricted
                {
                    self.answer(Err(DENIED.into()));
                }
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars::default());
            // SAFETY: NSObject's init.
            unsafe { msg_send![super(this), init] }
        }

        fn answer(&self, a: Answer) {
            let mut slot = self.ivars().answer.borrow_mut();
            if slot.is_none() {
                *slot = Some(a);
            }
        }

        fn take(&self) -> Option<Answer> {
            self.ivars().answer.borrow_mut().take()
        }
    }

    pub fn current() -> Result<Fix> {
        // SAFETY: a class method with no arguments.
        if !unsafe { CLLocationManager::locationServicesEnabled_class() } {
            bail!("Location Services are off. Turn them on in System Settings > Privacy & Security > Location Services.");
        }
        let delegate = Delegate::new();
        // SAFETY: plain Core Location calls; the delegate outlives the manager's use of it
        // because both live to the end of this function, and the manager is stopped first.
        let manager = unsafe {
            let m = CLLocationManager::new();
            m.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            m.setDesiredAccuracy(kCLLocationAccuracyHundredMeters);
            let s = m.authorizationStatus();
            if s == CLAuthorizationStatus::NotDetermined {
                m.requestWhenInUseAuthorization();
            } else if authorized(s) {
                m.requestLocation();
            } else {
                bail!("{DENIED}");
            }
            m
        };
        // Core Location answers on the run loop of the thread that made the manager: turn it.
        let run_loop = NSRunLoop::currentRunLoop();
        let started = std::time::Instant::now();
        let answer = loop {
            if let Some(a) = delegate.take() {
                break Some(a);
            }
            if started.elapsed() > TIMEOUT {
                break None;
            }
            // SAFETY: NSDefaultRunLoopMode is a static NSString.
            run_loop.runMode_beforeDate(
                unsafe { NSDefaultRunLoopMode },
                &NSDate::dateWithTimeIntervalSinceNow(0.05),
            );
        };
        // SAFETY: stopping and detaching a live manager.
        unsafe {
            manager.stopUpdatingLocation();
            manager.setDelegate(None);
        }
        match answer {
            Some(Ok(f)) => Ok(f),
            Some(Err(e)) => bail!("{e}"),
            None => bail!(
                "Location Services gave no location within {} s",
                TIMEOUT.as_secs()
            ),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn current() -> anyhow::Result<super::Fix> {
        anyhow::bail!("Location Services need macOS")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cargo_processes_never_ask_location_services() {
        assert!(super::allowed(None, false));
        assert!(!super::allowed(None, true));
        assert!(!super::allowed(Some("yes"), true));
        assert!(super::allowed(Some("real"), true));
        if std::env::var("QUADCAM_LOCATION").as_deref() != Ok("real") {
            let e = super::current().unwrap_err();
            assert!(e.to_string().contains("off in tests"), "{e}");
        }
    }
}
