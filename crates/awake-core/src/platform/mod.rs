use crate::{declare_verified, Diagnostics, Method, Result};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

/// Platform backend. Not `Send`: create it on the thread that uses it (on
/// Windows the sleep inhibition is bound to the calling thread).
pub trait Platform {
    /// Prevent idle system sleep (and optionally display sleep) until
    /// [`release`](Self::release). Calling again replaces the previous state.
    fn inhibit_sleep(&mut self, keep_display_on: bool) -> Result<()>;

    /// Drop every inhibition. Idempotent.
    fn release(&mut self);

    /// OS-wide seconds since the last user input, as chat apps see it.
    fn idle_seconds(&self) -> Option<u64>;

    /// Try each method from [`activity_methods`](Self::activity_methods) in
    /// order and return the first one that verifiably reset the idle timer.
    fn declare_activity(&self) -> Result<Method> {
        declare_verified(self, &self.activity_methods()).map(|a| a.method)
    }

    fn diagnostics(&self) -> Diagnostics;

    /// Candidate methods on this system, in preferred order.
    fn activity_methods(&self) -> Vec<Method>;

    /// Run one method without verification.
    fn declare_activity_with(&self, method: Method) -> Result<()>;

    /// Why nothing works and what to do about it, for error messages.
    fn failure_hint(&self) -> Option<String> {
        None
    }
}

/// The backend for the current OS.
pub fn new_platform() -> Box<dyn Platform> {
    #[cfg(target_os = "macos")]
    return Box::new(macos::MacPlatform::new());
    #[cfg(not(target_os = "macos"))]
    return Box::new(unsupported::Unsupported);
}

/// macOS: whether this process may post synthetic input events
/// (Accessibility). `None` on platforms without such a permission.
pub fn accessibility_trusted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    return Some(macos::accessibility_trusted());
    #[cfg(not(target_os = "macos"))]
    return None;
}

/// macOS: register this app in the Accessibility list (system prompt) and
/// open the matching System Settings pane. No-op elsewhere.
pub fn request_accessibility() {
    #[cfg(target_os = "macos")]
    macos::request_accessibility();
}
