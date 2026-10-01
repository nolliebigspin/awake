use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{new_platform, Error, Method, Platform, Result, VERIFY_BELOW_SECS};

/// How long to keep re-reading the idle time after declaring activity.
const VERIFY_WINDOW: Duration = Duration::from_millis(1000);
const VERIFY_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// How often to check the idle time.
    pub interval: Duration,
    /// Declare activity once the idle time exceeds this.
    pub threshold: Duration,
    pub keep_display_on: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            interval: Duration::from_secs(60),
            threshold: Duration::from_secs(20),
            keep_display_on: false,
        }
    }
}

impl Config {
    /// Clamp to values the verification logic can work with.
    fn sanitized(mut self) -> Self {
        self.interval = self.interval.max(Duration::from_secs(1));
        // Below this we could not tell our reset apart from real input.
        self.threshold = self
            .threshold
            .max(Duration::from_secs(VERIFY_BELOW_SECS + 1));
        self
    }
}

/// Outcome of one successful activity declaration.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub method: Method,
    pub idle_before: Option<u64>,
    pub idle_after: Option<u64>,
    /// False when the idle time could not be read, so success is assumed.
    pub verified: bool,
    /// Methods tried before `method`, and why they did not count.
    pub failed: Vec<(Method, Error)>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Started { sleep_inhibited: bool },
    Tick { idle: Option<u64> },
    Activity(Attempt),
    Error(Error),
    ConfigChanged(Config),
    Stopped,
}

/// Try `order` until one method verifiably drops the idle time below
/// [`VERIFY_BELOW_SECS`].
pub fn declare_verified<P: Platform + ?Sized>(p: &P, order: &[Method]) -> Result<Attempt> {
    let idle_before = p.idle_seconds();
    let can_verify = idle_before.is_some_and(|s| s >= VERIFY_BELOW_SECS);
    let mut failed = Vec::new();

    for &method in order {
        if let Err(e) = p.declare_activity_with(method) {
            failed.push((method, e));
            continue;
        }
        if !can_verify {
            return Ok(Attempt {
                method,
                idle_before,
                idle_after: p.idle_seconds(),
                verified: false,
                failed,
            });
        }
        let idle_after = wait_for_reset(p);
        if idle_after.is_some_and(|s| s < VERIFY_BELOW_SECS) {
            return Ok(Attempt {
                method,
                idle_before,
                idle_after,
                verified: true,
                failed,
            });
        }
        failed.push((method, Error::Unverified { method, idle_after }));
    }

    Err(Error::NoWorkingMethod {
        attempts: failed,
        hint: p.failure_hint(),
    })
}

/// Wait (at most `timeout`) until the idle time reaches `secs`, so that a
/// reset can be measured. False on input or when the idle time is unreadable.
pub fn wait_until_idle<P: Platform + ?Sized>(p: &P, secs: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match p.idle_seconds() {
            Some(s) if s >= secs => return true,
            Some(_) => thread::sleep(Duration::from_millis(250)),
            None => return false,
        }
    }
    false
}

fn wait_for_reset<P: Platform + ?Sized>(p: &P) -> Option<u64> {
    let deadline = Instant::now() + VERIFY_WINDOW;
    loop {
        thread::sleep(VERIFY_POLL);
        let idle = p.idle_seconds();
        if idle.is_some_and(|s| s < VERIFY_BELOW_SECS) || Instant::now() >= deadline {
            return idle;
        }
    }
}

enum Command {
    SetConfig(Config),
    Stop,
}

/// Runs the keep-awake loop: inhibit sleep, and every `interval` reset the
/// idle timer if it exceeds `threshold`, remembering which method works.
pub struct Keeper {
    platform: Box<dyn Platform>,
    config: Config,
    preferred: Option<Method>,
    events: Sender<Event>,
    warned_no_idle: bool,
}

