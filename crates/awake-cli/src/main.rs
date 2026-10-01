mod log;
mod service;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use awake_core::{
    declare_verified, format_remaining, new_platform, parse_duration, wait_until_idle, Config,
    Error, Event, Keeper, VERIFY_BELOW_SECS,
};
use clap::{Args, Parser, Subcommand};

use crate::log::Log;

/// Keep the machine awake and chat apps (Teams, Slack) showing "Available".
///
/// Runs in the foreground until Ctrl+C: inhibits system sleep and, whenever
/// the OS input idle time exceeds --threshold, resets it and verifies the
/// reset took effect.
#[derive(Parser)]
#[command(version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Show platform, idle time, available methods and permissions.
    Status {
        /// Actually try every method and verify it resets the idle timer
        /// (asks you not to touch the keyboard or mouse for a few seconds).
        #[arg(long)]
        test: bool,
    },
    /// Start awake at login as a user-level service (no admin needed).
    Install(RunArgs),
    /// Remove the login service.
    Uninstall,
}

#[derive(Args, Clone, Debug)]
pub struct RunArgs {
    /// Seconds between idle checks.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval: u64,
    /// Reset the idle timer once it exceeds this many seconds.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(3..))]
    pub threshold: u64,
    /// Also keep the display from sleeping.
    #[arg(long)]
    pub display: bool,
    /// Stop automatically after this long, e.g. 30m, 1h, 1h30m.
    #[arg(long = "for", value_name = "DURATION", value_parser = parse_duration)]
    pub duration: Option<Duration>,
    /// Log every tick and every failed method.
    #[arg(short, long)]
    pub verbose: bool,
    /// Append the log to this file instead of stdout.
    #[arg(long, value_name = "PATH")]
    pub log_file: Option<std::path::PathBuf>,
    /// Detach from the console (used by the Windows logon task).
    #[arg(long, hide = true)]
    pub background: bool,
}

impl RunArgs {
    fn config(&self) -> Config {
        Config {
            interval: Duration::from_secs(self.interval),
            threshold: Duration::from_secs(self.threshold),
            keep_display_on: self.display,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        None => run(&cli.run),
        Some(Command::Status { test }) => status(test),
        Some(Command::Install(args)) if args.duration.is_some() => {
            Err("--for can't be combined with install".into())
        }
        Some(Command::Install(args)) => service::install(&args),
        Some(Command::Uninstall) => service::uninstall(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("awake: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &RunArgs) -> Result<(), String> {
    if args.background && service::detach_if_needed()? {
        return Ok(());
    }
    let mut log = Log::open(args.log_file.as_deref())?;
    service::write_pid_file();

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .map_err(|e| format!("cannot install Ctrl+C handler: {e}"))?;
    }

    let config = args.config();
    log.line(&format!(
        "awake {}: interval {}s, threshold {}s, display {}",
        env!("CARGO_PKG_VERSION"),
        args.interval,
        args.threshold,
        if args.display { "kept on" } else { "may sleep" }
    ));
    if let Some(d) = args.duration {
        log.line(&format!(
            "stopping automatically in {}",
            format_remaining(d)
        ));
    }
    // Wall clock, so time spent asleep anyway (lid closed) still counts.
    let deadline = args.duration.map(|d| SystemTime::now() + d);
    let (keeper, events) = Keeper::spawn(config);
    let mut printer = Printer::new(args.verbose);

    while !stop.load(Ordering::SeqCst) {
        if let Ok(event) = events.recv_timeout(Duration::from_millis(200)) {
            printer.print(&mut log, &event);
        }
        if deadline.is_some_and(|d| SystemTime::now() >= d) {
            let d = args.duration.unwrap_or_default();
            log.line(&format!("timer ended after {}", format_remaining(d)));
            break;
        }
    }
    log.line("stopping, releasing inhibitions…");
    keeper.stop();
    for event in events.try_iter() {
        printer.print(&mut log, &event);
    }
    service::remove_pid_file();
    Ok(())
}

struct Printer {
    verbose: bool,
    last_error: Option<String>,
    repeats: u32,
}

impl Printer {
    fn new(verbose: bool) -> Self {
        Printer {
            verbose,
            last_error: None,
            repeats: 0,
        }
    }

    fn print(&mut self, log: &mut Log, event: &Event) {
        match event {
            Event::Started { sleep_inhibited } => log.line(if *sleep_inhibited {
                "sleep inhibited; watching idle time"
            } else {
                "WARNING: sleep could NOT be inhibited; still resetting idle time"
            }),
            Event::Tick { idle } => {
                if self.verbose {
                    match idle {
                        Some(s) => log.line(&format!("tick: idle {s}s")),
                        None => log.line("tick: idle time unknown"),
                    }
                }
            }
            Event::Activity(a) => {
                self.last_error = None;
                let fmt = |s: Option<u64>| s.map_or("?".into(), |s| format!("{s}s"));
                let check = if a.verified { "verified" } else { "UNVERIFIED" };
                log.line(&format!(
                    "reset idle via {} ({} -> {}, {check})",
                    a.method,
                    fmt(a.idle_before),
                    fmt(a.idle_after)
                ));
                if self.verbose {
                    for (m, e) in &a.failed {
                        log.line(&format!("  tried {m}: {e}"));
                    }
                }
            }
            Event::Error(e) => {
                let text = e.to_string();
                if self.verbose || self.last_error.as_deref() != Some(&text) {
                    log.line(&format!("ERROR: {text}"));
                    self.repeats = 0;
                } else {
                    self.repeats += 1;
                    if self.repeats % 10 == 0 {
                        log.line(&format!(
                            "ERROR (still, {} more times): {text}",
                            self.repeats
                        ));
                    }
                }
                self.last_error = Some(text);
            }
            Event::ConfigChanged(_) => {}
            Event::Stopped => log.line("stopped; sleep allowed again"),
        }
    }
}

fn status(test: bool) -> Result<(), String> {
    let platform = new_platform();
    print!("{}", platform.diagnostics());
    if !test {
        println!(
            "\nRun `awake status --test` to verify which methods really reset the idle timer."
        );
        return Ok(());
    }

    println!("\nTesting activity methods. Do not touch the keyboard or mouse.");
    let mut any = false;
    for method in platform.activity_methods() {
        print!("  {:<40} ", method.name());
        let _ = std::io::Write::flush(&mut std::io::stdout());
        if !wait_until_idle(
            platform.as_ref(),
            VERIFY_BELOW_SECS + 2,
            Duration::from_secs(30),
        ) {
            println!("skipped (input detected or idle time unreadable)");
            continue;
        }
        match declare_verified(platform.as_ref(), &[method]) {
            Ok(a) if a.verified => {
                any = true;
                println!(
                    "WORKS (idle {}s -> {}s)",
                    a.idle_before.unwrap_or(0),
                    a.idle_after.unwrap_or(0)
                );
            }
            Ok(_) => println!("ran, but cannot verify (idle time unreadable)"),
            Err(Error::NoWorkingMethod { attempts, .. }) => match attempts.first() {
                Some((_, e)) => println!("FAILED: {e}"),
                None => println!("FAILED"),
            },
            Err(e) => println!("FAILED: {e}"),
        }
    }
    if !any {
        if let Some(hint) = platform.failure_hint() {
            println!("\n{hint}");
        }
    }
    Ok(())
}
