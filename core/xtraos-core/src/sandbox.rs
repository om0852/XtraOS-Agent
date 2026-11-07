//! P3-M4 (ADR 0040): a sandboxed desktop for the agent. Windows Sandbox gives the agent its
//! own desktop, cursor and keyboard, so it works in parallel with the user. A second XtraOS
//! runs inside (`--sandbox-agent`); the host core talks to it only through one shared
//! exchange folder (one request file and one response file per call). There is no network
//! listener and no pipe across the VM boundary.
//!
//! Host:   sandbox {op: status | start (network?) | call (tool, args, timeout_ms?) | stop}
//! Inside: xtraos-core.exe --sandbox-agent <exchange> [--session <dir>] [--test-on-host]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

use crate::input::ActError;

const SANDBOX_USER: &str = "WDAGUtilityAccount";
const MAX_RESPONSE: u64 = 32 * 1024 * 1024;
const MAX_REQUEST: u64 = 4 * 1024 * 1024;
const HEARTBEAT_FRESH: Duration = Duration::from_secs(6);
/// Tools the inner agent allows in --test-on-host mode (it then runs on the real desktop).
const TEST_TOOLS: &[&str] = &[
    "list_windows",
    "observe",
    "find",
    "read_text",
    "screenshot",
    "ocr",
    "frame_diff",
];
static SEQ: AtomicU64 = AtomicU64::new(0);

fn err(code: &'static str, m: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: m.into(),
        hint,
    }
}

fn bad(m: impl Into<String>) -> ActError {
    err(
        "INVALID_ARGS",
        m,
        "sandbox {op: status | start (network?) | call (tool, args, timeout_ms?) | stop}",
    )
}

fn base() -> PathBuf {
    crate::pairing::data_dir().join("sandbox")
}

