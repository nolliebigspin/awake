//! macOS: IOPMAssertions for sleep, HIDIdleTime + CGEventSource for idle,
//! and DeclareUserActivity / CGEvent posting for activity.

use std::cell::Cell;
use std::ffi::{c_char, c_void, CStr};
use std::process::Command;

use core_foundation::base::{kCFAllocatorDefault, CFAllocatorRef, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef, CFMutableDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use crate::{Diagnostics, Error, Method, MethodStatus, Permission, Platform, Result};

type IOReturn = i32;
type IOPMAssertionID = u32;
type IoObject = u32;
type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

const K_IO_RETURN_SUCCESS: IOReturn = 0;
const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
const K_IOPM_USER_ACTIVE_LOCAL: u32 = 0;
const K_IO_MAIN_PORT_DEFAULT: u32 = 0;

const K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION: i32 = 0;
const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM: i32 = 1;
const K_CG_ANY_INPUT_EVENT_TYPE: u32 = !0;
const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
const K_CG_HID_EVENT_TAP: u32 = 0;
const KEYCODE_F15: u16 = 113;

const ACCESSIBILITY_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        level: u32,
        name: CFStringRef,
        id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(id: IOPMAssertionID) -> IOReturn;
    fn IOPMAssertionDeclareUserActivity(
        name: CFStringRef,
        user_type: u32,
        id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingService(main_port: u32, matching: CFDictionaryRef) -> IoObject;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> CFTypeRef;
    fn IOObjectRelease(object: IoObject) -> IOReturn;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    fn CGEventSourceCreate(state: i32) -> CGEventSourceRef;
    fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: u32,
        position: CGPoint,
        button: u32,
    ) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(source: CGEventSourceRef, keycode: u16, down: bool)
        -> CGEventRef;
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 || CGPreflightPostEventAccess() }
}

pub fn request_accessibility() {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef());
        // CGEventPost checks the separate PostEvent service; register for it too.
        CGRequestPostEventAccess();
    }
    let _ = Command::new("open").arg(ACCESSIBILITY_PANE).status();
}

