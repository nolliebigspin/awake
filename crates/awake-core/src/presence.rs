//! Turns the keeper's per-tick events into "presence lost / restored"
//! transitions, so frontends can alert once per episode instead of on every
//! failed tick.

use crate::{Error, Event, Method};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// No method could reset the idle timer, so chat apps will drop to Away.
    Lost {
        /// Every method that ran was refused by the OS or had no effect, and
        /// at least one was refused: granting a permission should fix it.
        needs_permission: bool,
        error: Error,
    },
    /// Resetting works again after a [`Lost`](Transition::Lost).
    Restored { method: Method },
}

#[derive(Debug, Default)]
pub struct PresenceMonitor {
    lost: bool,
}

impl PresenceMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the last episode ended in [`Transition::Lost`].
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    pub fn observe(&mut self, event: &Event) -> Option<Transition> {
        match event {
            Event::Error(e @ Error::NoWorkingMethod { attempts, .. }) if !self.lost => {
                self.lost = true;
                Some(Transition::Lost {
                    needs_permission: needs_permission(attempts),
                    error: e.clone(),
                })
            }
            Event::Activity(a) if self.lost => {
                self.lost = false;
                Some(Transition::Restored { method: a.method })
            }
            Event::Stopped => {
                self.lost = false;
                None
            }
            _ => None,
        }
    }
}

fn needs_permission(attempts: &[(Method, Error)]) -> bool {
    let denied = |e: &Error| matches!(e, Error::PermissionDenied(_));
    attempts.iter().any(|(_, e)| denied(e))
        && attempts
            .iter()
            .all(|(_, e)| denied(e) || matches!(e, Error::Unverified { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attempt;
    use Method::{MacDeclareUserActivity as A, MacF15Key as C, MacMouseNudge as B};

    /// What the tray logged all day on a Mac without Accessibility.
    fn no_accessibility() -> Event {
        Event::Error(Error::NoWorkingMethod {
            attempts: vec![
                (
                    A,
                    Error::Unverified {
                        method: A,
                        idle_after: Some(255),
                    },
                ),
                (B, Error::PermissionDenied("Accessibility".into())),
                (C, Error::PermissionDenied("Accessibility".into())),
            ],
            hint: Some("grant it".into()),
        })
    }

    fn os_failure() -> Event {
        Event::Error(Error::NoWorkingMethod {
            attempts: vec![(B, Error::Os("CGEvent creation failed".into()))],
            hint: None,
        })
    }

    fn activity(method: Method) -> Event {
        Event::Activity(Attempt {
            method,
            idle_before: Some(30),
            idle_after: Some(0),
            verified: true,
            failed: Vec::new(),
        })
    }

    #[test]
    fn repeated_failures_are_lost_once() {
        let mut m = PresenceMonitor::new();
        let transitions: Vec<_> = (0..286)
            .filter_map(|_| m.observe(&no_accessibility()))
            .collect();
        assert_eq!(transitions.len(), 1);
        assert!(matches!(
            transitions[0],
            Transition::Lost {
                needs_permission: true,
                ..
            }
        ));
        assert!(m.is_lost());
    }

    #[test]
    fn os_errors_do_not_ask_for_permission() {
        let mut m = PresenceMonitor::new();
        assert!(matches!(
            m.observe(&os_failure()),
            Some(Transition::Lost {
                needs_permission: false,
                ..
            })
        ));
    }

    #[test]
    fn unverified_alone_does_not_ask_for_permission() {
        let attempts = vec![(
            A,
            Error::Unverified {
                method: A,
                idle_after: Some(40),
            },
        )];
        assert!(!needs_permission(&attempts));
    }

    #[test]
    fn restored_after_lost_then_lost_again() {
        let mut m = PresenceMonitor::new();
        assert!(m.observe(&no_accessibility()).is_some());
        assert_eq!(
            m.observe(&activity(B)),
            Some(Transition::Restored { method: B })
        );
        assert_eq!(m.observe(&activity(B)), None, "still fine: no news");
        assert!(matches!(
            m.observe(&no_accessibility()),
            Some(Transition::Lost { .. })
        ));
    }

    #[test]
    fn success_without_prior_loss_is_not_news() {
        let mut m = PresenceMonitor::new();
        assert_eq!(m.observe(&activity(B)), None);
        assert!(!m.is_lost());
    }

    #[test]
    fn ticks_and_unreadable_idle_are_ignored() {
        let mut m = PresenceMonitor::new();
        assert_eq!(m.observe(&Event::Tick { idle: Some(90) }), None);
        assert_eq!(
            m.observe(&Event::Error(Error::Unavailable("no idle".into()))),
            None
        );
        assert!(!m.is_lost());
    }

    #[test]
    fn stop_ends_the_episode() {
        let mut m = PresenceMonitor::new();
        m.observe(&no_accessibility());
        assert_eq!(m.observe(&Event::Stopped), None);
        assert!(!m.is_lost());
        assert!(m.observe(&no_accessibility()).is_some());
    }
}
