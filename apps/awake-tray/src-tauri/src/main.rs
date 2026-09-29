//! Awake tray app: a menu-bar/tray-only frontend for awake-core.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use awake_core::{
    accessibility_trusted, format_remaining, new_platform, request_accessibility, utc_timestamp,
    Config, Error, Event, Keeper, KeeperHandle,
};
use serde_json::json;
use tauri::image::Image;
use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, Menu, MenuBuilder, MenuItem, MenuItemBuilder,
    SubmenuBuilder,
};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_store::StoreExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const TRAY_ID: &str = "awake";
const STORE_FILE: &str = "settings.json";
const INTERVALS: [u64; 3] = [30, 60, 120];
const THRESHOLD_SECS: u64 = 20;
/// Menu refresh rate. A whole number of ticks per second keeps the idle and
/// countdown numbers changing exactly one second apart.
const TICK: Duration = Duration::from_millis(250);
/// Accessibility changes are rare; check every 2 s.
const AX_CHECK_EVERY: u32 = 8;
/// "Stop after" presets: minutes and menu label.
const TIMERS: [(u64, &str); 6] = [
    (30, "30 minutes"),
    (60, "1 hour"),
    (180, "3 hours"),
    (360, "6 hours"),
    (720, "12 hours"),
    (1440, "24 hours"),
];

/// A running "Stop after" timer. Not persisted: a relaunch forgets it.
#[derive(Clone, Copy, Debug)]
struct Timer {
    minutes: u64,
    /// Wall clock, so time asleep anyway (lid closed) still counts.
    ends: SystemTime,
}

impl Timer {
    fn start(minutes: u64) -> Self {
        Timer {
            minutes,
            ends: SystemTime::now() + Duration::from_secs(minutes * 60),
        }
    }

    fn remaining(self) -> Duration {
        self.ends
            .duration_since(SystemTime::now())
            .unwrap_or_default()
    }
}

/// Presets offered in the menu; debug builds add a 1-minute one for testing.
fn timer_presets() -> Vec<(u64, &'static str)> {
    let mut presets = Vec::new();
    if cfg!(debug_assertions) {
        presets.push((1, "1 minute (debug)"));
    }
    presets.extend(TIMERS);
    presets
}

fn timer_label(minutes: u64) -> String {
    timer_presets()
        .into_iter()
        .find(|(m, _)| *m == minutes)
        .map(|(_, label)| label.to_owned())
        .unwrap_or_else(|| format_remaining(Duration::from_secs(minutes * 60)))
}

#[derive(Clone, Copy, Debug)]
struct Settings {
    enabled: bool,
    interval: u64,
    keep_display_on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        // First launch: the user started Awake to keep awake.
        Settings {
            enabled: true,
            interval: 60,
            keep_display_on: false,
        }
    }
}

impl Settings {
    fn config(self) -> Config {
        Config {
            interval: Duration::from_secs(self.interval),
            threshold: Duration::from_secs(THRESHOLD_SECS),
            keep_display_on: self.keep_display_on,
        }
    }
}

/// Menu items we update after building the menu.
struct Items {
    toggle: CheckMenuItem<Wry>,
    /// "Never" is minutes 0.
    timers: Vec<(u64, CheckMenuItem<Wry>)>,
    idle: MenuItem<Wry>,
    status: MenuItem<Wry>,
    intervals: Vec<(u64, CheckMenuItem<Wry>)>,
    display: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    update: MenuItem<Wry>,
}

struct Texts {
    toggle: String,
    idle: String,
    status: String,
    update: String,
}

#[derive(Default)]
struct AppState {
    settings: Mutex<Settings>,
    timer: Mutex<Option<Timer>>,
    keeper: Mutex<Option<KeeperHandle>>,
    items: Mutex<Option<Items>>,
    texts: Mutex<Option<Texts>>,
    pending_update: Mutex<Option<Update>>,
    ax_trusted: Mutex<Option<bool>>,
    log_path: Mutex<Option<PathBuf>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn main() {
    let app = tauri::Builder::default()
        // Must be first so a second launch exits before touching anything.
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            setup(app.handle())?;
            Ok(())
        })
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .build(tauri::generate_context!())
        .expect("failed to build Awake");

    app.run(|app, event| match event {
        // No windows exist; only an explicit Quit may end the app.
        RunEvent::ExitRequested {
            api, code: None, ..
        } => api.prevent_exit(),
        RunEvent::Exit => stop_keeper_blocking(app),
        _ => {}
    });
}