/// Drop this bundle's Accessibility and PostEvent entries. System Settings can
/// show an entry as enabled although it belongs to another build of the app
/// (different code signature) or was never actually saved; macOS then denies
/// us while the switch looks on. Resetting lets the next request register the
/// running build afresh.
///
/// An open System Settings window keeps showing the old list, and switching
/// a row there that no longer exists saves nothing, so it is closed too.
pub fn reset_accessibility(bundle_id: &str) -> Result<()> {
    let settings_open = || {
        Command::new("/usr/bin/pgrep")
            .args(["-x", "System Settings"])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if settings_open() {
        let _ = Command::new("/usr/bin/pkill")
            .args(["-x", "System Settings"])
            .status();
        // Let it exit, or the pane we open next lands in the closing window.
        for _ in 0..20 {
            if !settings_open() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    for service in ["Accessibility", "PostEvent"] {
        let out = Command::new("/usr/bin/tccutil")
            .args(["reset", service, bundle_id])
            .output()
            .map_err(|e| Error::Os(format!("tccutil: {e}")))?;
        if !out.status.success() {
            return Err(Error::Os(format!(
                "tccutil reset {service} {bundle_id}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
    }
    Ok(())
}

pub struct MacPlatform {
    system: Option<IOPMAssertionID>,
    display: Option<IOPMAssertionID>,
    /// DeclareUserActivity hands back an ID that later calls refresh.
    activity: Cell<IOPMAssertionID>,
}

impl MacPlatform {
    pub fn new() -> Self {
        MacPlatform {
            system: None,
            display: None,
            activity: Cell::new(0),
        }
    }
}

fn reason() -> CFString {
    CFString::from_static_string("awake: keeping the system and its connections alive")
}

fn create_assertion(kind: &'static str) -> Result<IOPMAssertionID> {
    let kind_cf = CFString::from_static_string(kind);
    let mut id = 0;
    let rc = unsafe {
        IOPMAssertionCreateWithName(
            kind_cf.as_concrete_TypeRef(),
            K_IOPM_ASSERTION_LEVEL_ON,
            reason().as_concrete_TypeRef(),
            &mut id,
        )
    };
    if rc == K_IO_RETURN_SUCCESS {
        Ok(id)
    } else {
        Err(Error::Os(format!(
            "IOPMAssertionCreateWithName({kind}) failed: {rc:#x}"
        )))
    }
}

fn release_assertion(id: &mut Option<IOPMAssertionID>) {
    if let Some(id) = id.take() {
        unsafe { IOPMAssertionRelease(id) };
    }
}

/// `HIDIdleTime` from the IOHIDSystem registry entry, in nanoseconds.
fn hid_idle_nanos() -> Option<u64> {
    unsafe {
        let matching = IOServiceMatching(c"IOHIDSystem".as_ptr());
        if matching.is_null() {
            return None;
        }
        // Consumes `matching`.
        let service = IOServiceGetMatchingService(K_IO_MAIN_PORT_DEFAULT, matching as _);
        if service == 0 {
            return None;
        }
        let key = CFString::from_static_string("HIDIdleTime");
        let prop = IORegistryEntryCreateCFProperty(
            service,
            key.as_concrete_TypeRef(),
            kCFAllocatorDefault,
            0,
        );
        IOObjectRelease(service);
        if prop.is_null() {
            return None;
        }
        let value = CFType::wrap_under_create_rule(prop);
        if let Some(n) = value.downcast::<CFNumber>() {
            return n.to_i64().map(|v| v.max(0) as u64);
        }
        // Very old systems report raw bytes.
        let data = value.downcast::<CFData>()?;
        let bytes: [u8; 8] = data.bytes().try_into().ok()?;
        Some(u64::from_ne_bytes(bytes))
    }
}

/// What Chromium/Electron (Slack, Teams web) read.
fn cg_idle_seconds() -> Option<f64> {
    let s = unsafe {
        CGEventSourceSecondsSinceLastEventType(
            K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION,
            K_CG_ANY_INPUT_EVENT_TYPE,
        )
    };
    s.is_finite().then_some(s.max(0.0))
}

fn require_post_access() -> Result<()> {
    if accessibility_trusted() {
        Ok(())
    } else {
        Err(Error::PermissionDenied(
            "Accessibility access not granted".into(),
        ))
    }
}

fn accessibility_hint() -> String {
    "posting input events needs Accessibility access. Open System Settings → Privacy & \
     Security → Accessibility and enable the app running awake (your terminal app for the \
     CLI, the `awake` binary itself for the LaunchAgent, or Awake.app for the tray). If it \
     already looks enabled there, macOS is not applying that entry (it belongs to another \
     build, or System Settings shows a stale list): remove it with −, reopen System \
     Settings and add it again, or use \"Fix Accessibility access…\" in the tray menu."
        .into()
}

/// Post events created by `make` and release them.
fn post(make: impl Fn(CGEventSourceRef) -> Vec<CGEventRef>) -> Result<()> {
    require_post_access()?;
    unsafe {
        let source = CGEventSourceCreate(K_CG_EVENT_SOURCE_STATE_HID_SYSTEM);
        let events = make(source);
        let ok = !events.is_empty() && events.iter().all(|e| !e.is_null());
        for &e in &events {
            if !e.is_null() {
                if ok {
                    CGEventPost(K_CG_HID_EVENT_TAP, e);
                }
                CFRelease(e);
            }
        }
        if !source.is_null() {
            CFRelease(source);
        }
        if ok {
            Ok(())
        } else {
            Err(Error::Os("CGEvent creation failed".into()))
        }
    }
}

fn os_version() -> String {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    let version = if rc == 0 {
        CStr::from_bytes_until_nul(&buf)
            .ok()
            .and_then(|s| s.to_str().ok())
            .unwrap_or("?")
            .to_owned()
    } else {
        "?".into()
    };
    format!("macOS {version} ({})", std::env::consts::ARCH)
}

impl Platform for MacPlatform {
    fn inhibit_sleep(&mut self, keep_display_on: bool) -> Result<()> {
        if self.system.is_none() {
            self.system = Some(create_assertion("PreventUserIdleSystemSleep")?);
        }
        match (keep_display_on, self.display.is_some()) {
            (true, false) => self.display = Some(create_assertion("PreventUserIdleDisplaySleep")?),
            (false, true) => release_assertion(&mut self.display),
            _ => {}
        }
        Ok(())
    }

    fn release(&mut self) {
        release_assertion(&mut self.system);
        release_assertion(&mut self.display);
        let mut activity = Some(self.activity.replace(0)).filter(|&id| id != 0);
        release_assertion(&mut activity);
    }

    fn idle_seconds(&self) -> Option<u64> {
        // Take the larger of the two: we only count a reset that every app sees.
        let hid = hid_idle_nanos().map(|ns| ns / 1_000_000_000);
        let cg = cg_idle_seconds().map(|s| s as u64);
        match (hid, cg) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    fn activity_methods(&self) -> Vec<Method> {
        vec![
            Method::MacDeclareUserActivity,
            Method::MacMouseNudge,
            Method::MacF15Key,
        ]
    }

    fn declare_activity_with(&self, method: Method) -> Result<()> {
        match method {
            Method::MacDeclareUserActivity => {
                let mut id = self.activity.get();
                let rc = unsafe {
                    IOPMAssertionDeclareUserActivity(
                        reason().as_concrete_TypeRef(),
                        K_IOPM_USER_ACTIVE_LOCAL,
                        &mut id,
                    )
                };
                if rc != K_IO_RETURN_SUCCESS {
                    return Err(Error::Os(format!(
                        "IOPMAssertionDeclareUserActivity: {rc:#x}"
                    )));
                }
                self.activity.set(id);
                Ok(())
            }
            Method::MacMouseNudge => post(|src| unsafe {
                let here = CGEventCreate(std::ptr::null_mut());
                if here.is_null() {
                    return vec![];
                }
                let p = CGEventGetLocation(here);
                CFRelease(here);
                let nudged = CGPoint {
                    x: p.x + 1.0,
                    y: p.y,
                };
                vec![
                    CGEventCreateMouseEvent(
                        src,
                        K_CG_EVENT_MOUSE_MOVED,
                        nudged,
                        K_CG_MOUSE_BUTTON_LEFT,
                    ),
                    CGEventCreateMouseEvent(src, K_CG_EVENT_MOUSE_MOVED, p, K_CG_MOUSE_BUTTON_LEFT),
                ]
            }),
            Method::MacF15Key => post(|src| unsafe {
                vec![
                    CGEventCreateKeyboardEvent(src, KEYCODE_F15, true),
                    CGEventCreateKeyboardEvent(src, KEYCODE_F15, false),
                ]
            }),
            m => Err(Error::Unavailable(format!("{m} is not a macOS method"))),
        }
    }

    fn failure_hint(&self) -> Option<String> {
        (!accessibility_trusted()).then(accessibility_hint)
    }

    fn diagnostics(&self) -> Diagnostics {
        let trusted = accessibility_trusted();
        let hid = hid_idle_nanos().is_some();
        let cg = cg_idle_seconds().is_some();
        let idle_source = match (hid, cg) {
            (true, true) => Some("max(IOKit HIDIdleTime, CGEventSource combined session)".into()),
            (true, false) => Some("IOKit HIDIdleTime".into()),
            (false, true) => Some("CGEventSource combined session".into()),
            (false, false) => None,
        };
        let needs_ax = if trusted {
            "Accessibility granted".to_owned()
        } else {
            "needs Accessibility permission".to_owned()
        };
        Diagnostics {
            os: os_version(),
            session: None,
            idle_source,
            idle_seconds: self.idle_seconds(),
            sleep_inhibit: "IOPMAssertion PreventUserIdleSystemSleep (+ PreventUserIdleDisplaySleep)"
                .into(),
            methods: vec![
                MethodStatus {
                    method: Method::MacDeclareUserActivity,
                    available: true,
                    detail: "no permission needed; may not reset the timer apps read (verified at runtime)".into(),
                },
                MethodStatus {
                    method: Method::MacMouseNudge,
                    available: trusted,
                    detail: needs_ax.clone(),
                },
                MethodStatus {
                    method: Method::MacF15Key,
                    available: trusted,
                    detail: format!("{needs_ax}; F15 can change brightness on some keyboards"),
                },
            ],
            permissions: vec![Permission {
                name: "Accessibility".into(),
                granted: trusted,
                hint: accessibility_hint(),
            }],
            notes: Vec::new(),
        }
    }
}

impl Drop for MacPlatform {
    fn drop(&mut self) {
        self.release();
    }
}
