//! Linux: logind inhibitor for sleep; Mutter / XScreenSaver / freedesktop
//! ScreenSaver for idle; D-Bus SimulateUserActivity, XTest or a uinput
//! virtual pointer for activity.
//!
//! Wayland compositors deliberately give clients no way to read global idle
//! time or inject input, so on Wayland what works depends on the desktop:
//! - GNOME: idle via org.gnome.Mutter.IdleMonitor; no activity API (its
//!   `ResetIdletime` only works with MUTTER_DEBUG_RESET_IDLETIME), so only
//!   uinput works.
//! - KDE: `SimulateUserActivity` exists, but `GetSessionIdleTime` refuses
//!   to answer on Wayland, so success cannot be verified.

use std::cell::{Cell, RefCell};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use x11rb::connection::{Connection as _, RequestConnection};
use x11rb::protocol::screensaver::ConnectionExt as _;
use x11rb::protocol::xproto::Window;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedFd;

use crate::{Diagnostics, Error, Method, MethodStatus, Permission, Platform, Result};

const FDO_SS: (&str, &str, &str) = (
    "org.freedesktop.ScreenSaver",
    "/org/freedesktop/ScreenSaver",
    "org.freedesktop.ScreenSaver",
);
const MUTTER_IDLE: (&str, &str, &str) = (
    "org.gnome.Mutter.IdleMonitor",
    "/org/gnome/Mutter/IdleMonitor/Core",
    "org.gnome.Mutter.IdleMonitor",
);
const GNOME_SM: (&str, &str, &str) = (
    "org.gnome.SessionManager",
    "/org/gnome/SessionManager",
    "org.gnome.SessionManager",
);
const LOGIND: (&str, &str, &str) = (
    "org.freedesktop.login1",
    "/org/freedesktop/login1",
    "org.freedesktop.login1.Manager",
);
const UINPUT_PATH: &str = "/dev/uinput";
const WHO: &str = "awake";
const WHY: &str = "Keeping the system and its connections alive";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionType {
    Wayland,
    X11,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdleSource {
    Mutter,
    XScreenSaver,
    FdoScreenSaver,
}

impl IdleSource {
    fn name(self) -> &'static str {
        match self {
            IdleSource::Mutter => "org.gnome.Mutter.IdleMonitor.GetIdletime",
            IdleSource::XScreenSaver => "X11 XScreenSaver",
            IdleSource::FdoScreenSaver => "org.freedesktop.ScreenSaver.GetSessionIdleTime",
        }
    }
}

enum DisplayInhibit {
    Fdo(u32),
    Gnome(u32),
}

struct X11 {
    conn: RustConnection,
    root: Window,
    xtest: bool,
    screensaver: bool,
}

pub struct LinuxPlatform {
    session_type: SessionType,
    desktop: String,
    system_bus: Option<Connection>,
    session_bus: Option<Connection>,
    x11: Option<X11>,
    sleep_fd: Option<OwnedFd>,
    display: Option<DisplayInhibit>,
    idle_source: Cell<Option<IdleSource>>,
    uinput: RefCell<Option<Uinput>>,
}

fn call<B, R>(conn: &Connection, target: (&str, &str, &str), method: &str, body: &B) -> Result<R>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let (dest, path, iface) = target;
    let reply = conn
        .call_method(Some(dest), path, Some(iface), method, body)
        .map_err(|e| dbus_error(dest, method, e))?;
    reply
        .body()
        .deserialize()
        .map_err(|e| Error::Os(format!("{iface}.{method}: unexpected reply: {e}")))
}

fn dbus_error(dest: &str, method: &str, e: zbus::Error) -> Error {
    let text = format!("{dest} {method}: {e}");
    match &e {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.NotSupported" => Error::Unavailable(text),
            "org.freedesktop.DBus.Error.AccessDenied" => Error::PermissionDenied(text),
            _ => Error::Os(text),
        },
        _ => Error::Os(text),
    }
}

fn detect_session() -> (SessionType, String) {
    let var = |k| std::env::var(k).unwrap_or_default();
    let kind = match var("XDG_SESSION_TYPE").to_lowercase().as_str() {
        "wayland" => SessionType::Wayland,
        "x11" => SessionType::X11,
        _ if !var("WAYLAND_DISPLAY").is_empty() => SessionType::Wayland,
        _ if !var("DISPLAY").is_empty() => SessionType::X11,
        _ => SessionType::Other,
    };
    let desktop = var("XDG_CURRENT_DESKTOP");
    (
        kind,
        if desktop.is_empty() {
            "unknown desktop".into()
        } else {
            desktop
        },
    )
}