/// Exchange folder (override for tests: XTRAOS_SANDBOX_EXCHANGE).
pub fn exchange_dir() -> PathBuf {
    std::env::var_os("XTRAOS_SANDBOX_EXCHANGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| base().join("exchange"))
}

/// Session folder with the per-start token (read-only inside the Sandbox).
pub fn session_dir() -> PathBuf {
    std::env::var_os("XTRAOS_SANDBOX_SESSION")
        .map(PathBuf::from)
        .unwrap_or_else(|| base().join("session"))
}

fn sandbox_exe() -> PathBuf {
    PathBuf::from(std::env::var("windir").unwrap_or_else(|_| r"C:\Windows".into()))
        .join("System32")
        .join("WindowsSandbox.exe")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The Windows Sandbox configuration: program read-only, exchange read-write, session
/// read-only; no clipboard/printer/microphone/camera redirection; network off unless asked.
pub fn wsb_xml(program: &Path, exchange: &Path, session: &Path, network: bool) -> String {
    let folder = |host: &Path, inside: &str, ro: bool| {
        format!(
            "    <MappedFolder>\n      <HostFolder>{}</HostFolder>\n      <SandboxFolder>{inside}</SandboxFolder>\n      <ReadOnly>{ro}</ReadOnly>\n    </MappedFolder>\n",
            xml_escape(&host.display().to_string())
        )
    };
    format!(
        "<Configuration>\n  <Networking>{}</Networking>\n  <ClipboardRedirection>Disable</ClipboardRedirection>\n  <PrinterRedirection>Disable</PrinterRedirection>\n  <AudioInput>Disable</AudioInput>\n  <VideoInput>Disable</VideoInput>\n  <ProtectedClient>Enable</ProtectedClient>\n  <MappedFolders>\n{}{}{}  </MappedFolders>\n  <LogonCommand>\n    <Command>C:\\XtraOS\\xtraos-core.exe --sandbox-agent C:\\XtraOSExchange --session C:\\XtraOSSession</Command>\n  </LogonCommand>\n</Configuration>\n",
        if network { "Default" } else { "Disable" },
        folder(program, r"C:\XtraOS", true),
        folder(exchange, r"C:\XtraOSExchange", false),
        folder(session, r"C:\XtraOSSession", true),
    )
}

fn heartbeat(ex: &Path) -> Option<Value> {
    let p = ex.join("ready.json");
    let fresh = std::fs::metadata(&p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < HEARTBEAT_FRESH);
    if !fresh {
        return None;
    }
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn status() -> Value {
    let installed = sandbox_exe().exists();
    let hb = heartbeat(&exchange_dir());
    let text = match (installed, &hb) {
        (_, Some(h)) => format!(
            "sandbox running; its XtraOS answers (pid {} inside{})",
            h["pid"],
            if h["test"] == true {
                ", TEST MODE on this desktop"
            } else {
                ""
            }
        ),
        (true, None) => "Windows Sandbox installed; no sandbox running (sandbox start)".into(),
        (false, None) => {
            "Windows Sandbox is not installed (it is an optional Windows feature)".into()
        }
    };
    json!({"text": text, "installed": installed, "running": hb.is_some(), "agent": hb})
}

fn not_installed() -> ActError {
    err(
        "NEEDS_HUMAN",
        "Windows Sandbox is not installed on this PC",
        "The user turns it on once: Settings > System > Optional features > More Windows features > Windows Sandbox (or admin PowerShell: Enable-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM -All), then restarts.",
    )
}

fn start(p: &Value) -> Result<Value, ActError> {
    let ex = exchange_dir();
    if heartbeat(&ex).is_some() {
        return Ok(json!({"text": "sandbox already running", "running": true}));
    }
    if !sandbox_exe().exists() {
        return Err(not_installed());
    }
    let network = p.get("network").and_then(Value::as_bool).unwrap_or(false);
    let program = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .ok_or_else(|| err("INTERNAL", "own folder unknown", ""))?;
    let session = session_dir();
    let io = |e: std::io::Error| err("INTERNAL", format!("sandbox folders: {e}"), "");
    let _ = std::fs::remove_dir_all(&ex);
    std::fs::create_dir_all(ex.join("req")).map_err(io)?;
    std::fs::create_dir_all(ex.join("resp")).map_err(io)?;
    std::fs::create_dir_all(&session).map_err(io)?;
    let token: String = crate::winsec::random_bytes(32)
        .map_err(io)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(session.join("session.token"), &token).map_err(io)?;
    let wsb = base().join("xtraos.wsb");
    std::fs::write(&wsb, wsb_xml(&program, &ex, &session, network)).map_err(io)?;
    std::process::Command::new(sandbox_exe())
        .arg(&wsb)
        .spawn()
        .map_err(|e| {
            err(
                "INTERNAL",
                format!("could not start Windows Sandbox: {e}"),
                "",
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        crate::safety::check()?;
        if let Some(h) = heartbeat(&ex) {
            return Ok(json!({
                "text": format!("ok sandbox started (network {}); its XtraOS answers. Use sandbox call {{tool, args}}; the user's desktop is not touched",
                    if network { "on" } else { "off" }),
                "running": true, "agent": h,
            }));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(err(
        "TIMEOUT",
        "Windows Sandbox opened but XtraOS inside did not answer within 120 s",
        "Look at the Sandbox window; close it and try again.",
    ))
}

/// Runs one tool inside the Sandbox.
pub fn call(tool: &str, args: &Value, timeout: Duration) -> Result<Value, ActError> {
    let ex = exchange_dir();
    if heartbeat(&ex).is_none() {
        return Err(err(
            "NEEDS_HUMAN",
            "no sandbox is running (or its XtraOS stopped answering)",
            "sandbox start first.",
        ));
    }
    let token = std::fs::read_to_string(session_dir().join("session.token")).map_err(|_| {
        err(
            "INTERNAL",
            "sandbox session token missing",
            "sandbox start again.",
        )
    })?;
    let id = format!(
        "{}-{}",
        crate::protocol::now_ms(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let req = json!({"id": id, "token": token.trim(), "tool": tool, "args": args});
    write_atomic(
        &ex.join("req").join(format!("{id}.json")),
        &serde_json::to_vec(&req).unwrap_or_default(),
    )
    .map_err(|e| err("INTERNAL", format!("sandbox request: {e}"), ""))?;
    let resp = ex.join("resp").join(format!("{id}.json"));
    let deadline = Instant::now() + timeout;
    loop {
        crate::safety::check()?;
        if let Ok(m) = std::fs::metadata(&resp) {
            if m.len() > MAX_RESPONSE {
                let _ = std::fs::remove_file(&resp);
                return Err(err("INTERNAL", "sandbox reply too large: discarded", ""));
            }
            let text = std::fs::read_to_string(&resp)
                .map_err(|e| err("INTERNAL", format!("sandbox reply: {e}"), ""))?;
            let _ = std::fs::remove_file(&resp);
            // Untrusted data from inside the VM: parsed, never executed.
            let v: Value = serde_json::from_str(&text)
                .map_err(|_| err("INTERNAL", "sandbox reply is not valid JSON: discarded", ""))?;
            if v["id"] != id.as_str() {
                return Err(err(
                    "INTERNAL",
                    "sandbox reply for another request: discarded",
                    "",
                ));
            }
            if let Some(r) = v.get("result") {
                let mut r = r.clone();
                if let Some(o) = r.as_object_mut() {
                    o.insert("via".into(), json!("sandbox"));
                }
                return Ok(r);
            }
            let e = &v["error"];
            let code = match e["code"].as_str().unwrap_or("") {
                "INVALID_ARGS" => "INVALID_ARGS",
                "BLOCKED_BY_POLICY" => "BLOCKED_BY_POLICY",
                "ELEMENT_NOT_FOUND" => "ELEMENT_NOT_FOUND",
                "WINDOW_NOT_FOUND" => "WINDOW_NOT_FOUND",
                "STALE_REF" => "STALE_REF",
                "TIMEOUT" => "TIMEOUT",
                "NEEDS_HUMAN" => "NEEDS_HUMAN",
                "STOPPED" => "STOPPED",
                _ => "INTERNAL",
            };
            return Err(err(
                code,
                format!(
                    "in the sandbox: {}",
                    e["message"].as_str().unwrap_or("error")
                ),
                "",
            ));
        }
        if Instant::now() > deadline {
            let _ = std::fs::remove_file(ex.join("req").join(format!("{id}.json")));
            return Err(err(
                "TIMEOUT",
                format!("no answer from the sandbox within {} s", timeout.as_secs()),
                "sandbox status: is it still running?",
            ));
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Policy class name for a `sandbox` request (pure).
pub fn op_class(p: &Value) -> &'static str {
    match p.get("op").and_then(Value::as_str).unwrap_or("status") {
        "start" => "launch",
        "stop" => "close",
        _ => "observe", // status, call: nothing on the user's desktop
    }
}

/// The `sandbox` method (host side).
pub fn run(p: &Value) -> Result<Value, ActError> {
    match p.get("op").and_then(Value::as_str).unwrap_or("status") {
        "status" => Ok(status()),
        "start" => start(p),
        "call" => {
            let tool = p
                .get("tool")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("call needs 'tool'"))?;
            if matches!(
                tool,
                "sandbox" | "batch" | "elevated_helper" | "browser" | "record_session"
            ) {
                return Err(bad(format!("'{tool}' is not available inside the sandbox")));
            }
            let t = p
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(60_000)
                .clamp(1_000, 600_000);
            call(
                tool,
                &p.get("args").cloned().unwrap_or(json!({})),
                Duration::from_millis(t),
            )
        }
        "stop" => {
            call("__shutdown", &json!({}), Duration::from_secs(15))?;
            Ok(json!({"text": "ok sandbox stopping: everything inside it is discarded"}))
        }
        o => Err(bad(format!("unknown op '{o}'"))),
    }
}

// =============================================================================================
// Inside the Sandbox

/// Entry point of `--sandbox-agent`. Returns the exit code.
pub fn agent_main(args: &[String]) -> i32 {
    let mut it = args.iter().skip(1);
    let Some(ex) = it.next().map(PathBuf::from) else {
        eprintln!("usage: --sandbox-agent <exchange> [--session <dir>] [--test-on-host]");
        return 2;
    };
    let rest: Vec<&String> = it.collect();
    let test = rest.iter().any(|a| *a == "--test-on-host");
    let session = rest
        .iter()
        .position(|a| *a == "--session")
        .and_then(|i| rest.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\XtraOSSession"));
    let inside = std::env::var("USERNAME").is_ok_and(|u| u.eq_ignore_ascii_case(SANDBOX_USER));
    if !inside && !test {
        eprintln!("[xtraos-sandbox] refusing: this is not a Windows Sandbox session");
        return 3;
    }
    let Ok(token) = std::fs::read_to_string(session.join("session.token")) else {
        eprintln!("[xtraos-sandbox] no session token");
        return 4;
    };
    let token = token.trim().to_string();
    if inside {
        crate::safety::ensure_started(); // its own tray, kill switch and dialogs
    }
    crate::input::init_dpi();
    let (req_dir, resp_dir) = (ex.join("req"), ex.join("resp"));
    let _ = std::fs::create_dir_all(&req_dir);
    let _ = std::fs::create_dir_all(&resp_dir);
    let mut last_beat = Instant::now() - Duration::from_secs(10);
    loop {
        if last_beat.elapsed() >= Duration::from_secs(2) {
            let hb = json!({"pid": std::process::id(), "version": crate::protocol::VERSION, "test": test});
            let _ = write_atomic(&ex.join("ready.json"), hb.to_string().as_bytes());
            last_beat = Instant::now();
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&req_dir)
            .map(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for f in files {
            let too_big = std::fs::metadata(&f)
                .map(|m| m.len() > MAX_REQUEST)
                .unwrap_or(true);
            let text = if too_big {
                None
            } else {
                std::fs::read_to_string(&f).ok()
            };
            let _ = std::fs::remove_file(&f);
            let Some(req) = text.and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
                continue;
            };
            let id = req["id"].as_str().unwrap_or("").to_string();
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
                continue;
            }
            let tool = req["tool"].as_str().unwrap_or("");
            let reply = if req["token"] != token.as_str() {
                json!({"id": id, "error": {"code": "BLOCKED_BY_POLICY", "message": "wrong session token"}})
            } else if tool == "__shutdown" {
                let _ = write_atomic(
                    &resp_dir.join(format!("{id}.json")),
                    json!({"id": id, "result": {"text": "stopping"}})
                        .to_string()
                        .as_bytes(),
                );
                if !test {
                    let _ = std::process::Command::new("shutdown.exe")
                        .args(["/s", "/t", "0"])
                        .spawn();
                }
                let _ = std::fs::remove_file(ex.join("ready.json"));
                return 0;
            } else if test && !TEST_TOOLS.contains(&tool) {
                json!({"id": id, "error": {"code": "BLOCKED_BY_POLICY", "message": format!("'{tool}' is not allowed in test mode (it would act on the real desktop)")}})
            } else {
                let args = req.get("args").cloned().unwrap_or(json!({}));
                match crate::safety::authorize(tool, &args)
                    .and_then(|_| crate::exec::dispatch(tool, &args))
                {
                    Ok(v) => json!({"id": id, "result": v}),
                    Err(e) => json!({"id": id, "error": {"code": e.code, "message": e.message}}),
                }
            };
            let _ = write_atomic(
                &resp_dir.join(format!("{id}.json")),
                reply.to_string().as_bytes(),
            );
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsb_is_isolated_and_escaped() {
        let x = wsb_xml(
            Path::new(r"C:\Prog\Xtra&OS"),
            Path::new(r"C:\Users\me\AppData\Local\XtraOS\sandbox\exchange"),
            Path::new(r"C:\s"),
            false,
        );
        assert!(x.contains("<Networking>Disable</Networking>"));
        for k in [
            "ClipboardRedirection",
            "PrinterRedirection",
            "AudioInput",
            "VideoInput",
        ] {
            assert!(x.contains(&format!("<{k}>Disable</{k}>")), "{k}");
        }
        assert!(x.contains("C:\\Prog\\Xtra&amp;OS"), "escaped");
        assert!(x.contains(
            "<SandboxFolder>C:\\XtraOS</SandboxFolder>\n      <ReadOnly>true</ReadOnly>"
        ));
        assert!(x.contains(
            "<SandboxFolder>C:\\XtraOSExchange</SandboxFolder>\n      <ReadOnly>false</ReadOnly>"
        ));
        assert!(x.contains(
            "<SandboxFolder>C:\\XtraOSSession</SandboxFolder>\n      <ReadOnly>true</ReadOnly>"
        ));
        assert!(
            wsb_xml(Path::new("a"), Path::new("b"), Path::new("c"), true)
                .contains("<Networking>Default</Networking>")
        );
        assert_eq!(op_class(&json!({"op": "start"})), "launch");
        assert_eq!(op_class(&json!({"op": "call"})), "observe");
        assert_eq!(op_class(&json!({"op": "stop"})), "close");
    }
}
