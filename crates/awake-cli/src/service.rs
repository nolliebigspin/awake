//! User-level login service: launchd LaunchAgent (macOS), systemd --user
//! unit (Linux), Task Scheduler logon task (Windows). Never needs admin.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::RunArgs;

#[cfg(target_os = "macos")]
const LABEL: &str = "dev.awake.cli";

/// The binary the service should launch. Homebrew runs us from a versioned
/// Cellar path that disappears on upgrade, so prefer the stable symlink.
fn service_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate own binary: {e}"))?;
    let s = exe.to_string_lossy();
    if let Some(i) = s.find("/Cellar/") {
        let stable = Path::new(&s[..i]).join("bin").join("awake");
        if stable.exists() {
            return Ok(stable);
        }
    }
    if s.contains("/target/debug/") || s.contains("\\target\\debug\\") {
        eprintln!("note: installing a development build ({s})");
    }
    Ok(exe)
}

fn service_args(args: &RunArgs) -> Vec<String> {
    let mut v = vec![
        "--interval".into(),
        args.interval.to_string(),
        "--threshold".into(),
        args.threshold.to_string(),
    ];
    if args.display {
        v.push("--display".into());
    }
    if args.verbose {
        v.push("--verbose".into());
    }
    v
}

#[cfg(unix)]
fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn run_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = if err.trim().is_empty() {
            String::from_utf8_lossy(&out.stdout)
        } else {
            err
        };
        Err(format!("{program} {}: {}", args.join(" "), msg.trim()))
    }
}

fn write(path: &Path, content: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, content).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(any(target_os = "macos", windows, test))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// --- macOS ----------------------------------------------------------------