fn connect_x11() -> Option<X11> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let has = |name| conn.extension_information(name).ok().flatten().is_some();
    let xtest = has(x11rb::protocol::xtest::X11_EXTENSION_NAME);
    let screensaver = has(x11rb::protocol::screensaver::X11_EXTENSION_NAME);
    Some(X11 {
        conn,
        root,
        xtest,
        screensaver,
    })
}

impl LinuxPlatform {
    pub fn new() -> Self {
        let (session_type, desktop) = detect_session();
        // XWayland only sees X clients' input, so on Wayland neither its idle
        // time nor XTest reflects what the compositor (and chat apps) see.
        let x11 = (session_type == SessionType::X11)
            .then(connect_x11)
            .flatten();
        LinuxPlatform {
            session_type,
            desktop,
            system_bus: Connection::system().ok(),
            session_bus: Connection::session().ok(),
            x11,
            sleep_fd: None,
            display: None,
            idle_source: Cell::new(None),
            uinput: RefCell::new(None),
        }
    }

    fn session(&self) -> Result<&Connection> {
        self.session_bus.as_ref().ok_or_else(|| {
            Error::Unavailable("no D-Bus session bus (DBUS_SESSION_BUS_ADDRESS unset?)".into())
        })
    }

    fn has_name(&self, name: &str) -> bool {
        let Ok(conn) = self.session() else {
            return false;
        };
        call::<_, bool>(
            conn,
            (
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            ),
            "NameHasOwner",
            &(name,),
        )
        .unwrap_or(false)
    }

    fn read_idle(&self, source: IdleSource) -> Option<u64> {
        match source {
            IdleSource::Mutter => {
                let ms: u64 = call(self.session().ok()?, MUTTER_IDLE, "GetIdletime", &()).ok()?;
                Some(ms / 1000)
            }
            IdleSource::XScreenSaver => {
                let x = self.x11.as_ref().filter(|x| x.screensaver)?;
                let info = x.conn.screensaver_query_info(x.root).ok()?.reply().ok()?;
                Some(u64::from(info.ms_since_user_input) / 1000)
            }
            IdleSource::FdoScreenSaver => {
                // KDE returns milliseconds (KIdleTime::idleTime), and refuses on Wayland.
                let ms: u32 = call(self.session().ok()?, FDO_SS, "GetSessionIdleTime", &()).ok()?;
                Some(u64::from(ms) / 1000)
            }
        }
    }

    fn release_display(&mut self) {
        let Some(inhibit) = self.display.take() else {
            return;
        };
        let Ok(conn) = self.session() else { return };
        let _ = match inhibit {
            DisplayInhibit::Fdo(cookie) => call::<_, ()>(conn, FDO_SS, "UnInhibit", &(cookie,)),
            DisplayInhibit::Gnome(cookie) => call::<_, ()>(conn, GNOME_SM, "Uninhibit", &(cookie,)),
        };
    }

    fn inhibit_display(&mut self) -> Result<()> {
        let conn = self.session()?;
        let fdo = call::<_, u32>(conn, FDO_SS, "Inhibit", &(WHO, WHY));
        let inhibit = match fdo {
            Ok(cookie) => DisplayInhibit::Fdo(cookie),
            Err(fdo_err) => {
                // Flag 8 = inhibit the session being marked idle (screen blank).
                match call::<_, u32>(conn, GNOME_SM, "Inhibit", &(WHO, 0u32, WHY, 8u32)) {
                    Ok(cookie) => DisplayInhibit::Gnome(cookie),
                    Err(gnome_err) => {
                        return Err(Error::Os(format!(
                            "system sleep is inhibited, but keeping the display on failed: \
                             {fdo_err}; {gnome_err}"
                        )))
                    }
                }
            }
        };
        self.display = Some(inhibit);
        Ok(())
    }

    fn with_uinput(&self) -> Result<()> {
        let mut slot = self.uinput.borrow_mut();
        if slot.is_none() {
            *slot = Some(Uinput::create()?);
        }
        let dev = slot.as_mut().expect("just created");
        if let Err(e) = dev.nudge() {
            *slot = None;
            return Err(e);
        }
        Ok(())
    }

    fn x11_nudge(&self) -> Result<()> {
        let x = self
            .x11
            .as_ref()
            .ok_or_else(|| Error::Unavailable("not an X11 session".into()))?;
        if !x.xtest {
            return Err(Error::Unavailable(
                "X server lacks the XTEST extension".into(),
            ));
        }
        const MOTION_NOTIFY: u8 = 6;
        let err = |e: &dyn std::fmt::Display| Error::Os(format!("XTest: {e}"));
        for dx in [1i16, -1] {
            // detail = 1 makes the motion relative.
            x.conn
                .xtest_fake_input(MOTION_NOTIFY, 1, 0, x11rb::NONE, dx, 0, 0)
                .map_err(|e| err(&e))?
                .check()
                .map_err(|e| err(&e))?;
        }
        x.conn.flush().map_err(|e| err(&e))?;
        Ok(())
    }
}

