//! Where the core keeps its files, the pairing token, and the connection info the MCP
//! server reads. See ADR 0009 for why the token is a user-only file rather than DPAPI.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::winsec;

pub const TOKEN_FILE: &str = "pairing.token";
pub const CONNECTION_FILE: &str = "connection.json";

/// `%LOCALAPPDATA%\XtraOS` (override: `XTRAOS_HOME`). %LOCALAPPDATA% is user-only by default.
pub fn data_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("XTRAOS_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("XtraOS")
}

/// `\\.\pipe\xtraos-core-<user SID>` (override: `XTRAOS_PIPE`).
pub fn pipe_name() -> io::Result<String> {
    if let Ok(p) = std::env::var("XTRAOS_PIPE") {
        return Ok(p);
    }
    Ok(format!(
        r"\\.\pipe\xtraos-core-{}",
        winsec::current_user_sid()?
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_valid_token(t: &str) -> bool {
    t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reads the token (override: `XTRAOS_TOKEN`) or creates a new 256-bit one.
pub fn load_or_create_token(dir: &Path) -> io::Result<String> {
    if let Ok(t) = std::env::var("XTRAOS_TOKEN") {
        return Ok(t);
    }
    let path = dir.join(TOKEN_FILE);
    if let Ok(existing) = fs::read_to_string(&path) {
        let t = existing.trim().to_string();
        if is_valid_token(&t) {
            return Ok(t);
        }
    }
    fs::create_dir_all(dir)?;
    let t = hex(&winsec::random_bytes(32)?);
    fs::write(&path, &t)?;
    Ok(t)
}

/// Writes `connection.json` so the MCP server can find the pipe.
pub fn write_connection_info(dir: &Path, pipe: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let info = serde_json::json!({
        "pipe": pipe,
        "pid": std::process::id(),
        "version": crate::protocol::VERSION,
    });
    fs::write(dir.join(CONNECTION_FILE), serde_json::to_vec_pretty(&info)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "xtraos-test-{tag}-{}",
            hex(&winsec::random_bytes(4).unwrap())
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn token_is_created_once_and_reused() {
        if std::env::var("XTRAOS_TOKEN").is_ok() {
            return;
        }
        let d = temp_dir("token");
        let a = load_or_create_token(&d).unwrap();
        let b = load_or_create_token(&d).unwrap();
        assert!(is_valid_token(&a));
        assert_eq!(a, b);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn corrupt_token_file_is_replaced() {
        if std::env::var("XTRAOS_TOKEN").is_ok() {
            return;
        }
        let d = temp_dir("corrupt");
        fs::write(d.join(TOKEN_FILE), "short").unwrap();
        let t = load_or_create_token(&d).unwrap();
        assert!(is_valid_token(&t));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn default_pipe_name_contains_user_sid() {
        if std::env::var("XTRAOS_PIPE").is_ok() {
            return;
        }
        let p = pipe_name().unwrap();
        assert!(p.starts_with(r"\\.\pipe\xtraos-core-S-1-5-"), "{p}");
    }
}
