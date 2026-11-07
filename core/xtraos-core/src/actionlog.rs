//! Append-only action log (JSONL). One line per handled request. Never contains request
//! params or the pairing token (CLAUDE.md §5 logging rule).
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Serialize)]
pub struct Entry<'a> {
    pub ts_ms: u128,
    pub session: Option<&'a str>,
    pub client: Option<&'a str>,
    pub method: &'a str,
    pub ok: bool,
    pub code: Option<&'a str>,
    pub ms: f64,
    /// Policy decision for OS actions (M5): allow / confirmed / blocked; or a policy reload note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<&'a str>,
}

pub struct ActionLog {
    path: PathBuf,
    file: Mutex<File>,
}

impl ActionLog {
    /// Creates `<dir>/actions-<unix-ms>-<pid>.jsonl`.
    pub fn create(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            "actions-{}-{}.jsonl",
            crate::protocol::now_ms(),
            std::process::id()
        ));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(&self, entry: &Entry) {
        // Logging must never take the core down; failures go to stderr.
        let mut line = match serde_json::to_vec(entry) {
            Ok(v) => v,
            Err(e) => return eprintln!("[xtraos-core] log serialise failed: {e}"),
        };
        line.push(b'\n');
        let mut f = match self.file.lock() {
            Ok(f) => f,
            Err(p) => p.into_inner(),
        };
        if let Err(e) = f.write_all(&line) {
            eprintln!("[xtraos-core] log write failed: {e}");
        }
    }
}