impl Platform for LinuxPlatform {
    fn inhibit_sleep(&mut self, keep_display_on: bool) -> Result<()> {
        if self.sleep_fd.is_none() {
            let conn = self.system_bus.as_ref().ok_or_else(|| {
                Error::Unavailable("no D-Bus system bus; is systemd-logind running?".into())
            })?;
            let what = "sleep:idle";
            let fd: OwnedFd = call(conn, LOGIND, "Inhibit", &(what, WHO, WHY, "block"))?;
            self.sleep_fd = Some(fd);
        }
        match (keep_display_on, self.display.is_some()) {
            (true, false) => self.inhibit_display()?,
            (false, true) => self.release_display(),
            _ => {}
        }
        Ok(())
    }

    fn release(&mut self) {
        self.release_display();
        // Closing the fd releases the logind inhibitor.
        self.sleep_fd = None;
        *self.uinput.borrow_mut() = None;
    }

    fn idle_seconds(&self) -> Option<u64> {
        if let Some(src) = self.idle_source.get() {
            if let Some(s) = self.read_idle(src) {
                return Some(s);
            }
        }
        let order = [
            IdleSource::Mutter,
            IdleSource::XScreenSaver,
            IdleSource::FdoScreenSaver,
        ];
        for src in order {
            if let Some(s) = self.read_idle(src) {
                self.idle_source.set(Some(src));
                return Some(s);
            }
        }
        self.idle_source.set(None);
        None
    }

    fn activity_methods(&self) -> Vec<Method> {
        let mut m = Vec::new();
        if self.has_name(FDO_SS.0) {
            m.push(Method::DbusSimulateUserActivity);
        }
        if self.x11.as_ref().is_some_and(|x| x.xtest) {
            m.push(Method::X11XTest);
        }
        m.push(Method::Uinput);
        m
    }

    fn declare_activity_with(&self, method: Method) -> Result<()> {
        match method {
            Method::DbusSimulateUserActivity => {
                call::<_, ()>(self.session()?, FDO_SS, "SimulateUserActivity", &())
            }
            Method::X11XTest => self.x11_nudge(),
            Method::Uinput => self.with_uinput(),
            m => Err(Error::Unavailable(format!("{m} is not a Linux method"))),
        }
    }

    fn failure_hint(&self) -> Option<String> {
        let uinput = format!(
            "To use a virtual pointer, give your user write access to {UINPUT_PATH} \
             (install packaging/linux/60-awake-uinput.rules, run `sudo modprobe uinput`, \
             then log out and back in)."
        );
        Some(match self.session_type {
            SessionType::Wayland => format!(
                "Wayland ({}) does not let applications fake input or reset the idle timer. \
                 GNOME has no API for it; KDE only offers SimulateUserActivity. {uinput} \
                 Alternatively log into an X11 session.",
                self.desktop
            ),
            _ => uinput,
        })
    }

    fn diagnostics(&self) -> Diagnostics {
        let session = format!(
            "{} ({})",
            match self.session_type {
                SessionType::Wayland => "wayland",
                SessionType::X11 => "x11",
                SessionType::Other => "no graphical session detected",
            },
            self.desktop
        );
        let idle_seconds = self.idle_seconds();
        let fdo = self.has_name(FDO_SS.0);
        let x11 = self.x11.as_ref();
        let uinput_writable = uinput_writable();

        let mut notes = Vec::new();
        if self.session_type == SessionType::Wayland {
            notes.push(
                "Wayland: X11 idle time and XTest are ignored because XWayland does not see \
                 native Wayland input."
                    .into(),
            );
            if idle_seconds.is_none() {
                notes.push(
                    "The idle time cannot be read on this compositor, so resets cannot be \
                     verified; awake declares activity every interval instead."
                        .into(),
                );
            }
        }
        if self.system_bus.is_none() {
            notes.push("No system D-Bus: sleep cannot be inhibited through logind.".into());
        }

        Diagnostics {
            os: format!("Linux ({})", std::env::consts::ARCH),
            session: Some(session),
            idle_source: self.idle_source.get().map(|s| s.name().into()),
            idle_seconds,
            sleep_inhibit: "logind Inhibit(sleep:idle) (+ ScreenSaver/SessionManager idle inhibit)"
                .into(),
            methods: vec![
                MethodStatus {
                    method: Method::DbusSimulateUserActivity,
                    available: fdo,
                    detail: if fdo {
                        "org.freedesktop.ScreenSaver present".into()
                    } else {
                        "org.freedesktop.ScreenSaver not on the session bus".into()
                    },
                },
                MethodStatus {
                    method: Method::X11XTest,
                    available: x11.is_some_and(|x| x.xtest),
                    detail: match (self.session_type, x11) {
                        (SessionType::X11, Some(x)) if x.xtest => "X11 session".into(),
                        (SessionType::X11, Some(_)) => "XTEST extension missing".into(),
                        (SessionType::X11, None) => "cannot connect to $DISPLAY".into(),
                        _ => "only used in X11 sessions".into(),
                    },
                },
                MethodStatus {
                    method: Method::Uinput,
                    available: uinput_writable,
                    detail: if uinput_writable {
                        format!("{UINPUT_PATH} writable")
                    } else {
                        format!("{UINPUT_PATH} missing or not writable")
                    },
                },
            ],
            permissions: vec![Permission {
                name: format!("write access to {UINPUT_PATH}"),
                granted: uinput_writable,
                hint: "only needed when no other method works (typical on GNOME Wayland); \
                       see the README's Linux section"
                    .into(),
            }],
            notes,
        }
    }
}