fn setup(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    *lock(&state.log_path) = app.path().app_log_dir().ok().map(|d| d.join("awake.log"));
    let settings = load_settings(app);
    *lock(&state.settings) = settings;
    *lock(&state.ax_trusted) = accessibility_trusted();
    *lock(&state.texts) = Some(Texts {
        toggle: "Keep awake".into(),
        idle: "Idle: –".into(),
        status: "Starting…".into(),
        update: "Check for updates".into(),
    });
    log(
        app,
        &format!("Awake {} started", app.package_info().version),
    );

    let menu = rebuild_menu(app)?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon(settings.enabled))
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Awake")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;
    drop(tray);

    apply(app);
    spawn_poller(app.clone());
    Ok(())
}

// --- settings ---------------------------------------------------------------

fn load_settings(app: &AppHandle) -> Settings {
    let d = Settings::default();
    let Ok(store) = app.store(STORE_FILE) else {
        return d;
    };
    Settings {
        enabled: store
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(d.enabled),
        interval: store
            .get("interval")
            .and_then(|v| v.as_u64())
            .filter(|i| INTERVALS.contains(i))
            .unwrap_or(d.interval),
        keep_display_on: store
            .get("keep_display_on")
            .and_then(|v| v.as_bool())
            .unwrap_or(d.keep_display_on),
    }
}

fn save_settings(app: &AppHandle, s: Settings) {
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    store.set("enabled", json!(s.enabled));
    store.set("interval", json!(s.interval));
    store.set("keep_display_on", json!(s.keep_display_on));
    if let Err(e) = store.save() {
        log(app, &format!("could not save settings: {e}"));
    }
}

// --- keeper -----------------------------------------------------------------

/// Bring the keeper, icon and menu in line with the current settings.
fn apply(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = *lock(&state.settings);
    save_settings(app, s);
    {
        let mut keeper = lock(&state.keeper);
        if s.enabled {
            match keeper.as_ref() {
                Some(k) => k.set_config(s.config()),
                None => {
                    set_status(app, "Starting…");
                    *keeper = Some(start_keeper(app, s.config()));
                }
            }
        } else if let Some(k) = keeper.take() {
            // Joining can take ~1s mid-verification; keep the menu responsive.
            std::thread::spawn(move || k.stop());
            set_status(app, "Off: the system may sleep");
        }
    }
    sync_menu(app);
}

fn start_keeper(app: &AppHandle, config: Config) -> KeeperHandle {
    let (handle, events) = Keeper::spawn(config);
    let app = app.clone();
    std::thread::spawn(move || {
        for event in events {
            on_keeper_event(&app, event);
        }
    });
    handle
}

fn stop_keeper_blocking(app: &AppHandle) {
    let state = app.state::<AppState>();
    let keeper = lock(&state.keeper).take();
    if let Some(k) = keeper {
        k.stop();
    }
}

fn on_keeper_event(app: &AppHandle, event: Event) {
    match event {
        Event::Started { sleep_inhibited } => {
            if sleep_inhibited {
                log(app, "sleep inhibited");
            } else {
                log(app, "WARNING: sleep could not be inhibited");
            }
        }
        Event::Activity(a) => {
            let verified = if a.verified { "" } else { " (unverified)" };
            set_status(app, &format!("Presence kept via {}{verified}", a.method));
            log(
                app,
                &format!(
                    "reset idle via {} ({:?}s -> {:?}s){verified}",
                    a.method, a.idle_before, a.idle_after
                ),
            );
        }
        Event::Error(e) => {
            set_status(app, &short_error(&e));
            log(app, &format!("ERROR: {e}"));
        }
        Event::Stopped => log(app, "stopped; sleep allowed again"),
        Event::Tick { .. } | Event::ConfigChanged(_) => {}
    }
}

