# awake

Keep your machine awake, including VPNs, agents and network connections, **and** keep Microsoft Teams, Slack and similar apps showing **Available** instead of Away.

Two frontends share one core:

- **Awake** (tray app): a menu bar or tray icon for macOS, Windows and Linux.
- **`awake`** (CLI): runs in the foreground, or as a login service with `awake install`.

## Why "prevent sleep" is not enough

Chat apps decide whether you are Away from the **OS-wide input idle time**: how long ago the last keyboard or mouse event reached the system. Activity inside their own windows doesn't count. Caffeine-style tools only take a sleep assertion. The machine stays up, but the idle timer keeps counting, and Teams flips to Away after about 5 minutes.

awake does both jobs:

1. **Inhibits system sleep**, and optionally display sleep.
2. **Resets the idle timer** whenever it exceeds a threshold. After each reset it **re-reads the idle time to check that it dropped below 2 s**. A method that "succeeds" without moving the timer is treated as a failure, and awake falls through to the next one. The first method that verifiably works is tried first next time.

This check matters in practice. On macOS 27, `IOPMAssertionDeclareUserActivity` (what `caffeinate -u` uses) wakes the display but **does not** reset `HIDIdleTime`, so on its own it doesn't keep Teams Available. awake detects that at runtime and moves on to CGEvent input, which needs Accessibility access (see below).

### Methods per OS (tried in this order)

| OS | Sleep inhibition | Idle time source | Activity methods |
|---|---|---|---|
| macOS | `IOPMAssertion` PreventUserIdleSystemSleep (+ PreventUserIdleDisplaySleep) | max(IOKit `HIDIdleTime`, `CGEventSource` combined session) | `IOPMAssertionDeclareUserActivity` → CGEvent 1 px mouse move and back → CGEvent F15 (keycode 113) |
| Windows | `SetThreadExecutionState(ES_CONTINUOUS \| ES_SYSTEM_REQUIRED [\| ES_DISPLAY_REQUIRED])` | `GetLastInputInfo` | `SendInput` F15 → `SendInput` 1 px mouse move and back |
| Linux | logind `Inhibit("sleep:idle")` (+ ScreenSaver / GNOME SessionManager idle inhibit) | Mutter `IdleMonitor` → X11 XScreenSaver (X11 sessions) → `org.freedesktop.ScreenSaver.GetSessionIdleTime` | D-Bus `SimulateUserActivity` → XTest (X11 sessions) → `/dev/uinput` virtual pointer |

Every step is checked, and failures are reported through events, log lines and the tray status line. Nothing fails silently.

## Install

Or download from the website. Every release also has version-less copies of the tray installers, so `https://github.com/nolliebigspin/awake/releases/latest/download/<name>` always points at the latest one: `Awake-macOS.dmg`, `Awake-Windows-Setup.exe`, `Awake-Windows.msi`, `Awake-Linux-amd64.deb`, `Awake-Linux-x86_64.AppImage`.

### Tray app