impl Drop for LinuxPlatform {
    fn drop(&mut self) {
        self.release();
    }
}

fn uinput_writable() -> bool {
    let path = c"/dev/uinput";
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

// --- uinput virtual pointer -------------------------------------------------

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const BTN_LEFT: u16 = 0x110;
const BUS_VIRTUAL: u16 = 0x06;

// _IOW('U', nr, size) / _IO('U', nr) with the generic ioctl encoding.
const fn iow(nr: u32, size: u32) -> u32 {
    (1 << 30) | (size << 16) | ((b'U' as u32) << 8) | nr
}
const UI_SET_EVBIT: u32 = iow(100, 4);
const UI_SET_KEYBIT: u32 = iow(101, 4);
const UI_SET_RELBIT: u32 = iow(102, 4);
const UI_DEV_SETUP: u32 = iow(3, std::mem::size_of::<UinputSetup>() as u32);
const UI_DEV_CREATE: u32 = 0x5501;
const UI_DEV_DESTROY: u32 = 0x5502;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

struct Uinput {
    file: File,
}

impl Uinput {
    fn create() -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(UINPUT_PATH)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => Error::PermissionDenied(format!(
                    "{UINPUT_PATH}: {e}; see the README's Linux section for the udev rule"
                )),
                std::io::ErrorKind::NotFound => Error::Unavailable(format!(
                    "{UINPUT_PATH} does not exist; run `sudo modprobe uinput`"
                )),
                _ => Error::Os(format!("{UINPUT_PATH}: {e}")),
            })?;
        let fd = file.as_raw_fd();
        let ioctl = |req: u32, arg: libc::c_ulong| -> Result<()> {
            if unsafe { libc::ioctl(fd, req as _, arg) } < 0 {
                Err(Error::Os(format!(
                    "uinput ioctl {req:#x}: {}",
                    std::io::Error::last_os_error()
                )))
            } else {
                Ok(())
            }
        };
        // A relative pointer with a button is what libinput treats as a mouse.
        ioctl(UI_SET_EVBIT, EV_KEY.into())?;
        ioctl(UI_SET_KEYBIT, BTN_LEFT.into())?;
        ioctl(UI_SET_EVBIT, EV_REL.into())?;
        ioctl(UI_SET_RELBIT, REL_X.into())?;
        ioctl(UI_SET_RELBIT, REL_Y.into())?;

        let mut setup = UinputSetup {
            id: InputId {
                bustype: BUS_VIRTUAL,
                vendor: 0x1209,
                product: 0xa3a7,
                version: 1,
            },
            name: [0; 80],
            ff_effects_max: 0,
        };
        let name = b"awake virtual pointer";
        setup.name[..name.len()].copy_from_slice(name);
        ioctl(UI_DEV_SETUP, &setup as *const _ as libc::c_ulong)?;
        ioctl(UI_DEV_CREATE, 0)?;
        // Give udev and the compositor time to pick up the new device.
        std::thread::sleep(Duration::from_millis(800));
        Ok(Uinput { file })
    }

    fn nudge(&mut self) -> Result<()> {
        for dx in [1, -1] {
            self.emit(EV_REL, REL_X, dx)?;
            self.emit(EV_SYN, SYN_REPORT, 0)?;
        }
        Ok(())
    }

    fn emit(&mut self, kind: u16, code: u16, value: i32) -> Result<()> {
        let ev = libc::input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: kind,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &ev as *const _ as *const u8,
                std::mem::size_of::<libc::input_event>(),
            )
        };
        self.file
            .write_all(bytes)
            .map_err(|e| Error::Os(format!("uinput write: {e}")))
    }
}

impl Drop for Uinput {
    fn drop(&mut self) {
        unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY as _) };
    }
}
