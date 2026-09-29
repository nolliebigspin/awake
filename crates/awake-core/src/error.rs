use std::fmt;

use crate::Method;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The method or API does not exist on this system (wrong session type,
    /// service not running, device missing, …).
    Unavailable(String),
    /// The API exists but the OS refused us (Accessibility, /dev/uinput, …).
    PermissionDenied(String),
    /// An OS call failed unexpectedly.
    Os(String),
    /// The method reported success but the idle timer did not reset.
    Unverified {
        method: Method,
        idle_after: Option<u64>,
    },
    /// Every activity method failed. `hint` explains what the user can do.
    NoWorkingMethod {
        attempts: Vec<(Method, Error)>,
        hint: Option<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unavailable(m) => write!(f, "unavailable: {m}"),
            Error::PermissionDenied(m) => write!(f, "permission denied: {m}"),
            Error::Os(m) => write!(f, "{m}"),
            Error::Unverified { method, idle_after } => match idle_after {
                Some(s) => write!(f, "{method} ran but idle time is still {s}s"),
                None => write!(f, "{method} ran but idle time could not be re-read"),
            },
            Error::NoWorkingMethod { attempts, hint } => {
                if attempts.is_empty() {
                    write!(
                        f,
                        "no method to reset the idle timer is available on this system"
                    )?;
                } else {
                    write!(f, "no method reset the idle timer:")?;
                    for (m, e) in attempts {
                        write!(f, "\n  - {m}: {e}")?;
                    }
                }
                if let Some(h) = hint {
                    write!(f, "\n{h}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}
