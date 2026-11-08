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

/// Host folders shared into the Sandbox: `%USERPROFILE%\.xtraos-sandbox`. Windows Sandbox
/// fails to share folders under AppData (0x80070003, found live: ADR 0040), so neither the
/// data folder nor the install folder is shared. With XTRAOS_HOME (tests) it stays under it.
pub fn shared_base() -> PathBuf {
    match (
        std::env::var_os("XTRAOS_HOME"),
        std::env::var_os("USERPROFILE"),
    ) {
        (None, Some(home)) => PathBuf::from(home).join(".xtraos-sandbox"),
        _ => base(),
    }
}

/// Exchange folder (override for tests: XTRAOS_SANDBOX_EXCHANGE).
pub fn exchange_dir() -> PathBuf {
    std::env::var_os("XTRAOS_SANDBOX_EXCHANGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| shared_base().join("exchange"))
}

/// Session folder with the per-start token (read-only inside the Sandbox).
pub fn session_dir() -> PathBuf {
    std::env::var_os("XTRAOS_SANDBOX_SESSION")
        .map(PathBuf::from)
        .unwrap_or_else(|| shared_base().join("session"))
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
    if sandbox_window_open() {
        // Windows allows only one Sandbox. Never reset the folders an open one has mapped.
        if launched_recently() {
            // Ours, still booting (its first start can install updates for minutes): wait.
            return wait_ready(
                &ex,
                "Windows Sandbox is open, and XtraOS inside it now answers",
            );
        }
        return Err(err(
            "NEEDS_HUMAN",
            "a Windows Sandbox that XtraOS did not start is open (Windows allows only one)",
            "Ask the user to close that Sandbox window (everything inside it is discarded), then sandbox start again.",
        ));
    }
    let network = p.get("network").and_then(Value::as_bool).unwrap_or(false);
    let session = session_dir();
    let io = |e: std::io::Error| err("INTERNAL", format!("sandbox folders: {e}"), "");
    // A copy of this program, shared read-only (the install folder is under AppData).
    let program = shared_base().join("program");
    std::fs::create_dir_all(&program).map_err(io)?;
    // This program, or (tests, whose core runs inside the test binary) XTRAOS_SANDBOX_PROGRAM.
    let exe = match std::env::var_os("XTRAOS_SANDBOX_PROGRAM") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe().map_err(io)?,
    };
    std::fs::copy(&exe, program.join("xtraos-core.exe")).map_err(io)?;
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
    let _ = std::fs::remove_file(session.join("next")); // request numbers start at 1 again
    let wsb = base().join("xtraos.wsb");
    std::fs::write(&wsb, wsb_xml(&program, &ex, &session, network)).map_err(io)?;
    // No inherited handles: WindowsSandbox.exe outlives this call and would keep a caller's
    // output pipe open (a shell reading it would wait until the Sandbox closes).
    crate::cli::keep_std_handles_private();
    let child = std::process::Command::new(sandbox_exe())
        .arg(&wsb)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            err(
                "INTERNAL",
                format!("could not start Windows Sandbox: {e}"),
                "",
            )
        })?;
    *LAUNCHER.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    *LAUNCHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    wait_ready(
        &ex,
        &format!(
            "ok sandbox started (network {}); its XtraOS answers. Use sandbox call {{tool, args}}; the user's desktop is not touched",
            if network { "on" } else { "off" }
        ),
    )
}

/// When this core last launched a Sandbox; an open Sandbox is taken as ours only shortly after.
static LAUNCHED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
/// The WindowsSandbox.exe this core started. It stays running, windowless, after the Sandbox
/// inside has shut down, so `stop` ends it.
static LAUNCHER: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

/// After the agent inside shut the Sandbox down: give its window time to close, then end the
/// launcher this core started (only that process, never a Sandbox the user opened).
fn end_launcher() {
    let Some(mut child) = LAUNCHER.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while sandbox_window_open() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    if let Ok(None) = child.try_wait() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
const OURS_FOR: Duration = Duration::from_secs(30 * 60);

fn launched_recently() -> bool {
    LAUNCHED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|t| t.elapsed() < OURS_FOR)
}

/// True when a Windows Sandbox window is open on this desktop.
fn sandbox_window_open() -> bool {
    crate::winmgr::listable_windows().into_iter().any(|h| {
        crate::winmgr::process_name(crate::winmgr::pid_of(h)).starts_with("windowssandbox")
    })
}

