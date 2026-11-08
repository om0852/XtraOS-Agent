//! Command-line bridge (ADR 0039): `xtraos-core call <tool> [params-json | -] [--json] [--out file]`.
//! Lets any LLM or agent that can run a shell command use XtraOS without MCP. It pairs with
//! the running core exactly like the MCP server (user-only token in the data folder; starts
//! the core if it is not running), so every call passes the same policy, approval dialogs,
//! kill switch and action log. Params `-` = read the JSON from stdin (no shell quoting issues).
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const USAGE: &str = "usage: xtraos-core call <tool> [params-json | -] [--json] [--out <file.png>]
  e.g. xtraos-core call list_windows
       xtraos-core call volume \"{\\\"op\\\":\\\"change\\\",\\\"by\\\":-10}\"
       echo {\"op\":\"get\"} | xtraos-core call volume -
  Tools are the same as the MCP tools (docs/04-mcp-tool-contract.md). Exit 0 = ok, 1 = error.";

/// GUI-subsystem release builds have no console: attach to the caller's console so output
/// shows in a terminal (redirected output works either way).
pub fn attach_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
        fn GetStdHandle(n: u32) -> *mut std::ffi::c_void;
    }
    unsafe {
        let h = GetStdHandle(0xFFFF_FFF5); // STD_OUTPUT_HANDLE
        if h.is_null() {
            AttachConsole(0xFFFF_FFFF); // ATTACH_PARENT_PROCESS
        }
    }
}

