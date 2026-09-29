use crate::{Diagnostics, Error, Method, Platform, Result};

pub struct Unsupported;

impl Platform for Unsupported {
    fn inhibit_sleep(&mut self, _keep_display_on: bool) -> Result<()> {
        Err(Error::Unavailable(format!(
            "{} is not supported",
            std::env::consts::OS
        )))
    }

    fn release(&mut self) {}

    fn idle_seconds(&self) -> Option<u64> {
        None
    }

    fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            os: std::env::consts::OS.into(),
            sleep_inhibit: "unsupported".into(),
            ..Default::default()
        }
    }

    fn activity_methods(&self) -> Vec<Method> {
        Vec::new()
    }

    fn declare_activity_with(&self, method: Method) -> Result<()> {
        Err(Error::Unavailable(format!(
            "{method} is not supported here"
        )))
    }
}