| Channel | How |
|---|---|
| macOS (Homebrew) | `brew install --cask nolliebigspin/tap/awake-tray` |
| macOS (manual) | Download `Awake_<version>_universal.dmg` from [Releases](https://github.com/nolliebigspin/awake/releases) and drag Awake to Applications |
| Windows | `Awake_<version>_x64-setup.exe` (per-user NSIS, no admin) or `Awake_<version>_x64_en-US.msi` |
| Debian / Ubuntu | `sudo apt install ./Awake_<version>_amd64.deb` |
| Other Linux | `chmod +x Awake_<version>_amd64.AppImage && ./Awake_<version>_amd64.AppImage` |

The tray app updates itself from GitHub Releases: **Check for updates** in the menu.

### CLI

| Channel | How |
|---|---|
| macOS / Linux (shell) | `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nolliebigspin/awake/releases/latest/download/awake-cli-installer.sh \| sh` |
| Windows (PowerShell) | `powershell -ExecutionPolicy Bypass -c "irm https://github.com/nolliebigspin/awake/releases/latest/download/awake-cli-installer.ps1 \| iex"` |
| Homebrew | `brew install nolliebigspin/tap/awake` |
| From source | `cargo install --git https://github.com/nolliebigspin/awake awake-cli` |

## Usage

### CLI

```sh
awake                       # run until Ctrl+C; releases everything on exit
awake --for 3h              # stop by itself after 3 hours (also 30m, 1h30m, …)
awake --interval 30 --threshold 20 --display --verbose
awake status                # platform, idle time, methods, permissions
awake status --test         # really try each method and verify it (hands off for a few seconds)
awake install [flags]       # start at login (LaunchAgent / systemd --user / Task Scheduler)
awake uninstall
```

| Flag | Default | Meaning |
|---|---|---|
| `--interval <s>` | 60 | How often the idle time is checked |
| `--threshold <s>` | 20 | Reset the idle timer once idle is longer than this (minimum 3) |
| `--display` | off | Also keep the display awake |
| `--for <duration>` | none | Stop automatically after this long, e.g. `30m`, `1h`, `1h30m`. Not available for `awake install` |
| `--verbose` | off | Log every tick and every failed method |
| `--log-file <path>` | stdout | Append the log to a file |

With the defaults, the idle time never gets much past **interval + threshold** (about 80 s), well under any chat app's Away timeout. Lines ending in `verified` confirm that resets are working.

`awake install` sets up the login service for the current user only, with no root or admin:

- **macOS:** `~/Library/LaunchAgents/dev.awake.cli.plist`, logging to `~/Library/Logs/awake.log`.
- **Linux:** `~/.config/systemd/user/awake.service`, bound to `graphical-session.target`. Read the logs with `journalctl --user -u awake`.
- **Windows:** a Task Scheduler task `awake` that runs at *your* logon. It is imported from XML (a plain `schtasks /SC ONLOGON` task needs admin), has no 72-hour time limit, runs on battery, and starts without a console window. It logs to `%LOCALAPPDATA%\awake\awake.log`.

### Tray menu

- **Keep awake**: on/off. The icon shows an open eye when on and a closed eye when off. With a timer running it reads *Keep awake · 2h 59m left*.
- **Stop after**: Never, 30 minutes, 1, 3, 6, 12 or 24 hours. Picking a time turns Keep awake on, and when it runs out Awake switches off and shows a notification. Toggling Keep awake by hand cancels the timer. The timer is not remembered across restarts.
- **Idle: Xs**: live system idle time, refreshed every 2 s.
- **Status line**: which method is keeping you Available, or what's wrong.
- **Check interval**: 30, 60 or 120 s. The threshold is fixed at 20 s.
- **Keep display on**
- **Launch at login**
- **Fix Accessibility access…**: macOS only; shown only while access is missing. It clears Awake's Accessibility entry, closes System Settings if it's open, then registers this build again and opens the right pane.
- **Show log**, **Check for updates**, **Quit Awake**

Settings persist across launches, and Awake comes back in the same on/off state.

When Awake can no longer keep you Available (for example, Accessibility is missing), it shows a notification once, not on every check. It shows another when resets work again.

The display toggle has a side effect. Resetting the idle timer counts as user activity, so while resets are happening the display (and screen lock) also stays awake on most systems. **Keep display on** additionally holds a display assertion. That covers the case where no reset is needed or possible.

## macOS: Accessibility permission

Posting synthetic mouse or keyboard events (CGEvent) requires **Accessibility** access. As shown above, the one method that needs no permission doesn't reset the idle timer apps read on current macOS. **Without Accessibility, awake can keep the Mac awake but cannot keep Teams Available.** `awake status` and the tray menu tell you when access is missing.

Grant it under **System Settings → Privacy & Security → Accessibility**, and enable the right entry:

| You run | Enable |
|---|---|
| Tray app | **Awake** (the menu item *Fix Accessibility access…* adds it to the list and opens the pane) |
| CLI in a terminal | Your terminal app (Terminal, iTerm2, Ghostty, VS Code, …), because macOS attributes the request to it |
| CLI via `awake install` | The `awake` binary itself: click **+**, press ⌘⇧G and paste the path printed by `awake install` (for example `/opt/homebrew/bin/awake`) |

The grant is tied to the code signature. Signed tray updates keep it. Rebuilding the binary locally, or using an unsigned build, can silently revoke it. `awake status` then shows `NOT granted` again. Toggle the entry off and on, or remove and re-add it.

### "Awake is enabled in System Settings, but it says *Needs Accessibility access*"

The switch in System Settings can show **on** while macOS denies the running app. This is what broke 0.2.0 for a whole day of testing. It happens in two ways:

- **The entry belongs to another build.** An unsigned local build with the same bundle ID (`dev.awake.tray`) was granted earlier. The entry keeps that build's code signature, so the signed app doesn't match it, but the row still reads *Awake* and stays on. `mise run tray-bundle` now builds as *Awake Local* (`dev.awake.tray.local`) so this can't recur.
- **System Settings shows a stale list.** If the window was open while the entry was reset or re-added, switching the old row saves nothing.

Fix: choose **Fix Accessibility access…** in the tray menu and switch Awake on in the pane that opens. By hand: quit System Settings, run `tccutil reset Accessibility dev.awake.tray` and `tccutil reset PostEvent dev.awake.tray`, then use the menu item and enable Awake. Within a few seconds the log (**Show log**) reads `Accessibility trusted: Some(true)`, and the next reset says `via CGEvent mouse nudge`.

F15 is the last resort. On some Apple keyboard layouts F14 and F15 control display brightness, so you may see a brightness change if awake ever falls back to it.

The tray app runs without the App Sandbox (see `apps/awake-tray/src-tauri/Entitlements.plist`), because a sandboxed app cannot post events into the HID stream. It runs as an agent app (`LSUIElement`, `ActivationPolicy::Accessory`), so it has no Dock icon.

## Linux: tray and Wayland limitations

**Tray icon.** The tray uses AppIndicator/StatusNotifierItem through `libayatana-appindicator3`. KDE, Cinnamon, XFCE, MATE and Budgie show it out of the box. **Stock GNOME does not**: install the *AppIndicator and KStatusNotifierItem Support* extension (preinstalled on Ubuntu). AppIndicator menus open on click, which fits a menu-only app.

**Wayland** deliberately gives applications no way to read the global idle time or inject input. What works depends on the desktop:

| Session | Idle time | Activity | Result |
|---|---|---|---|
| X11 (any desktop) | XScreenSaver ✔ | XTest ✔ (+ `SimulateUserActivity` where available) | Fully works, verified |
| KDE Plasma Wayland | ✘ (`GetSessionIdleTime` is refused on Wayland) | `SimulateUserActivity` ✔ | Works, but **unverified**: awake declares activity every interval and tells you it cannot verify |
| GNOME Wayland | Mutter `IdleMonitor` ✔ | **None**: Mutter's `ResetIdletime` only works when `MUTTER_DEBUG_RESET_IDLETIME` is set | Works only with uinput (below); otherwise awake reports a clear error |
| Other Wayland compositors | usually ✘ | usually ✘ | uinput, or an X11 session |

On Wayland, X11 idle time and XTest are ignored on purpose. XWayland only sees input sent to X clients, so a "successful" XTest reset would be a lie.

**uinput fallback.** If you give your user write access to `/dev/uinput`, awake creates a virtual pointer that the compositor treats like a real mouse. It sends 1 px relative moves and back, which works on any Wayland desktop.

```sh
sudo cp packaging/linux/60-awake-uinput.rules /etc/udev/rules.d/
echo uinput | sudo tee /etc/modules-load.d/uinput.conf
sudo modprobe uinput && sudo udevadm control --reload && sudo udevadm trigger
# log out and back in, then:
awake status --test
```

This also lets any other program running as you inject input. Only enable it if that is acceptable on your machine.

**Sleep inhibition** needs systemd-logind. Without it, awake reports that it cannot inhibit sleep and keeps resetting the idle timer anyway.

## Windows notes

- Windows blocks injected input while the session is **locked**, on the secure desktop (UAC prompts), and towards elevated windows. Chat apps show Away when you lock the screen anyway.
- Release builds are not Authenticode-signed unless you add signing, so SmartScreen may warn on first run.

## Corporate machines: EDR, antivirus and policy

awake synthesizes input: F15 key presses and 1 px mouse moves, via `SendInput`, `CGEventPost`, XTest or uinput. Endpoint detection tools (Defender for Endpoint, CrowdStrike, SentinelOne, …) and some antivirus products flag input injection as suspicious or classify tools like this as "potentially unwanted". Expect that it may be:

- blocked or quarantined,
- reported to your IT or security team, or
- against your employer's acceptable-use or presence policy.

Check with IT before running it on a managed device. Sleep inhibition on its own (no input synthesis) is rarely an issue. On macOS, MDM profiles can also prevent Accessibility from being granted, in which case awake can't keep Teams Available.

## Development

The toolchain is managed with [mise](https://mise.jdx.dev): Rust 1.98.1 with cross-check targets, bun (runs the Tauri CLI for the tray app, pinned in `apps/awake-tray/bun.lock`) and cargo-dist.

```sh
mise install
mise tasks            # list tasks
mise run test         # unit tests (the Keeper logic is tested against a fake platform)
mise run test-live    # macOS: real idle-timer resets (needs Accessibility for your terminal, ~30 s hands-off)
mise run lint         # rustfmt + clippy
mise run check-all    # type-check core + CLI for macOS, Windows and Linux from any host
mise run status       # awake status from source
mise run cli -- --interval 5 --threshold 3 -v
mise run tray         # tray app in dev mode (runs `bun install` first)
mise run tray-bundle  # unsigned local "Awake Local" .app/.dmg/.msi/... (own bundle ID, no updater artifacts)
mise run icons        # regenerate icons (scripts/make_icons.py, no dependencies)
```

Layout:

```
crates/awake-core   Platform trait, per-OS backends, Keeper loop + events (no UI)
crates/awake-cli    `awake` binary (clap), login-service installers
apps/awake-tray     Tauri v2 tray-only app (src-tauri/), empty frontend (dist/)
packaging/          Homebrew cask template, Linux udev rule
.github/workflows   release.yml (cargo-dist, CLI), tray-release.yml (tauri-action)
site/               Static website (Vercel)
```

### Website

`site/` is plain HTML and CSS with no build step: a landing page and a privacy policy. The imprint links to [awinter.dev/imprint](https://awinter.dev/imprint). To deploy, import the repository in Vercel, set **Root Directory** to `site` and **Framework Preset** to *Other*. `site/vercel.json` sets clean URLs, the `/imprint` redirect and the security headers. Preview it locally with `python3 -m http.server -d site`.

## Releasing

Bump `version` in the root `Cargo.toml` (`[workspace.package]`), commit, then tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The tag triggers both workflows:

1. **`tray-release.yml`**: checks that the tag matches the workspace version, then creates a draft GitHub Release. It builds the tray app on macOS (universal arm64 + x86_64, signed and notarized), Windows (.msi + NSIS .exe) and Linux (.deb + .AppImage), and uploads the installers, updater bundles, signatures and `latest.json`. Finally it publishes the Homebrew cask `awake-tray` to your tap and un-drafts the release.
2. **`release.yml`** (generated by `dist generate`, don't edit by hand): builds the CLI for five targets and uploads the archives and shell/PowerShell installers into the same release. It then publishes the Homebrew formula `awake`.

Edit `dist-workspace.toml`, then run `dist generate` to refresh `release.yml`.

### One-time setup

1. Create the tap repository `nolliebigspin/homebrew-tap`. It can be empty.
2. The updater key pair lives in `~/.tauri/` (`awake-updater.key`, `.key.pub`, `.password`) and never goes in the repo. The public key is already in `plugins.updater.pubkey` in `tauri.conf.json`. **Back up the private key and password**: without them, installed apps can't verify any future update. To create a new pair:
   ```sh
   cd apps/awake-tray && bun run tauri signer generate -w ~/.tauri/awake-updater.key
   ```
3. Add the secrets below under *Settings → Secrets and variables → Actions*.

### Release secrets

| Name | Kind | Used by | What it is |
|---|---|---|---|
| `TAURI_UPDATER_PUBKEY` | variable | tray | Contents of `awake-updater.key.pub` |
| `TAURI_SIGNING_PRIVATE_KEY` | secret | tray | Contents of `awake-updater.key` (signs updater bundles) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | tray | Password chosen during `tauri signer generate` (empty if none) |
| `APPLE_CERTIFICATE` | secret | tray | Base64 of your **Developer ID Application** certificate + private key exported as `.p12`: `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | secret | tray | Password of that `.p12` |
| `APPLE_SIGNING_IDENTITY` | secret | tray | e.g. `Developer ID Application: Jane Doe (TEAMID1234)` (`security find-identity -v -p codesigning`) |
| `APPLE_ID` | secret | tray | Apple ID email used for notarization |
| `APPLE_PASSWORD` | secret | tray | An [app-specific password](https://account.apple.com) for that Apple ID (not your login password) |
| `APPLE_TEAM_ID` | secret | tray | 10-character Team ID from developer.apple.com → Membership |
| `HOMEBREW_TAP_TOKEN` | secret | both | Fine-grained GitHub token with *Contents: read & write* on `nolliebigspin/homebrew-tap` |

`GITHUB_TOKEN` is provided automatically. Without the Apple secrets, the macOS build is unsigned and un-notarized. Gatekeeper will then block it, and Accessibility grants won't survive updates.

## Name availability

Checked on 2026-09-29:

| Name | crates.io | npm | Homebrew formula | Homebrew cask |
|---|---|---|---|---|
| `awake` | taken | taken | free | free |
| `awake-cli` | free | taken | free | free |
| **`staygreen`** | free | free | free | free |
| **`wideawake`** | free | free | free | free |
| **`lidless`** | free | free | free | free |

The project keeps `awake` as the binary and product name. It isn't published to npm or crates.io, and the Homebrew formula and cask live in your own tap, so nothing conflicts today. For an unscoped npm name, a crates.io release or a future homebrew-core submission, the proposed free names are:

1. **staygreen**: after the green "Available" dot in Teams and Slack.
2. **wideawake**: a plain description, easy to search for.
3. **lidless**: short and memorable, a lidless eye never closes (it matches the icon).

## Manual test checklist

Run these on each OS before a release. "Untouched" means no keyboard, mouse or trackpad input at all: not even a stray touch.

### macOS

- [ ] `awake status`: Accessibility shows `ok` for the terminal (or grant it first).
- [ ] `awake status --test`: at least one CGEvent method reports **WORKS**. Expect `IOPMAssertionDeclareUserActivity` to fail verification.
- [ ] `mise run test-live`: both live tests pass. The keeper test prints the idle time per tick, which never goes above 5 s.
- [ ] `awake -v`, machine untouched for **10 minutes**: Teams (and Slack) stays **Available**, and every `tick: idle` line stays below about **interval + threshold** (80 s by default).
- [ ] While it runs, `pmset -g assertions` lists `PreventUserIdleSystemSleep` owned by `awake`. After Ctrl+C it's gone.
- [ ] Tray: no Dock icon; the icon switches between open and closed eye with **Keep awake**; the **Idle** line counts up and drops after a reset.
- [ ] Tray without Accessibility (`tccutil reset Accessibility dev.awake.tray`, then relaunch): within one interval a notification says Awake can't keep you Available, and only one appears. The status shows *Needs Accessibility access*.
- [ ] **Fix Accessibility access…** closes System Settings, reopens it on the Accessibility pane, and Awake is listed. After you enable it: within a few seconds the item disappears, the status reads *Accessibility granted; checking…*, and the next reset brings a "working again" notification.
- [ ] The installed release build, untouched for **10 minutes** with Teams open: Teams stays **Available**, and the log shows `reset idle via CGEvent mouse nudge` on every check.
- [ ] Tray: quit and relaunch; the previous on/off state and interval are restored.
- [ ] **Launch at login** survives a logout and login.
- [ ] `awake install`: grant Accessibility to the printed binary path, log out and in, then `tail -f ~/Library/Logs/awake.log` shows `verified` resets. `awake uninstall` removes the agent.
- [ ] Lid open, on battery, 10 minutes untouched: the VPN stays connected.

### Windows

- [ ] `awake status --test`: `SendInput F15 key` reports **WORKS**.
- [ ] `awake -v`, untouched **10 minutes**: Teams stays **Available**; idle stays below about interval + threshold.
- [ ] `powercfg /requests` lists `awake.exe` under SYSTEM (and DISPLAY with `--display`). After Ctrl+C it's gone.
- [ ] Tray: the icon toggles, the idle line updates, and settings persist across restarts.
- [ ] Lock the screen (Win+L): the log shows a clear SendInput or verification error (expected). Unlock: resets resume.
- [ ] `awake install` as a standard (non-admin) user succeeds, no console window stays open, and after re-logon `%LOCALAPPDATA%\awake\awake.log` shows `verified` resets. `awake uninstall` removes the task and stops the process.
- [ ] Laptop on battery, untouched for longer than the sleep timeout: the machine doesn't sleep.

### Linux

Repeat for each session type you support: X11, GNOME Wayland, KDE Wayland.

- [ ] `awake status` shows the expected session type, idle source and methods.
- [ ] `awake status --test`:
  - X11: XTest (or `SimulateUserActivity`) reports **WORKS**.
  - GNOME Wayland without uinput: every method fails, and the Wayland explanation is printed.
  - GNOME Wayland with the udev rule: `uinput virtual pointer` reports **WORKS**.
- [ ] `awake -v`, untouched **10 minutes**: Teams / Slack stays **Available**; idle stays below about interval + threshold. On KDE Wayland, resets are logged as `UNVERIFIED`, so check presence manually.
- [ ] `systemd-inhibit --list` shows `awake` blocking `sleep:idle`. After Ctrl+C it's gone.
- [ ] Tray icon appears (GNOME: with the AppIndicator extension), the menu works, and the idle line updates.
- [ ] `awake install`, log out and in: `systemctl --user status awake` is active and `journalctl --user -u awake` shows resets. `awake uninstall` removes it.
- [ ] Both the `.deb` and the `.AppImage` start on a clean Ubuntu 22.04 VM.

## License

[MIT](LICENSE)