impl Keeper {
    pub fn new(platform: Box<dyn Platform>, config: Config, events: Sender<Event>) -> Self {
        Keeper {
            platform,
            config: config.sanitized(),
            preferred: None,
            events,
            warned_no_idle: false,
        }
    }

    /// Run on a background thread with the current OS backend.
    pub fn spawn(config: Config) -> (KeeperHandle, Receiver<Event>) {
        Self::spawn_with(new_platform, config)
    }

    /// Like [`spawn`](Self::spawn) with a custom backend. The backend is
    /// created on the keeper thread.
    pub fn spawn_with<F>(make_platform: F, config: Config) -> (KeeperHandle, Receiver<Event>)
    where
        F: FnOnce() -> Box<dyn Platform> + Send + 'static,
    {
        let (event_tx, event_rx) = mpsc::channel();
        let (control_tx, control_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("awake-keeper".into())
            .spawn(move || Keeper::new(make_platform(), config, event_tx).run(control_rx))
            .expect("spawn keeper thread");
        let handle = KeeperHandle {
            control: control_tx,
            thread: Some(thread),
        };
        (handle, event_rx)
    }

    pub fn config(&self) -> Config {
        self.config
    }

    /// The method that worked last, tried first next time.
    pub fn preferred(&self) -> Option<Method> {
        self.preferred
    }

    pub fn start(&mut self) {
        let sleep_inhibited = self.inhibit();
        self.emit(Event::Started { sleep_inhibited });
    }

    pub fn stop(&mut self) {
        self.platform.release();
        self.emit(Event::Stopped);
    }

    pub fn apply(&mut self, config: Config) {
        let config = config.sanitized();
        let display_changed = config.keep_display_on != self.config.keep_display_on;
        self.config = config;
        if display_changed {
            self.inhibit();
        }
        self.emit(Event::ConfigChanged(config));
    }

    /// One iteration: read idle time, and reset it if over the threshold.
    pub fn tick(&mut self) {
        let idle = self.platform.idle_seconds();
        self.emit(Event::Tick { idle });

        match idle {
            Some(s) if s <= self.config.threshold.as_secs() => return,
            Some(_) => {}
            None if !self.warned_no_idle => {
                self.warned_no_idle = true;
                self.emit(Event::Error(Error::Unavailable(
                    "cannot read the system idle time; declaring activity every interval \
                     without being able to verify it works"
                        .into(),
                )));
            }
            None => {}
        }

        let mut order = self.platform.activity_methods();
        if let Some(p) = self.preferred {
            if let Some(i) = order.iter().position(|&m| m == p) {
                order.remove(i);
                order.insert(0, p);
            }
        }

        match declare_verified(self.platform.as_ref(), &order) {
            Ok(attempt) => {
                self.preferred = Some(attempt.method);
                self.emit(Event::Activity(attempt));
            }
            Err(e) => {
                self.preferred = None;
                self.emit(Event::Error(e));
            }
        }
    }

    fn inhibit(&mut self) -> bool {
        match self.platform.inhibit_sleep(self.config.keep_display_on) {
            Ok(()) => true,
            Err(e) => {
                self.emit(Event::Error(e));
                false
            }
        }
    }

    fn run(mut self, control: Receiver<Command>) {
        self.start();
        loop {
            self.tick();
            let next = Instant::now() + self.config.interval;
            // Wait for the next tick, applying config changes as they come.
            loop {
                let wait = next.saturating_duration_since(Instant::now());
                match control.recv_timeout(wait) {
                    Ok(Command::SetConfig(c)) => {
                        let interval_changed = c.interval != self.config.interval;
                        self.apply(c);
                        if interval_changed {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => {
                        self.stop();
                        return;
                    }
                }
            }
        }
    }

    fn emit(&self, event: Event) {
        // A frontend that stopped listening is not our problem.
        let _ = self.events.send(event);
    }
}

/// Controls a keeper thread. Dropping it stops the keeper and releases all
/// inhibitions.
pub struct KeeperHandle {
    control: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl KeeperHandle {
    pub fn set_config(&self, config: Config) {
        let _ = self.control.send(Command::SetConfig(config));
    }

    /// Stop and wait for the release to finish.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.control.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for KeeperHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Diagnostics;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// Fake backend: `working` methods reset idle to 0, `broken` ones error,
    /// `denied` ones are refused by the OS (no Accessibility), anything else
    /// "succeeds" without touching the idle time.
    struct Fake {
        idle: Rc<Cell<Option<u64>>>,
        methods: Vec<Method>,
        working: Vec<Method>,
        broken: Vec<Method>,
        denied: Rc<RefCell<Vec<Method>>>,
        calls: Rc<RefCell<Vec<Method>>>,
        inhibited: Rc<Cell<Option<bool>>>,
    }

    impl Platform for Fake {
        fn inhibit_sleep(&mut self, keep_display_on: bool) -> Result<()> {
            self.inhibited.set(Some(keep_display_on));
            Ok(())
        }
        fn release(&mut self) {
            self.inhibited.set(None);
        }
        fn idle_seconds(&self) -> Option<u64> {
            self.idle.get()
        }
        fn diagnostics(&self) -> Diagnostics {
            Diagnostics::default()
        }
        fn activity_methods(&self) -> Vec<Method> {
            self.methods.clone()
        }
        fn declare_activity_with(&self, m: Method) -> Result<()> {
            self.calls.borrow_mut().push(m);
            if self.broken.contains(&m) {
                return Err(Error::Os("boom".into()));
            }
            if self.denied.borrow().contains(&m) {
                return Err(Error::PermissionDenied("Accessibility".into()));
            }
            if self.working.contains(&m) && self.idle.get().is_some() {
                self.idle.set(Some(0));
            }
            Ok(())
        }
        fn failure_hint(&self) -> Option<String> {
            Some("try harder".into())
        }
    }

    use Method::{MacDeclareUserActivity as A, MacF15Key as C, MacMouseNudge as B};

    struct Rig {
        keeper: Keeper,
        idle: Rc<Cell<Option<u64>>>,
        calls: Rc<RefCell<Vec<Method>>>,
        inhibited: Rc<Cell<Option<bool>>>,
        denied: Rc<RefCell<Vec<Method>>>,
        events: Receiver<Event>,
    }

    fn rig(working: &[Method], broken: &[Method], idle: Option<u64>) -> Rig {
        let idle = Rc::new(Cell::new(idle));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let inhibited = Rc::new(Cell::new(None));
        let denied = Rc::new(RefCell::new(Vec::new()));
        let fake = Fake {
            idle: idle.clone(),
            methods: vec![A, B, C],
            working: working.to_vec(),
            broken: broken.to_vec(),
            denied: denied.clone(),
            calls: calls.clone(),
            inhibited: inhibited.clone(),
        };
        let (tx, events) = mpsc::channel();
        let keeper = Keeper::new(Box::new(fake), Config::default(), tx);
        Rig {
            keeper,
            idle,
            calls,
            inhibited,
            denied,
            events,
        }
    }

    fn drain(rx: &Receiver<Event>) -> Vec<Event> {
        rx.try_iter().collect()
    }

    #[test]
    fn below_threshold_does_nothing() {
        let mut r = rig(&[A], &[], Some(5));
        r.keeper.tick();
        assert!(r.calls.borrow().is_empty());
    }

    #[test]
    fn remembers_first_verified_method() {
        let mut r = rig(&[B, C], &[A], Some(100));
        r.keeper.tick();
        assert_eq!(*r.calls.borrow(), vec![A, B]);
        assert_eq!(r.keeper.preferred(), Some(B));

        r.calls.borrow_mut().clear();
        r.idle.set(Some(100));
        r.keeper.tick();
        assert_eq!(
            *r.calls.borrow(),
            vec![B],
            "preferred method is tried first"
        );
    }

    #[test]
    fn success_without_idle_drop_is_not_trusted() {
        // A "succeeds" but the idle time does not move; C really works.
        let mut r = rig(&[C], &[], Some(100));
        r.keeper.tick();
        assert_eq!(r.keeper.preferred(), Some(C));
        let events = drain(&r.events);
        let attempt = events
            .iter()
            .find_map(|e| match e {
                Event::Activity(a) => Some(a),
                _ => None,
            })
            .unwrap();
        assert!(attempt.verified);
        assert_eq!(attempt.idle_after, Some(0));
        assert!(matches!(
            attempt.failed[0].1,
            Error::Unverified { method: A, .. }
        ));
    }

    #[test]
    fn nothing_works_reports_every_attempt_and_hint() {
        let mut r = rig(&[], &[A], Some(100));
        r.keeper.tick();
        assert_eq!(r.keeper.preferred(), None);
        let err = drain(&r.events)
            .into_iter()
            .find_map(|e| match e {
                Event::Error(e) => Some(e),
                _ => None,
            })
            .unwrap();
        let msg = err.to_string();
        assert!(msg.contains("boom") && msg.contains("try harder"), "{msg}");
        match err {
            Error::NoWorkingMethod { attempts, .. } => assert_eq!(attempts.len(), 3),
            e => panic!("unexpected {e:?}"),
        }
    }

    /// The 0.2.0 field failure: Accessibility missing, so only the
    /// permission-free method runs and it does not move the idle time. The
    /// user must hear about it once, and again when it works after granting.
    #[test]
    fn missing_accessibility_is_reported_once_and_recovery_detected() {
        use crate::{PresenceMonitor, Transition};

        let mut r = rig(&[B, C], &[], Some(100));
        r.denied.borrow_mut().extend([B, C]);
        let mut monitor = PresenceMonitor::new();
        let mut transitions = Vec::new();
        for _ in 0..5 {
            r.idle.set(Some(100));
            r.keeper.tick();
            transitions.extend(drain(&r.events).iter().filter_map(|e| monitor.observe(e)));
        }
        assert_eq!(transitions.len(), 1, "{transitions:?}");
        match &transitions[0] {
            Transition::Lost {
                needs_permission: true,
                error,
            } => assert!(error.to_string().contains("try harder"), "hint is kept"),
            t => panic!("unexpected {t:?}"),
        }

        // The user grants Accessibility.
        r.denied.borrow_mut().clear();
        r.idle.set(Some(100));
        r.keeper.tick();
        let after: Vec<_> = drain(&r.events)
            .iter()
            .filter_map(|e| monitor.observe(e))
            .collect();
        assert_eq!(after, vec![Transition::Restored { method: B }]);
        assert_eq!(r.keeper.preferred(), Some(B));
    }

    #[test]
    fn unknown_idle_warns_once_and_runs_unverified() {
        let mut r = rig(&[], &[A], None);
        r.keeper.tick();
        r.keeper.tick();
        let events = drain(&r.events);
        let warnings = events
            .iter()
            .filter(|e| matches!(e, Event::Error(Error::Unavailable(_))))
            .count();
        assert_eq!(warnings, 1);
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Activity(a) if a.method == B && !a.verified)));
    }

    #[test]
    fn display_toggle_reinhibits_and_stop_releases() {
        let mut r = rig(&[A], &[], Some(0));
        r.keeper.start();
        assert_eq!(r.inhibited.get(), Some(false));
        r.keeper.apply(Config {
            keep_display_on: true,
            ..Config::default()
        });
        assert_eq!(r.inhibited.get(), Some(true));
        r.keeper.stop();
        assert_eq!(r.inhibited.get(), None);
    }

    #[test]
    fn threshold_is_clamped_above_verification_bound() {
        let mut r = rig(&[A], &[], Some(0));
        r.keeper.apply(Config {
            threshold: Duration::ZERO,
            ..Config::default()
        });
        assert!(r.keeper.config().threshold.as_secs() > VERIFY_BELOW_SECS);
    }
}