#[cfg(target_os = "macos")]
fn plist_path() -> Result<PathBuf, String> {
    Ok(home()?
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

#[cfg(target_os = "macos")]
fn launchd_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn launchd_plist(exe: &Path, args: &[String], log: &Path) -> String {
    let mut program = format!(
        "    <string>{}</string>\n",
        xml_escape(&exe.to_string_lossy())
    );
    for a in args {
        program += &format!("    <string>{}</string>\n", xml_escape(a));
    }
    let log = xml_escape(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{program}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
pub fn install(args: &RunArgs) -> Result<(), String> {
    let exe = service_exe()?;
    let plist = plist_path()?;
    let log = home()?.join("Library/Logs/awake.log");
    write(&plist, launchd_plist(&exe, &service_args(args), &log))?;
    let domain = launchd_domain();
    let _ = run_cmd("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
    run_cmd(
        "launchctl",
        &["bootstrap", &domain, &plist.to_string_lossy()],
    )?;
    println!("Installed LaunchAgent {}", plist.display());
    println!("Log: {}", log.display());
    println!(
        "\nIMPORTANT: the agent runs {} directly, so that binary needs Accessibility access\n\
         (System Settings → Privacy & Security → Accessibility → + → press ⌘⇧G and paste the\n\
         path). Without it only IOPMAssertionDeclareUserActivity is available, which may not\n\
         keep Teams/Slack Available. Check the log for 'verified' lines.",
        exe.display()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<(), String> {
    let plist = plist_path()?;
    let _ = run_cmd(
        "launchctl",
        &["bootout", &format!("{}/{LABEL}", launchd_domain())],
    );
    remove_if_exists(&plist)
}

// --- Linux ----------------------------------------------------------------

#[cfg(target_os = "linux")]
fn unit_path() -> Result<PathBuf, String> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => home()?.join(".config"),
    };
    Ok(base.join("systemd/user/awake.service"))
}

/// Quote one ExecStart word for systemd.
#[cfg(any(target_os = "linux", test))]
fn systemd_quote(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "linux")]
fn systemd_unit(exe: &Path, args: &[String]) -> String {
    let mut exec = systemd_quote(&exe.to_string_lossy());
    for a in args {
        exec.push(' ');
        exec += &systemd_quote(a);
    }
    format!(
        "[Unit]\n\
         Description=awake: keep the machine awake and chat presence Available\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n"
    )
}

#[cfg(target_os = "linux")]
pub fn install(args: &RunArgs) -> Result<(), String> {
    let exe = service_exe()?;
    let unit = unit_path()?;
    write(&unit, systemd_unit(&exe, &service_args(args)))?;
    run_cmd("systemctl", &["--user", "daemon-reload"])?;
    run_cmd("systemctl", &["--user", "enable", "--now", "awake.service"])?;
    println!("Installed {}", unit.display());
    println!("Logs: journalctl --user -u awake -f");
    if run_cmd(
        "systemctl",
        &["--user", "is-active", "graphical-session.target"],
    )
    .is_err()
    {
        println!(
            "\nWARNING: graphical-session.target is not active, so the service will not start.\n\
             Your desktop may not integrate with systemd (common with minimal window managers).\n\
             Start `awake` from your session's autostart instead, or edit the unit to use\n\
             WantedBy=default.target and import DISPLAY/WAYLAND_DISPLAY with\n\
             `systemctl --user import-environment`."
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<(), String> {
    let _ = run_cmd(
        "systemctl",
        &["--user", "disable", "--now", "awake.service"],
    );
    remove_if_exists(&unit_path()?)?;
    let _ = run_cmd("systemctl", &["--user", "daemon-reload"]);
    Ok(())
}

// --- Windows --------------------------------------------------------------

#[cfg(windows)]
const TASK_NAME: &str = "awake";
#[cfg(windows)]
const DETACHED_ENV: &str = "AWAKE_DETACHED";

#[cfg(windows)]
fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("awake"))
        .ok_or_else(|| "LOCALAPPDATA is not set".into())
}

#[cfg(windows)]
fn windows_quote(s: &str) -> String {
    if s.is_empty() || s.contains([' ', '\t', '"']) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_owned()
    }
}

#[cfg(windows)]
fn task_xml(exe: &Path, args: &[String], user: &str) -> String {
    let arguments = args
        .iter()
        .map(|a| windows_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    let (exe, arguments, user) = (
        xml_escape(&exe.to_string_lossy()),
        xml_escape(&arguments),
        xml_escape(user),
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>awake: keep the machine awake and chat presence Available</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(windows)]
pub fn install(args: &RunArgs) -> Result<(), String> {
    let exe = service_exe()?;
    let dir = data_dir()?;
    let log = dir.join("awake.log");
    let mut task_args = service_args(args);
    task_args.extend([
        "--background".into(),
        "--log-file".into(),
        log.to_string_lossy().into_owned(),
    ]);

    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) => format!("{d}\\{u}"),
        (_, Ok(u)) => u,
        _ => return Err("USERNAME is not set".into()),
    };
    // schtasks wants UTF-16 with a BOM.
    let xml = task_xml(&exe, &task_args, &user);
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    let xml_path = std::env::temp_dir().join("awake-task.xml");
    write(&xml_path, bytes)?;

    let created = run_cmd(
        "schtasks",
        &[
            "/Create",
            "/TN",
            TASK_NAME,
            "/XML",
            &xml_path.to_string_lossy(),
            "/F",
        ],
    );
    let _ = std::fs::remove_file(&xml_path);
    created?;
    run_cmd("schtasks", &["/Run", "/TN", TASK_NAME])?;
    println!("Installed logon task '{TASK_NAME}' and started it.");
    println!("Log: {}", log.display());
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> Result<(), String> {
    let deleted = run_cmd("schtasks", &["/Delete", "/TN", TASK_NAME, "/F"]);
    let pid_file = data_dir()?.join("awake.pid");
    if let Ok(pid) = std::fs::read_to_string(&pid_file) {
        let pid = pid.trim();
        if pid != std::process::id().to_string() {
            let _ = run_cmd("taskkill", &["/PID", pid, "/F"]);
        }
        let _ = std::fs::remove_file(&pid_file);
    }
    deleted.map(|_| println!("Removed logon task '{TASK_NAME}'."))
}

/// Relaunch without a console window. Returns true in the parent, which
/// should exit.
#[cfg(windows)]
pub fn detach_if_needed() -> Result<bool, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    if std::env::var_os(DETACHED_ENV).is_some() {
        return Ok(false);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(DETACHED_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| format!("cannot detach: {e}"))?;
    Ok(true)
}

#[cfg(windows)]
pub fn write_pid_file() {
    if std::env::var_os(DETACHED_ENV).is_some() {
        if let Ok(dir) = data_dir() {
            let _ = write(&dir.join("awake.pid"), std::process::id().to_string());
        }
    }
}

#[cfg(windows)]
pub fn remove_pid_file() {
    if std::env::var_os(DETACHED_ENV).is_some() {
        if let Ok(dir) = data_dir() {
            let _ = std::fs::remove_file(dir.join("awake.pid"));
        }
    }
}

// --- shared / fallbacks ---------------------------------------------------

#[cfg(not(windows))]
pub fn detach_if_needed() -> Result<bool, String> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn write_pid_file() {}
#[cfg(not(windows))]
pub fn remove_pid_file() {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remove_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            println!("Removed {}", path.display());
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Nothing to remove ({} does not exist)", path.display());
            Ok(())
        }
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn install(_: &RunArgs) -> Result<(), String> {
    Err("install is not supported on this OS".into())
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn uninstall() -> Result<(), String> {
    Err("uninstall is not supported on this OS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_quoting() {
        assert_eq!(
            systemd_quote("/opt/my apps/awake"),
            "\"/opt/my apps/awake\""
        );
        assert_eq!(systemd_quote("100%$x\""), "\"100%%$$x\\\"\"");
    }

    #[test]
    fn xml_escaping() {
        assert_eq!(xml_escape("a<b>&\"c"), "a&lt;b&gt;&amp;&quot;c");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_contains_args() {
        let p = launchd_plist(
            Path::new("/opt/homebrew/bin/awake"),
            &["--interval".into(), "60".into()],
            Path::new("/tmp/a.log"),
        );
        assert!(
            p.contains("<string>/opt/homebrew/bin/awake</string>\n    <string>--interval</string>")
        );
        assert!(p.contains("<key>RunAtLoad</key>"));
    }
}
