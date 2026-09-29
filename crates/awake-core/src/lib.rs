//! Keep the machine — and everything running on it — awake, and keep chat
//! presence (Teams, Slack, …) at "Available".
//!
//! Two separate things have to happen for that:
//!
//! 1. **Sleep inhibition** keeps the OS from suspending, so VPNs, agents and
//!    network connections survive ([`Platform::inhibit_sleep`]).
//! 2. **Idle-timer reset.** Chat apps derive presence from the OS-wide
//!    input idle time, and inhibiting sleep does not touch that timer. So we
//!    periodically declare user activity ([`Platform::declare_activity`]) and
//!    re-read the idle time to verify the reset actually happened.
//!
//! [`Keeper`] runs both on a background thread and reports what it does as
//! [`Event`]s.

mod error;
mod keeper;
mod platform;
mod time;
mod types;

pub use error::{Error, Result};
pub use keeper::{declare_verified, Attempt, Config, Event, Keeper, KeeperHandle};
pub use platform::{accessibility_trusted, new_platform, request_accessibility, Platform};
pub use time::utc_timestamp;
pub use types::{Diagnostics, Method, MethodStatus, Permission};

/// Idle time must drop below this many seconds after declaring activity for
/// a method to count as working.
pub const VERIFY_BELOW_SECS: u64 = 2;