/// Waits up to 120 s for the inner agent's heartbeat.
fn wait_ready(ex: &Path, text: &str) -> Result<Value, ActError> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        crate::safety::check()?;
        if let Some(h) = heartbeat(ex) {
            return Ok(json!({"text": text, "running": true, "agent": h}));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let inner = std::fs::read_to_string(ex.join("agent.txt"))
        .map(|t| format!("XtraOS inside says: {}", t.trim()))
        .unwrap_or_else(|_| "XtraOS inside has not started yet".into());
    Err(err(
        "TIMEOUT",
        format!("Windows Sandbox is open but XtraOS inside has not answered within 120 s ({inner})"),
        "Its first start installs updates and can take several minutes: run sandbox start again to keep waiting (it does not open a second one). If the window shows an error, close it and start again.",
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
    let n = next_request_number(&session_dir())
        .map_err(|e| err("INTERNAL", format!("sandbox request: {e}"), ""))?;
    let req_file = ex.join("req").join(format!("{n}.json"));
    write_atomic(&req_file, &serde_json::to_vec(&req).unwrap_or_default())
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
            let _ = std::fs::remove_file(&req_file);
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
            // As long as any call: the shared folder sometimes shows a request 10-20 s late.
            call("__shutdown", &json!({}), Duration::from_secs(60))?;
            end_launcher();
            Ok(json!({"text": "ok sandbox stopped: everything inside it is discarded"}))
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
    // Nobody sees stderr inside the Sandbox: the host reads this note when start times out.
    let note = |m: &str| {
        eprintln!("[xtraos-sandbox] {m}");
        let _ = std::fs::write(ex.join("agent.txt"), m);
    };
    let user = std::env::var("USERNAME").unwrap_or_default();
    let inside = user.eq_ignore_ascii_case(SANDBOX_USER);
    if !inside && !test {
        note(&format!(
            "refusing: not a Windows Sandbox session (user '{user}')"
        ));
        return 3;
    }
    let Ok(token) = std::fs::read_to_string(session.join("session.token")) else {
        note("no session token in the session folder");
        return 4;
    };
    note(&format!(
        "started (pid {}, user '{user}')",
        std::process::id()
    ));
    let token = token.trim().to_string();
    if inside {
        crate::safety::ensure_started(); // its own tray, kill switch and dialogs
    }
    crate::input::init_dpi();
    let (req_dir, resp_dir) = (ex.join("req"), ex.join("resp"));
    let _ = std::fs::create_dir_all(&req_dir);
    let _ = std::fs::create_dir_all(&resp_dir);
    let mut last_beat = Instant::now() - Duration::from_secs(10);
    // Requests are numbered 1, 2, 3… (host counter, reset at start). Inside the Sandbox a
    // listing of the shared folder shows a new file only ~10-20 s late (found live, ADR 0040),
    // so the next number is opened by name; the listing is only a 2 s fallback to resync.
    let mut next: u64 = 1;
    let mut last_scan = Instant::now();
    loop {
        if last_beat.elapsed() >= Duration::from_secs(2) {
            let hb = json!({"pid": std::process::id(), "version": crate::protocol::VERSION, "test": test});
            let _ = write_atomic(&ex.join("ready.json"), hb.to_string().as_bytes());
            last_beat = Instant::now();
        }
        let direct = req_dir.join(format!("{next}.json"));
        let mut files: Vec<PathBuf> = Vec::new();
        if direct.exists() {
            files.push(direct);
        } else if last_scan.elapsed() >= Duration::from_secs(2) {
            last_scan = Instant::now();
            files = std::fs::read_dir(&req_dir)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "json"))
                        .collect()
                })
                .unwrap_or_default();
            files.sort_by_key(|p| (request_number(p).unwrap_or(u64::MAX), p.clone()));
        }
        let busy = !files.is_empty();
        for f in files {
            if let Some(k) = request_number(&f) {
                next = next.max(k + 1);
            }
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
        if !busy {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}

/// `req/17.json` → 17.
fn request_number(p: &Path) -> Option<u64> {
    p.file_stem()?.to_str()?.parse().ok()
}

/// The next request number of this Sandbox session (host side; counter in the session folder).
fn next_request_number(session: &Path) -> std::io::Result<u64> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = session.join("next");
    let n: u64 = std::fs::read_to_string(&f)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(1);
    std::fs::write(&f, (n + 1).to_string())?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_numbered_from_one_per_session() {
        let d = std::env::temp_dir().join(format!("xtraos-sbn-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let _ = std::fs::remove_file(d.join("next"));
        let got: Vec<u64> = (0..3).map(|_| next_request_number(&d).unwrap()).collect();
        assert_eq!(got, vec![1, 2, 3]);
        assert_eq!(request_number(Path::new(r"C:\x\req\17.json")), Some(17));
        assert_eq!(request_number(Path::new(r"C:\x\req\1-2.json")), None);
        std::fs::remove_dir_all(&d).ok();
    }

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
