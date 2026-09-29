use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Timestamped lines to stdout or an append-only file.
pub struct Log {
    file: Option<File>,
}

impl Log {
    pub fn open(path: Option<&Path>) -> Result<Self, String> {
        let file = match path {
            Some(p) => {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                Some(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .map_err(|e| format!("cannot open log file {}: {e}", p.display()))?,
                )
            }
            None => None,
        };
        Ok(Log { file })
    }

    pub fn line(&mut self, msg: &str) {
        let line = format!("{} {msg}\n", awake_core::utc_timestamp());
        match &mut self.file {
            Some(f) => {
                let _ = f.write_all(line.as_bytes());
            }
            None => {
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(line.as_bytes());
                let _ = out.flush();
            }
        }
    }
}
