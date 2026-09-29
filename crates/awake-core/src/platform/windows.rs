//! Windows: SetThreadExecutionState for sleep, GetLastInputInfo for idle,
//! SendInput for activity.

use std::mem::size_of;

use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, LASTINPUTINFO, MOUSEEVENTF_MOVE, MOUSEINPUT, VK_F15,
};

use crate::{Diagnostics, Error, Method, MethodStatus, Platform, Result};

pub struct WinPlatform {
    inhibited: bool,
}

impl WinPlatform {
    pub fn new() -> Self {
        WinPlatform { inhibited: false }
    }
}

fn key(vk: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_move(dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(Error::Os(format!(
            "SendInput injected {sent}/{} events: {} (a locked workstation, UAC prompt or \
             elevated foreground window blocks input injection)",
            inputs.len(),
            std::io::Error::last_os_error()
        )))
    }
}

impl Platform for WinPlatform {
    fn inhibit_sleep(&mut self, keep_display_on: bool) -> Result<()> {
        let mut flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
        if keep_display_on {
            flags |= ES_DISPLAY_REQUIRED;
        }
        // Bound to this thread: the Keeper creates and uses us on one thread.
        if unsafe { SetThreadExecutionState(flags) } == 0 {
            return Err(Error::Os("SetThreadExecutionState failed".into()));
        }
        self.inhibited = true;
        Ok(())
    }

    fn release(&mut self) {
        if std::mem::take(&mut self.inhibited) {
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }

    fn idle_seconds(&self) -> Option<u64> {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // Both are 32-bit millisecond tick counts; wrapping_sub survives the
        // 49.7-day rollover.
        let now = unsafe { GetTickCount() };
        Some(u64::from(now.wrapping_sub(info.dwTime)) / 1000)
    }

    fn activity_methods(&self) -> Vec<Method> {
        vec![Method::WinF15Key, Method::WinMouseNudge]
    }

    fn declare_activity_with(&self, method: Method) -> Result<()> {
        match method {
            Method::WinF15Key => send(&[key(VK_F15, 0), key(VK_F15, KEYEVENTF_KEYUP)]),
            Method::WinMouseNudge => send(&[mouse_move(1, 0), mouse_move(-1, 0)]),
            m => Err(Error::Unavailable(format!("{m} is not a Windows method"))),
        }
    }

    fn failure_hint(&self) -> Option<String> {
        Some(
            "Windows blocks injected input while the workstation is locked, on the secure \
             desktop (UAC), or towards elevated windows when awake is not elevated. Chat \
             apps show Away while locked regardless."
                .into(),
        )
    }

    fn diagnostics(&self) -> Diagnostics {
        let ok = |detail: &str| detail.to_owned();
        Diagnostics {
            os: format!("Windows ({})", std::env::consts::ARCH),
            session: std::env::var("SESSIONNAME").ok(),
            idle_source: self.idle_seconds().map(|_| "GetLastInputInfo".into()),
            idle_seconds: self.idle_seconds(),
            sleep_inhibit: "SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED [| ES_DISPLAY_REQUIRED])".into(),
            methods: vec![
                MethodStatus {
                    method: Method::WinF15Key,
                    available: true,
                    detail: ok("no permission needed"),
                },
                MethodStatus {
                    method: Method::WinMouseNudge,
                    available: true,
                    detail: ok("fallback"),
                },
            ],
            permissions: Vec::new(),
            notes: vec![
                "Injected input is blocked while the session is locked; presence will show Away then."
                    .into(),
            ],
        }
    }
}

impl Drop for WinPlatform {
    fn drop(&mut self) {
        self.release();
    }
}