fn token() -> Option<String> {
    if let Ok(t) = std::env::var("XTRAOS_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    std::fs::read_to_string(crate::pairing::data_dir().join("pairing.token"))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn pipe() -> Option<String> {
    if let Ok(p) = std::env::var("XTRAOS_PIPE") {
        return Some(p);
    }
    std::fs::read_to_string(crate::pairing::data_dir().join("connection.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["pipe"].as_str().map(String::from))
        .or_else(|| crate::pairing::pipe_name().ok())
}

fn open_pipe(name: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
}

/// Before starting a process that outlives the caller (the core, Windows Sandbox): stop this
/// process's console/pipe handles from being inherited. `Stdio::null()` is not enough, because
/// Windows hands every inheritable handle to the child, and a shell reading this process's
/// output would then wait until that long-lived child exits. (Rust's `Stdio::inherit()`
/// duplicates the handle, so later short-lived children still get their output through.)
pub(crate) fn keep_std_handles_private() {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> *mut std::ffi::c_void;
        fn SetHandleInformation(h: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    for n in [0xFFFF_FFF6u32, 0xFFFF_FFF5, 0xFFFF_FFF4] {
        // STD_INPUT, STD_OUTPUT, STD_ERROR
        let h = unsafe { GetStdHandle(n) };
        if !h.is_null() && h as isize != -1 {
            unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) };
        }
    }
}

/// Connects; starts the core (hidden, detached) once if nothing answers.
fn connect() -> Result<(std::fs::File, String), String> {
    if let (Some(p), Some(t)) = (pipe(), token()) {
        if let Ok(f) = open_pipe(&p) {
            return Ok((f, t));
        }
    }
    if std::env::var_os("XTRAOS_PIPE").is_some() {
        return Err("the XtraOS core is not reachable on XTRAOS_PIPE".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        keep_std_handles_private();
        std::process::Command::new(&exe)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start the XtraOS core: {e}"))?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        if let (Some(p), Some(t)) = (pipe(), token()) {
            if let Ok(f) = open_pipe(&p) {
                return Ok((f, t));
            }
        }
        if Instant::now() > deadline {
            return Err("started the XtraOS core but it did not answer within 10 s".into());
        }
    }
}

fn rpc(
    w: &mut std::fs::File,
    r: &mut BufReader<std::fs::File>,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let mut line = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )
    .map_err(|e| e.to_string())?;
    line.push(b'\n');
    w.write_all(&line).map_err(|e| format!("pipe write: {e}"))?;
    w.flush().map_err(|e| format!("pipe write: {e}"))?;
    let mut reply = String::new();
    r.read_line(&mut reply)
        .map_err(|e| format!("pipe read: {e}"))?;
    serde_json::from_str(&reply).map_err(|e| format!("bad reply: {e}"))
}

/// Entry point for `xtraos-core call …`. Returns the exit code.
pub fn call_main(args: &[String]) -> i32 {
    attach_console();
    let mut rest: Vec<&String> = args.iter().skip(1).collect();
    let as_json = rest.iter().any(|a| *a == "--json");
    rest.retain(|a| *a != "--json");
    let mut out: Option<PathBuf> = None;
    if let Some(i) = rest.iter().position(|a| *a == "--out") {
        out = rest.get(i + 1).map(PathBuf::from);
        rest.drain(i..(i + 2).min(rest.len()));
    }
    let Some(method) = rest.first().map(|s| s.to_string()) else {
        eprintln!("{USAGE}");
        return 2;
    };
    if method == "help" || method == "--help" {
        println!("{USAGE}");
        return 0;
    }
    let raw = match rest.get(1).map(|s| s.as_str()) {
        None => "{}".to_string(),
        Some("-") => {
            let mut s = String::new();
            if std::io::stdin().read_to_string(&mut s).is_err() {
                eprintln!("could not read params from stdin");
                return 2;
            }
            s
        }
        Some(s) => s.to_string(),
    };
    // Windows PowerShell pipes text with a UTF-8 byte-order mark; it is never part of the JSON.
    let params: Value = match serde_json::from_str(raw.trim_start_matches('\u{feff}').trim()) {
        Ok(v @ Value::Object(_)) => v,
        Ok(_) => {
            eprintln!("params must be a JSON object, e.g. {{\"op\":\"get\"}} (or - to read it from stdin)");
            return 2;
        }
        Err(e) => {
            eprintln!("params are not valid JSON: {e}");
            if raw.contains('\\') {
                // The usual cause on Windows: a path with single backslashes ("C:\Users" has the
                // invalid escape \U). Never guessed: "C:\new" would silently become a newline.
                eprintln!("hint: in JSON a backslash must be doubled (\"C:\\\\Users\") or use forward slashes (\"C:/Users\")");
            }
            return 2;
        }
    };
    let (f, token) = match connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR NOT_PAIRED: {e}");
            return 1;
        }
    };
    let mut reader = BufReader::new(match f.try_clone() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR INTERNAL: {e}");
            return 1;
        }
    });
    let mut w = f;
    match rpc(
        &mut w,
        &mut reader,
        1,
        "pair",
        json!({"token": token, "client": "xtraos-cli"}),
    ) {
        Ok(v) if v.get("result").is_some() => {}
        Ok(v) => {
            eprintln!(
                "ERROR NOT_PAIRED: {}",
                v["error"]["message"].as_str().unwrap_or("pairing refused")
            );
            return 1;
        }
        Err(e) => {
            eprintln!("ERROR NOT_PAIRED: {e}");
            return 1;
        }
    }
    let reply = match rpc(&mut w, &mut reader, 2, &method, params) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR INTERNAL: {e}");
            return 1;
        }
    };
    if let Some(e) = reply.get("error") {
        eprintln!(
            "ERROR {}: {}{}",
            e["code"].as_str().unwrap_or("INTERNAL"),
            e["message"].as_str().unwrap_or(""),
            e["hint"]
                .as_str()
                .filter(|h| !h.is_empty())
                .map(|h| format!("\nhint: {h}"))
                .unwrap_or_default()
        );
        return 1;
    }
    let mut result = reply.get("result").cloned().unwrap_or(Value::Null);
    if let Some(b64) = result
        .get("png_base64")
        .and_then(Value::as_str)
        .map(String::from)
    {
        let path = out.unwrap_or_else(|| {
            std::env::temp_dir().join(format!("xtraos-shot-{}.png", crate::protocol::now_ms()))
        });
        match decode_b64(&b64).map(|bytes| std::fs::write(&path, bytes)) {
            Some(Ok(())) => result["png_base64"] = json!(format!("saved to {}", path.display())),
            _ => {
                eprintln!("ERROR INTERNAL: could not save the image");
                return 1;
            }
        }
    }
    match (as_json, result.get("text").and_then(Value::as_str)) {
        (false, Some(t)) => println!("{t}"),
        _ => println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        ),
    }
    0
}

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in ["", "f", "fo", "foo", "foobar", "png\x00\x01\u{ff}"] {
            let enc = crate::capture::base64(s.as_bytes());
            assert_eq!(decode_b64(&enc).unwrap(), s.as_bytes(), "{s:?}");
        }
        assert!(decode_b64("@@@@").is_none());
    }
}