fn short_error(e: &Error) -> String {
    match e {
        Error::NoWorkingMethod { .. } if accessibility_trusted() == Some(false) => {
            "⚠ Needs Accessibility access".into()
        }
        Error::NoWorkingMethod { .. } => "⚠ Could not reset idle timer (see log)".into(),
        Error::Unavailable(_) => "⚠ Idle time unreadable; resets unverified".into(),
        e => {
            let text = e.to_string();
            let first: String = text.lines().next().unwrap_or("").chars().take(48).collect();
            format!("⚠ {first}")
        }
    }
}

/// Live idle line and timer countdown, plus Accessibility changes (which
/// add/remove a menu item). Menu text is only touched when it changes.
fn spawn_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let platform = new_platform();
        let mut start = Instant::now();
        let mut n: u32 = 0;
        loop {
            check_timer(&app);
            let idle = match platform.idle_seconds() {
                Some(s) => format!("Idle: {s}s"),
                None => "Idle: unknown".into(),
            };
            set_text(&app, |t| &mut t.idle, &idle, |i| &i.idle);

            if n % AX_CHECK_EVERY == 0 {
                let trusted = accessibility_trusted();
                let state = app.state::<AppState>();
                let changed = std::mem::replace(&mut *lock(&state.ax_trusted), trusted) != trusted;
                if changed {
                    log(&app, &format!("Accessibility trusted: {trusted:?}"));
                    let _ = rebuild_menu(&app);
                }
            }

            // Sleep until the next point on a fixed grid, not for a fixed
            // time after the work, so the schedule never drifts.
            n = n.wrapping_add(1);
            let next = start + TICK * n;
            match next.checked_duration_since(Instant::now()) {
                Some(wait) => std::thread::sleep(wait),
                // Fell behind (e.g. a stall): restart the grid from now.
                None => {
                    start = Instant::now();
                    n = 0;
                }
            }
        }
    });
}

// --- timer ------------------------------------------------------------------

/// Stop when the timer has run out; otherwise refresh the countdown.
fn check_timer(app: &AppHandle) {
    let state = app.state::<AppState>();
    let expired = {
        let mut timer = lock(&state.timer);
        match *timer {
            Some(t) if SystemTime::now() >= t.ends => timer.take(),
            _ => None,
        }
    };
    let Some(t) = expired else {
        sync_countdown(app);
        return;
    };
    let label = timer_label(t.minutes);
    log(app, &format!("timer ended after {label}"));
    lock(&state.settings).enabled = false;
    apply(app);
    set_status(app, "Off: timer ended");
    let notified = app
        .notification()
        .builder()
        .title("Awake stopped")
        .body(format!(
            "Stopped after {label}. Your computer may sleep again."
        ))
        .show();
    if let Err(e) = notified {
        log(app, &format!("could not show notification: {e}"));
    }
}

fn remaining(app: &AppHandle) -> Option<String> {
    let timer = *lock(&app.state::<AppState>().timer);
    timer.map(|t| format_remaining(t.remaining()))
}

/// "Keep awake · 2h 59m left" and the tooltip, only touched when they change.
fn sync_countdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    let enabled = lock(&state.settings).enabled;
    let left = remaining(app);
    let toggle = match &left {
        Some(r) => format!("Keep awake · {r} left"),
        None => "Keep awake".into(),
    };
    {
        let mut texts = lock(&state.texts);
        let Some(t) = texts.as_mut() else { return };
        if t.toggle == toggle {
            return;
        }
        t.toggle = toggle.clone();
    }
    if let Some(items) = lock(&state.items).as_ref() {
        let _ = items.toggle.set_text(&toggle);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip(enabled, left.as_deref())));
    }
}

fn tooltip(enabled: bool, left: Option<&str>) -> String {
    match (enabled, left) {
        (true, Some(r)) => format!("Awake: on, stops in {r}"),
        (true, None) => "Awake: on".into(),
        (false, _) => "Awake: off".into(),
    }
}

// --- menu -------------------------------------------------------------------

fn tray_icon(on: bool) -> Image<'static> {
    let bytes: &'static [u8] = match (cfg!(target_os = "macos"), on) {
        (true, true) => include_bytes!("../icons/tray-on-template.png"),
        (true, false) => include_bytes!("../icons/tray-off-template.png"),
        (false, true) => include_bytes!("../icons/tray-on.png"),
        (false, false) => include_bytes!("../icons/tray-off.png"),
    };
    Image::from_bytes(bytes).expect("embedded tray icon is a valid PNG")
}

