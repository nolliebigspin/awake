use std::fmt;

/// A way of telling the OS "the user is active". Defined on every platform so
/// frontends can display and log them uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// macOS `IOPMAssertionDeclareUserActivity`.
    MacDeclareUserActivity,
    /// macOS CGEvent: move the pointer 1px and back.
    MacMouseNudge,
    /// macOS CGEvent: press and release F15 (keycode 113).
    MacF15Key,
    /// Windows `SendInput` F15 key press.
    WinF15Key,
    /// Windows `SendInput` 1px relative mouse move and back.
    WinMouseNudge,
    /// Linux `org.freedesktop.ScreenSaver.SimulateUserActivity` (KDE, Xfce, …).
    DbusSimulateUserActivity,
    /// Linux X11 XTest fake 1px pointer motion and back.
    X11XTest,
    /// Linux virtual pointer via `/dev/uinput` (works on Wayland if permitted).
    Uinput,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::MacDeclareUserActivity => "IOPMAssertionDeclareUserActivity",
            Method::MacMouseNudge => "CGEvent mouse nudge",
            Method::MacF15Key => "CGEvent F15 key",
            Method::WinF15Key => "SendInput F15 key",
            Method::WinMouseNudge => "SendInput mouse nudge",
            Method::DbusSimulateUserActivity => "D-Bus ScreenSaver.SimulateUserActivity",
            Method::X11XTest => "X11 XTest pointer nudge",
            Method::Uinput => "uinput virtual pointer",
        }
    }

    /// Whether this method synthesizes input events (as opposed to a
    /// first-class "user is active" API). EDR tools may flag these.
    pub fn synthesizes_input(self) -> bool {
        !matches!(
            self,
            Method::MacDeclareUserActivity | Method::DbusSimulateUserActivity
        )
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone)]
pub struct MethodStatus {
    pub method: Method,
    /// Best static guess; the Keeper still verifies at runtime.
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct Permission {
    pub name: String,
    pub granted: bool,
    /// What to do if not granted.
    pub hint: String,
}

#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    /// e.g. "macOS 15.2 (aarch64)".
    pub os: String,
    /// Linux: "wayland (GNOME)", "x11 (KDE)", …
    pub session: Option<String>,
    /// Which API `idle_seconds` reads from, if any works.
    pub idle_source: Option<String>,
    pub idle_seconds: Option<u64>,
    /// How sleep is inhibited on this platform.
    pub sleep_inhibit: String,
    /// Candidate activity methods in the order they will be tried.
    pub methods: Vec<MethodStatus>,
    pub permissions: Vec<Permission>,
    /// Limitations and warnings worth showing the user.
    pub notes: Vec<String>,
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Platform:      {}", self.os)?;
        if let Some(s) = &self.session {
            writeln!(f, "Session:       {s}")?;
        }
        match (self.idle_seconds, &self.idle_source) {
            (Some(s), Some(src)) => writeln!(f, "Idle time:     {s}s (via {src})")?,
            _ => writeln!(f, "Idle time:     unavailable")?,
        }
        writeln!(f, "Sleep inhibit: {}", self.sleep_inhibit)?;
        writeln!(f, "Activity methods (in order):")?;
        for m in &self.methods {
            let mark = if m.available { "ok " } else { "-- " };
            writeln!(f, "  {mark} {:<40} {}", m.method.name(), m.detail)?;
        }
        if !self.permissions.is_empty() {
            writeln!(f, "Permissions:")?;
            for p in &self.permissions {
                if p.granted {
                    writeln!(f, "  ok  {}", p.name)?;
                } else {
                    writeln!(f, "  !!  {}: NOT granted. {}", p.name, p.hint)?;
                }
            }
        }
        for n in &self.notes {
            writeln!(f, "Note: {n}")?;
        }
        Ok(())
    }
}
