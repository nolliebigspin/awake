//! Live checks against the real macOS input system: do our events really reset
//! the idle time that Teams/Slack read, and does the keeper hold it down?
//!
//! Opt-in (`mise run test-live`). They need Accessibility for the app running
//! `cargo test` (your terminal or editor) and no keyboard/mouse input for
//! about half a minute.
#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use awake_core::{
    accessibility_trusted, new_platform, wait_until_idle, Config, Error, Event, Keeper, Method,
    VERIFY_BELOW_SECS,
};

fn require_accessibility() {
    assert_eq!(
        accessibility_trusted(),
        Some(true),
        "grant Accessibility to the app running cargo test (System Settings → Privacy & \
         Security → Accessibility), then rerun"
    );
}

#[test]
#[ignore = "live: needs Accessibility and an untouched machine"]
fn mouse_nudge_resets_the_idle_timer_apps_read() {
    require_accessibility();
    let p = new_platform();
    eprintln!("Hands off the keyboard and mouse…");
    assert!(
        wait_until_idle(p.as_ref(), VERIFY_BELOW_SECS + 2, Duration::from_secs(30)),
        "the machine never went idle for {}s",
        VERIFY_BELOW_SECS + 2
    );
    let before = p.idle_seconds().unwrap();

    p.declare_activity_with(Method::MacMouseNudge)
        .expect("posting the mouse nudge");

    // idle_seconds() is max(HIDIdleTime, CGEventSource combined session): it
    // only drops once both, the latter being what Chromium/Teams read, reset.
    let deadline = Instant::now() + Duration::from_secs(1);
    let after = loop {
        let idle = p.idle_seconds().unwrap();
        if idle < VERIFY_BELOW_SECS || Instant::now() >= deadline {
            break idle;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        after < VERIFY_BELOW_SECS,
        "idle {before}s -> {after}s: the nudge did not reset the idle timer"
    );
}

#[test]
#[ignore = "live: needs Accessibility and an untouched machine"]
fn keeper_keeps_idle_below_threshold() {
    require_accessibility();
    let config = Config {
        interval: Duration::from_secs(1),
        threshold: Duration::from_secs(VERIFY_BELOW_SECS + 1),
        keep_display_on: false,
    };
    let limit = config.threshold.as_secs() + config.interval.as_secs() + 1;
    eprintln!("Hands off the keyboard and mouse for 20s…");

    let (handle, events) = Keeper::spawn(config);
    std::thread::sleep(Duration::from_secs(20));
    handle.stop();
    let events: Vec<Event> = events.try_iter().collect();

    let resets: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Activity(a) => Some(a),
            _ => None,
        })
        .collect();
    assert!(
        resets
            .iter()
            .any(|a| a.verified && a.method.synthesizes_input()),
        "no verified CGEvent reset in 20s: {events:#?}"
    );
    if let Some(e) = events.iter().find_map(|e| match e {
        Event::Error(e @ Error::NoWorkingMethod { .. }) => Some(e),
        _ => None,
    }) {
        panic!("a tick found no working method: {e}");
    }

    // Skip the first tick: the idle time before we started is not ours.
    let idles: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            Event::Tick { idle } => *idle,
            _ => None,
        })
        .skip(1)
        .collect();
    assert!(idles.len() >= 10, "too few ticks: {idles:?}");
    assert!(
        idles.iter().all(|&s| s <= limit),
        "idle time escaped the keeper (limit {limit}s): {idles:?}"
    );
    eprintln!(
        "{} resets via {:?}, idle per tick: {idles:?}",
        resets.len(),
        resets.last().map(|a| a.method)
    );
}