/// Build the menu from state and attach it to the tray (if it exists yet).
fn rebuild_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let s = *lock(&state.settings);
    let ax = *lock(&state.ax_trusted);
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    let timer_minutes = lock(&state.timer).map_or(0, |t| t.minutes);
    let (toggle_text, idle_text, status_text, update_text) = {
        let t = lock(&state.texts);
        let t = t.as_ref().expect("texts initialised in setup");
        (
            t.toggle.clone(),
            t.idle.clone(),
            t.status.clone(),
            t.update.clone(),
        )
    };

    let toggle = CheckMenuItemBuilder::with_id("toggle", toggle_text)
        .checked(s.enabled)
        .build(app)?;
    let timers = std::iter::once((0, "Never"))
        .chain(timer_presets())
        .map(|(m, label)| {
            CheckMenuItemBuilder::with_id(format!("timer-{m}"), label)
                .checked(timer_minutes == m)
                .build(app)
                .map(|item| (m, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let mut timer_menu = SubmenuBuilder::new(app, "Stop after");
    for (m, item) in &timers {
        timer_menu = timer_menu.item(item);
        if *m == 0 {
            timer_menu = timer_menu.separator();
        }
    }
    let timer_menu = timer_menu.build()?;
    let idle = MenuItemBuilder::with_id("idle", idle_text)
        .enabled(false)
        .build(app)?;
    let status = MenuItemBuilder::with_id("status", status_text)
        .enabled(false)
        .build(app)?;
    let intervals = INTERVALS
        .iter()
        .map(|&i| {
            CheckMenuItemBuilder::with_id(format!("interval-{i}"), format!("{i} seconds"))
                .checked(s.interval == i)
                .build(app)
                .map(|item| (i, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let mut interval_menu = SubmenuBuilder::new(app, "Check interval");
    for (_, item) in &intervals {
        interval_menu = interval_menu.item(item);
    }
    let interval_menu = interval_menu.build()?;
    let display = CheckMenuItemBuilder::with_id("display", "Keep display on")
        .checked(s.keep_display_on)
        .build(app)?;
    let autostart_item = CheckMenuItemBuilder::with_id("autostart", "Launch at login")
        .checked(autostart)
        .build(app)?;
    let update = MenuItemBuilder::with_id("update", update_text).build(app)?;

    let mut builder = MenuBuilder::new(app)
        .item(&toggle)
        .item(&timer_menu)
        .item(&idle)
        .item(&status)
        .separator()
        .item(&interval_menu)
        .item(&display)
        .item(&autostart_item)
        .separator();
    if ax == Some(false) {
        builder = builder.text("ax", "Grant Accessibility access…");
    }
    let menu = builder
        .text("log", "Show log")
        .item(&update)
        .separator()
        .text("quit", "Quit Awake")
        .build()?;

    *lock(&state.items) = Some(Items {
        toggle,
        timers,
        idle,
        status,
        intervals,
        display,
        autostart: autostart_item,
        update,
    });
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu.clone()))?;
    }
    Ok(menu)
}

/// Push settings into check marks and the tray icon.
fn sync_menu(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = *lock(&state.settings);
    let timer_minutes = lock(&state.timer).map_or(0, |t| t.minutes);
    let items = lock(&state.items);
    if let Some(items) = items.as_ref() {
        let _ = items.toggle.set_checked(s.enabled);
        for (m, item) in &items.timers {
            let _ = item.set_checked(*m == timer_minutes);
        }
        let _ = items.display.set_checked(s.keep_display_on);
        for (i, item) in &items.intervals {
            let _ = item.set_checked(*i == s.interval);
        }
        let _ = items
            .autostart
            .set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
    drop(items);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(tray_icon(s.enabled)));
        #[cfg(target_os = "macos")]
        let _ = tray.set_icon_as_template(true);
        let _ = tray.set_tooltip(Some(tooltip(s.enabled, remaining(app).as_deref())));
    }
    sync_countdown(app);
}

/// Update a remembered text (survives menu rebuilds) and its menu item.
fn set_text(
    app: &AppHandle,
    field: impl Fn(&mut Texts) -> &mut String,
    value: &str,
    item: impl Fn(&Items) -> &MenuItem<Wry>,
) {
    let state = app.state::<AppState>();
    {
        let mut texts = lock(&state.texts);
        let Some(t) = texts.as_mut() else { return };
        let slot = field(t);
        if slot == value {
            return;
        }
        *slot = value.to_owned();
    }
    let items = lock(&state.items);
    if let Some(items) = items.as_ref() {
        let _ = item(items).set_text(value);
    }
}

fn set_status(app: &AppHandle, value: &str) {
    set_text(app, |t| &mut t.status, value, |i| &i.status);
}

fn set_update_text(app: &AppHandle, value: &str) {
    set_text(app, |t| &mut t.update, value, |i| &i.update);
}

fn on_menu(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let mut changed = true;
    {
        let mut s = lock(&state.settings);
        match id {
            "toggle" => {
                // Switching by hand, either way, cancels a running timer.
                s.enabled = !s.enabled;
                *lock(&state.timer) = None;
            }
            "display" => s.keep_display_on = !s.keep_display_on,
            _ => {
                if let Some(i) = id.strip_prefix("interval-").and_then(|i| i.parse().ok()) {
                    s.interval = i;
                } else if let Some(m) = id.strip_prefix("timer-").and_then(|m| m.parse().ok()) {
                    let timer = (m > 0).then(|| Timer::start(m));
                    *lock(&state.timer) = timer;
                    if timer.is_some() {
                        s.enabled = true;
                    }
                } else {
                    changed = false;
                }
            }
        }
    }
    if let Some(m) = id
        .strip_prefix("timer-")
        .and_then(|m| m.parse::<u64>().ok())
    {
        match m {
            0 => log(app, "timer cleared"),
            m => log(app, &format!("timer set: stop after {}", timer_label(m))),
        }
    }
    if changed {
        apply(app);
        return;
    }
    match id {
        "autostart" => {
            let autolaunch = app.autolaunch();
            let result = if autolaunch.is_enabled().unwrap_or(false) {
                autolaunch.disable()
            } else {
                autolaunch.enable()
            };
            if let Err(e) = result {
                log(app, &format!("ERROR: launch at login: {e}"));
            }
            sync_menu(app);
        }
        "ax" => request_accessibility(),
        "log" => open_log(app),
        "update" => update(app.clone()),
        "quit" => {
            stop_keeper_blocking(app);
            app.exit(0);
        }
        _ => {}
    }
}

// --- updates ----------------------------------------------------------------

/// First click checks; if an update exists the item turns into "Install…".
fn update(app: AppHandle) {
    let pending = lock(&app.state::<AppState>().pending_update).take();
    tauri::async_runtime::spawn(async move {
        if let Some(update) = pending {
            set_update_text(&app, &format!("Installing {}…", update.version));
            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(()) => {
                    log(&app, &format!("installed update {}", update.version));
                    stop_keeper_blocking(&app);
                    app.restart();
                }
                Err(e) => {
                    log(&app, &format!("ERROR: update install failed: {e}"));
                    set_update_text(&app, "Update failed. Check again");
                }
            }
            return;
        }

        set_update_text(&app, "Checking for updates…");
        let result = match app.updater() {
            Ok(updater) => updater.check().await,
            Err(e) => Err(e),
        };
        match result {
            Ok(Some(update)) => {
                set_update_text(
                    &app,
                    &format!("Install update {} and restart", update.version),
                );
                *lock(&app.state::<AppState>().pending_update) = Some(update);
            }
            Ok(None) => set_update_text(
                &app,
                &format!("Up to date (v{}). Check again", app.package_info().version),
            ),
            Err(e) => {
                log(&app, &format!("ERROR: update check failed: {e}"));
                set_update_text(&app, "Update check failed. Retry");
            }
        }
    });
}

// --- logging ----------------------------------------------------------------

fn log(app: &AppHandle, msg: &str) {
    let line = format!("{} {msg}\n", utc_timestamp());
    eprint!("{line}");
    let path = lock(&app.state::<AppState>().log_path).clone();
    if let Some(path) = path {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

fn open_log(app: &AppHandle) {
    let Some(path) = lock(&app.state::<AppState>().log_path).clone() else {
        return;
    };
    if !path.exists() {
        log(app, "log opened");
    }
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("notepad");
        c.arg(&path);
        let _ = c.spawn();
        return;
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = cmd.arg(&path).spawn();
}
