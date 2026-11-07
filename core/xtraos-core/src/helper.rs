//! Elevated admin helper (P2-M5, ADR 0030).
//!
//! Windows blocks a normal program from driving administrator windows (UIPI). The helper is
//! this same binary, started for one session through a UAC prompt that only the human can
//! answer (`--elevated-helper`). It serves exactly one client, the core that started it:
//! - the core picks a random pipe name; the helper creates the pipe as the first and only
//!   instance (a squatted name makes it exit), allowing only the core's user SID, network
//!   denied, medium mandatory label so the medium core can open it;
//! - the helper holds a handle to the core process (no PID reuse), checks that the core's
//!   image is its own image, and accepts only a client whose PID is the core's; it never
//!   listens again;
//! - the core checks that the server PID is the helper it started, and opens the pipe with
//!   identification-level QoS.
//!
//! The helper asks the human itself (its dialogs run elevated, so normal programs cannot
//! click them) before the first action on each admin window, and only acts on windows that
//! need elevation. It exits with the core, the kill switch, its tray menu, or 15 idle minutes.
#![allow(non_snake_case, clippy::upper_case_acronyms)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::input::{ActError, HWND};
use crate::{perceive, safety, winmgr, winsec};

type BOOL = i32;
type HANDLE = *mut c_void;

const PIPE_PREFIX: &str = r"\\.\pipe\xtraos-elev-";
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PIPE_BUSY: u32 = 231;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_CANCELLED: u32 = 1223;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
const SECURITY_IDENTIFICATION: u32 = 1 << 16;
const SYNCHRONIZE: u32 = 0x0010_0000;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
const SEE_MASK_FLAG_NO_UI: u32 = 0x0000_0400;
const SW_HIDE: i32 = 0;
const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = 0xFFFF_FFFF;

/// Longest request/reply line on the helper pipe.
const MAX_LINE: u64 = 4 * 1024 * 1024;
/// A grant ("Allow this window for 10 minutes").
const GRANT: Duration = Duration::from_secs(600);
const IDLE_EXIT: Duration = Duration::from_secs(15 * 60);
const CONNECT_WAIT: Duration = Duration::from_secs(30);
const RATE_PER_MIN: usize = 120;

#[repr(C)]
struct SHELLEXECUTEINFOW {
    cbSize: u32,
    fMask: u32,
    hwnd: HWND,
    lpVerb: *const u16,
    lpFile: *const u16,
    lpParameters: *const u16,
    lpDirectory: *const u16,
    nShow: i32,
    hInstApp: *mut c_void,
    lpIDList: *mut c_void,
    lpClass: *const u16,
    hkeyClass: *mut c_void,
    dwHotKey: u32,
    hIconOrMonitor: *mut c_void,
    hProcess: HANDLE,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        sa: *mut c_void,
    ) -> HANDLE;
    fn ConnectNamedPipe(h: HANDLE, ov: *mut c_void) -> BOOL;
    fn DisconnectNamedPipe(h: HANDLE) -> BOOL;
    fn GetNamedPipeClientProcessId(h: HANDLE, pid: *mut u32) -> BOOL;
    fn GetNamedPipeServerProcessId(h: HANDLE, pid: *mut u32) -> BOOL;
    fn PeekNamedPipe(
        h: HANDLE,
        buf: *mut c_void,
        size: u32,
        read: *mut u32,
        avail: *mut u32,
        left: *mut u32,
    ) -> BOOL;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetProcessId(h: HANDLE) -> u32;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn CancelIoEx(h: HANDLE, ov: *mut c_void) -> BOOL;
    fn GetLastError() -> u32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut SHELLEXECUTEINFOW) -> BOOL;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn invalid_handle(h: HANDLE) -> bool {
    h.is_null() || h as isize == -1
}

fn err(code: &'static str, message: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: message.into(),
        hint,
    }
}

fn bad(m: impl Into<String>) -> ActError {
    err("INVALID_ARGS", m, "See docs/04-mcp-tool-contract.md.")
}

/// A process handle closed on drop.
struct Proc(HANDLE);
unsafe impl Send for Proc {}
impl Drop for Proc {
    fn drop(&mut self) {
        if !invalid_handle(self.0) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// UI methods the core may forward to the helper (ADR 0030). Launching, screenshots, OCR,
/// the clipboard, skills and the app map always stay in the core.
pub const ROUTABLE: &[&str] = &[
    "observe",
    "find",
    "read_text",
    "wait_for",
    "set_value",
    "click",
    "type",
    "press_keys",
    "scroll",
    "drag",
    "mouse",
    "focus_window",
    "close_window",
    "window_bounds",
];

const INPUT_METHODS: &[&str] = &["type", "press_keys", "set_value", "click"];

/// Processes the helper never touches, whatever it is asked (not from user-writable files).
const HARD_DENY: &[&str] = &[
    "sechealthui.exe",
    "securityhealthsystray.exe",
    "consent.exe",
    "logonui.exe",
    "credentialuibroker.exe",
    "xtraos-core.exe",
];

// =============================================================================================
// Core side

struct Link {
    writer: File,
    reader: BufReader<File>,
    proc: Proc,
    pid: u32,
    next: u64,
    started: Instant,
    elevated: bool,
}

static LINK: Mutex<Option<Link>> = Mutex::new(None);
/// The reader's raw handle, so the kill switch can abort a pending read (CancelIoEx) without
/// waiting for the lock. Cleared before the handle is closed.
static RAW_READER: Mutex<Option<usize>> = Mutex::new(None);
static CONNECTED: AtomicBool = AtomicBool::new(false);

fn link() -> std::sync::MutexGuard<'static, Option<Link>> {
    LINK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn connected() -> bool {
    CONNECTED.load(Ordering::SeqCst)
}

fn drop_link(g: &mut Option<Link>) {
    *RAW_READER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    CONNECTED.store(false, Ordering::SeqCst);
    if let Some(l) = g.take() {
        drop(l.writer);
        drop(l.reader); // closing the pipe ends the helper
        unsafe { WaitForSingleObject(l.proc.0, 3000) };
    }
}

/// Kill switch: end the helper. A request in flight is aborted (its reader is cancelled and
/// `forward` drops the link).
pub fn on_kill_switch() {
    if let Ok(mut g) = LINK.try_lock() {
        if g.is_some() {
            drop_link(&mut g);
        }
        return;
    }
    if let Some(h) = *RAW_READER.lock().unwrap_or_else(|e| e.into_inner()) {
        unsafe { CancelIoEx(h as HANDLE, std::ptr::null_mut()) };
    }
}

/// `{admin_helper: …}` for `status`.
pub fn status_json() -> Value {
    match link().as_ref() {
        Some(l) => json!({"running": true, "pid": l.pid, "elevated": l.elevated,
                          "up_s": l.started.elapsed().as_secs()}),
        None => json!({"running": false}),
    }
}

/// The `elevated_helper` method: op start | stop | status.
pub fn run(p: &Value) -> Result<Value, ActError> {
    match p.get("op").and_then(Value::as_str).unwrap_or("status") {
        "start" => {
            let exe = std::env::current_exe()
                .map_err(|e| err("INTERNAL", format!("own path: {e}"), ""))?;
            start(&exe, true)
        }
        "stop" => {
            let was = connected();
            drop_link(&mut link());
            Ok(
                json!({"text": if was { "ok admin helper stopped" } else { "admin helper was not running" }}),
            )
        }
        "status" => {
            let s = status_json();
            let text = if s["running"] == true {
                format!(
                    "admin helper running (pid {}, {}), up {} s",
                    s["pid"],
                    if s["elevated"] == true {
                        "elevated"
                    } else {
                        "NOT elevated"
                    },
                    s["up_s"]
                )
            } else {
                "admin helper not running".to_string()
            };
            Ok(json!({"text": text, "helper": s}))
        }
        o => Err(bad(format!("unknown op '{o}' (start, stop, status)"))),
    }
}

/// Starts the helper WITHOUT elevation (test mode, `--test-same-integrity`). It gains no
/// privilege; it lets automated tests exercise the channel, routing and approvals.
pub fn start_unelevated_for_tests(exe: &Path) -> Result<Value, ActError> {
    start(exe, false)
}

fn random_name() -> Result<String, ActError> {
    let b = winsec::random_bytes(16).map_err(|e| err("INTERNAL", format!("rng: {e}"), ""))?;
    Ok(format!(
        "{PIPE_PREFIX}{}",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    ))
}

fn start(exe: &Path, elevated: bool) -> Result<Value, ActError> {
    let mut g = link();
    if g.is_some() {
        drop(g);
        let mut s = run(&json!({"op": "status"}))?;
        s["already_running"] = json!(true);
        return Ok(s);
    }
    let name = random_name()?;
    let params = format!(
        "--elevated-helper --pipe {name} --core-pid {}{}",
        std::process::id(),
        if elevated {
            ""
        } else {
            " --test-same-integrity"
        }
    );
    let file = wide(&exe.to_string_lossy());
    let params_w = wide(&params);
    let verb = wide(if elevated { "runas" } else { "open" });
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        hwnd: std::ptr::null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params_w.as_ptr(),
        lpDirectory: std::ptr::null(),
        nShow: SW_HIDE,
        hInstApp: std::ptr::null_mut(),
        lpIDList: std::ptr::null_mut(),
        lpClass: std::ptr::null(),
        hkeyClass: std::ptr::null_mut(),
        dwHotKey: 0,
        hIconOrMonitor: std::ptr::null_mut(),
        hProcess: std::ptr::null_mut(),
    };
    // Blocks while Windows shows the UAC prompt (secure desktop: only the human answers it).
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let code = unsafe { GetLastError() };
        return Err(if code == ERROR_CANCELLED {
            err(
                "NEEDS_HUMAN",
                "the user declined the administrator (UAC) prompt: the admin helper is not running",
                "Ask the user before trying again.",
            )
        } else {
            err(
                "INTERNAL",
                format!("could not start the admin helper (Win32 error {code})"),
                "",
            )
        });
    }
    if invalid_handle(info.hProcess) {
        return Err(err(
            "INTERNAL",
            "no process handle for the admin helper",
            "",
        ));
    }
    let proc = Proc(info.hProcess);
    let helper_pid = unsafe { GetProcessId(proc.0) };

    // Connect: wait for the helper's pipe, then check who answers.
    let wname = wide(&name);
    let deadline = Instant::now() + Duration::from_secs(20);
    let h = loop {
        let h = unsafe {
            CreateFileW(
                wname.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if !invalid_handle(h) {
            break h;
        }
        let e = unsafe { GetLastError() };
        if unsafe { WaitForSingleObject(proc.0, 0) } == WAIT_OBJECT_0 {
            let mut code = 0u32;
            unsafe { GetExitCodeProcess(proc.0, &mut code) };
            return Err(err(
                "INTERNAL",
                format!("the admin helper exited before connecting (code {code})"),
                "Exit codes: 3 test mode refused, 4/5 core check failed, 6 pipe name taken.",
            ));
        }
        if Instant::now() > deadline {
            return Err(err(
                "TIMEOUT",
                "the admin helper did not open its pipe within 20 s",
                "",
            ));
        }
        if e == ERROR_PIPE_BUSY {
            unsafe { WaitNamedPipeW(wname.as_ptr(), 500) };
        } else if e != ERROR_FILE_NOT_FOUND {
            return Err(err("INTERNAL", format!("helper pipe: Win32 error {e}"), ""));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let writer = unsafe { File::from_raw_handle(h as RawHandle) };
    let mut server_pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(h, &mut server_pid) } == 0 || server_pid != helper_pid {
        drop(writer);
        return Err(err(
            "BLOCKED_BY_POLICY",
            format!("another process (pid {server_pid}) answered on the helper's pipe: refused"),
            "Something on this PC tried to impersonate the admin helper.",
        ));
    }
    let reader = BufReader::new(
        writer
            .try_clone()
            .map_err(|e| err("INTERNAL", format!("pipe: {e}"), ""))?,
    );
    let mut l = Link {
        writer,
        reader,
        proc,
        pid: helper_pid,
        next: 0,
        started: Instant::now(),
        elevated: false,
    };
    let hello = exchange(&mut l, "hello", &json!({}))?;
    l.elevated = hello["elevated"].as_bool().unwrap_or(false);
    if elevated && !l.elevated {
        return Err(err("INTERNAL", "the admin helper is not elevated", ""));
    }
    *RAW_READER.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(l.reader.get_ref().as_raw_handle() as usize);
    let pid = l.pid;
    let el = l.elevated;
    *g = Some(l);
    CONNECTED.store(true, Ordering::SeqCst);
    Ok(json!({
        "text": format!("ok admin helper running (pid {pid}{}): admin windows can now be read and controlled, each after the user's OK in the helper's own dialog",
            if el { ", elevated" } else { ", NOT elevated: test mode" }),
        "helper": {"running": true, "pid": pid, "elevated": el},
    }))
}

/// One request/reply on the link.
fn exchange(l: &mut Link, method: &str, params: &Value) -> Result<Value, ActError> {
    l.next += 1;
    let id = l.next;
    let mut line = serde_json::to_vec(&json!({"id": id, "method": method, "params": params}))
        .map_err(|e| err("INTERNAL", e.to_string(), ""))?;
    line.push(b'\n');
    let lost = |e: std::io::Error| {
        err(
            "INTERNAL",
            format!("admin helper connection lost: {e}"),
            "Start it again (a new administrator prompt).",
        )
    };
    l.writer.write_all(&line).map_err(lost)?;
    l.writer.flush().map_err(lost)?;
    let mut reply = String::new();
    let n = (&mut l.reader)
        .take(MAX_LINE)
        .read_line(&mut reply)
        .map_err(lost)?;
    if n == 0 {
        return Err(lost(std::io::Error::other("closed")));
    }
    let v: Value = serde_json::from_str(&reply)
        .map_err(|e| err("INTERNAL", format!("bad helper reply: {e}"), ""))?;
    if v["id"] != id {
        return Err(err("INTERNAL", "helper reply out of order", ""));
    }
    if let Some(r) = v.get("result") {
        return Ok(r.clone());
    }
    let e = &v["error"];
    Err(err(
        known_code(e["code"].as_str().unwrap_or("INTERNAL")),
        format!(
            "admin helper: {}{}",
            e["message"].as_str().unwrap_or("error"),
            e["hint"]
                .as_str()
                .filter(|h| !h.is_empty())
                .map(|h| format!(" ({h})"))
                .unwrap_or_default()
        ),
        "",
    ))
}

fn known_code(c: &str) -> &'static str {
    const CODES: &[&str] = &[
        "INVALID_ARGS",
        "STALE_REF",
        "ELEMENT_NOT_FOUND",
        "TARGET_OBSCURED",
        "TIMEOUT",
        "NEEDS_HUMAN",
        "BLOCKED_BY_POLICY",
        "STOPPED",
        "ELEVATION_REQUIRED",
        "USER_ACTIVE",
        "NOT_IMPLEMENTED",
    ];
    CODES
        .iter()
        .find(|&&k| k == c)
        .copied()
        .unwrap_or("INTERNAL")
}

/// True when the request names a ref from the helper's range.
pub fn owns_refs(p: &Value) -> bool {
    ["ref", "from_ref", "to_ref", "scope_ref"].iter().any(|k| {
        p.get(*k)
            .and_then(perceive::parse_elem_ref)
            .is_some_and(|n| n >= perceive::HELPER_REF_BASE)
    })
}

/// Sends a UI request to the helper.
pub fn forward(method: &str, p: &Value) -> Result<Value, ActError> {
    if !ROUTABLE.contains(&method) {
        return Err(bad(format!(
            "'{method}' cannot be used with admin-window refs"
        )));
    }
    safety::check()?;
    let mut g = link();
    let Some(l) = g.as_mut() else {
        return Err(err(
            "STALE_REF",
            "that ref belongs to the admin helper, which is not running",
            "Start it (elevated_helper start), then observe the admin window again.",
        ));
    };
    let r = exchange(l, method, p);
    if let Err(e) = &r {
        if e.message.starts_with("admin helper connection lost")
            || e.message.starts_with("bad helper reply")
            || e.message == "helper reply out of order"
        {
            drop_link(&mut g);
        }
    }
    if safety::stopped() {
        drop_link(&mut g);
        return Err(safety::stopped_err());
    }
    r.map(|mut v| {
        if let Some(o) = v.as_object_mut() {
            o.insert("via".into(), json!("admin_helper"));
        }
        v
    })
}

// =============================================================================================
// Helper side (`xtraos-core.exe --elevated-helper ...`)

struct HelperArgs {
    pipe: String,
    core_pid: u32,
    test: bool,
}

fn parse_args(args: &[String]) -> Result<HelperArgs, String> {
    let mut pipe = None;
    let mut core_pid = None;
    let mut test = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--elevated-helper" => {}
            "--pipe" => pipe = it.next().cloned(),
            "--core-pid" => core_pid = it.next().and_then(|s| s.parse::<u32>().ok()),
            "--test-same-integrity" => test = true,
            o => return Err(format!("unknown argument {o}")),
        }
    }
    let pipe = pipe.ok_or("--pipe missing")?;
    let suffix = pipe.strip_prefix(PIPE_PREFIX).ok_or("bad pipe name")?;
    if suffix.len() != 32 || !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("bad pipe name".into());
    }
    let core_pid = core_pid.filter(|&p| p != 0).ok_or("--core-pid missing")?;
    Ok(HelperArgs {
        pipe,
        core_pid,
        test,
    })
}

/// SDDL of the helper pipe: only the core's user, no network logons, medium label so the
/// (medium) core may write to it.
pub fn helper_pipe_sddl(user_sid: &str) -> String {
    format!("D:P(D;;GA;;;NU)(A;;GA;;;{user_sid})S:(ML;;NW;;;ME)")
}

static LAST_ACTIVITY_MS: AtomicU64 = AtomicU64::new(0);

fn mark_activity(t0: Instant) {
    LAST_ACTIVITY_MS.store(t0.elapsed().as_millis() as u64, Ordering::SeqCst);
}

struct Ctx {
    core_pid: u32,
    core_integrity: u32,
    test: bool,
    grants: HashMap<usize, (u32, Instant)>,
    recent: VecDeque<Instant>,
}

/// Entry point of the helper process. Returns the exit code.
pub fn helper_main(args: &[String]) -> i32 {
    let a = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[xtraos-helper] {e}");
            return 2;
        }
    };
    let elevated = winmgr::core_is_elevated();
    if a.test && elevated {
        eprintln!("[xtraos-helper] test mode is refused in an elevated process");
        return 3;
    }
    let core = Proc(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            a.core_pid,
        )
    });
    if invalid_handle(core.0) {
        eprintln!("[xtraos-helper] cannot open the core process");
        return 4;
    }
    if !a.test {
        let own = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_lowercase());
        let theirs = winmgr::image_path_of_handle(core.0).map(|p| p.to_lowercase());
        if own.is_none() || own != theirs {
            eprintln!("[xtraos-helper] the client process is not this XtraOS core: refused");
            return 5;
        }
    }
    let core_integrity = winmgr::integrity_of_handle(core.0).unwrap_or(0x2000);
    let Ok(sid) = winsec::user_sid_of_process(core.0) else {
        return 4;
    };
    let Ok(mut sa) = winsec::OwnedSecurityAttributes::from_sddl(&helper_pipe_sddl(&sid)) else {
        return 4;
    };
    let wname = wide(&a.pipe);
    let h = unsafe {
        CreateNamedPipeW(
            wname.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_REJECT_REMOTE_CLIENTS, // byte mode, blocking
            1,
            64 * 1024,
            64 * 1024,
            0,
            sa.as_mut_ptr(),
        )
    };
    if invalid_handle(h) {
        eprintln!("[xtraos-helper] pipe name already taken: refusing to run");
        return 6;
    }
    let pipe = unsafe { File::from_raw_handle(h as RawHandle) };

    safety::set_helper_mode();
    if !safety::ensure_started() {
        eprintln!("[xtraos-helper] safety UI unavailable");
        return 9;
    }
    crate::input::init_dpi();
    perceive::set_ref_base(perceive::HELPER_REF_BASE);
    let t0 = Instant::now();

    // Exit with the core.
    let core_raw = core.0 as usize;
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(core_raw as HANDLE, INFINITE) };
        std::process::exit(0);
    });
    // Nobody connects in time → exit.
    let connected = std::sync::Arc::new(AtomicBool::new(false));
    let c2 = connected.clone();
    std::thread::spawn(move || {
        std::thread::sleep(CONNECT_WAIT);
        if !c2.load(Ordering::SeqCst) {
            std::process::exit(7);
        }
    });

    if unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) } == 0
        && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED
    {
        return 7;
    }
    let mut client = 0u32;
    if unsafe { GetNamedPipeClientProcessId(h, &mut client) } == 0 || client != a.core_pid {
        eprintln!("[xtraos-helper] client pid {client} is not the core: refused");
        unsafe { DisconnectNamedPipe(h) };
        return 8;
    }
    connected.store(true, Ordering::SeqCst);
    mark_activity(t0);

    // Idle, tray Stop, or the core hung up (seen by peeking while no read is pending).
    let peek = pipe.try_clone().ok();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        if safety::stopped() {
            std::process::exit(0);
        }
        let idle = t0.elapsed().as_millis() as u64 - LAST_ACTIVITY_MS.load(Ordering::SeqCst);
        if idle > IDLE_EXIT.as_millis() as u64 {
            std::process::exit(0);
        }
        if let Some(p) = &peek {
            let mut avail = 0u32;
            let ok = unsafe {
                PeekNamedPipe(
                    p.as_raw_handle() as HANDLE,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut avail,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                std::process::exit(0);
            }
        }
    });

    let mut ctx = Ctx {
        core_pid: a.core_pid,
        core_integrity,
        test: a.test,
        grants: HashMap::new(),
        recent: VecDeque::new(),
    };
    let Ok(rd) = pipe.try_clone() else { return 4 };
    let mut reader = BufReader::new(rd);
    let mut writer = pipe;
    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => return 0,
            Ok(_) => {}
        }
        mark_activity(t0);
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                let id = req["id"].clone();
                let method = req["method"].as_str().unwrap_or("");
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let r = if method == "hello" {
                    Ok(json!({"pid": std::process::id(), "elevated": elevated,
                              "integrity": winmgr::own_integrity(), "version": crate::protocol::VERSION}))
                } else {
                    serve(&mut ctx, method, &params)
                };
                match r {
                    Ok(v) => json!({"id": id, "result": v}),
                    Err(e) => {
                        json!({"id": id, "error": {"code": e.code, "message": e.message, "hint": e.hint}})
                    }
                }
            }
            Err(_) => return 2,
        };
        mark_activity(t0);
        let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
        bytes.push(b'\n');
        if writer
            .write_all(&bytes)
            .and_then(|_| writer.flush())
            .is_err()
        {
            return 0;
        }
    }
}

fn serve(ctx: &mut Ctx, method: &str, p: &Value) -> Result<Value, ActError> {
    if !ROUTABLE.contains(&method) {
        return Err(bad(format!("the admin helper does not do '{method}'")));
    }
    safety::check()?;
    let now = Instant::now();
    while ctx
        .recent
        .front()
        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
    {
        ctx.recent.pop_front();
    }
    if ctx.recent.len() >= RATE_PER_MIN {
        return Err(safety::blocked(format!(
            "more than {RATE_PER_MIN} admin-window actions per minute"
        )));
    }
    ctx.recent.push_back(now);
    approve(ctx, method, p)?;
    crate::exec::dispatch_local(method, p)
}

/// Why the helper refuses a target window, if it does (pure rule; unit-tested).
pub fn refuse_reason(
    process: &str,
    pid: u32,
    core_pid: u32,
    integrity: Option<u32>,
    core_integrity: u32,
    test: bool,
) -> Option<String> {
    if pid == core_pid {
        return Some("that window belongs to the XtraOS core".into());
    }
    if HARD_DENY.contains(&process) {
        return Some(format!("{process} is never automated"));
    }
    if !test && integrity.unwrap_or(u32::MAX) <= core_integrity {
        return Some("not an administrator window: the core handles it without the helper".into());
    }
    None
}

fn grant_valid(g: Option<&(u32, Instant)>, h: HWND, now: Instant) -> bool {
    g.is_some_and(|&(pid, until)| now < until && winmgr::exists(h) && winmgr::pid_of(h) == pid)
}

fn approve(ctx: &mut Ctx, method: &str, p: &Value) -> Result<(), ActError> {
    let (f, mut what, hwnds) = safety::gather_targets(method, p);
    if hwnds.is_empty() {
        return Err(bad(
            "the admin helper needs a target: a window or a ref from an admin window",
        ));
    }
    if f.own_window {
        return Err(safety::blocked(
            "that is an XtraOS window: only the user may use it",
        ));
    }
    let mut procs = Vec::new();
    for &h in &hwnds {
        let pid = winmgr::pid_of(h);
        let pname = winmgr::process_name(pid);
        if let Some(why) = refuse_reason(
            &pname,
            pid,
            ctx.core_pid,
            winmgr::integrity_of_window(h),
            ctx.core_integrity,
            ctx.test,
        ) {
            return Err(safety::blocked(why));
        }
        procs.push(pname);
    }
    if f.password && INPUT_METHODS.contains(&method) {
        return Err(safety::blocked(
            "the target is a password field: the user types passwords",
        ));
    }
    let now = Instant::now();
    if hwnds
        .iter()
        .all(|&h| grant_valid(ctx.grants.get(&(h as usize)), h, now))
    {
        return Ok(());
    }
    // Show what will reach the admin window.
    match method {
        "type" => {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                what = format!("{what}\n\nText: \"{}\"", perceive::clip(t, 200));
            }
        }
        "set_value" => {
            if let Some(t) = p.get("value").and_then(Value::as_str) {
                what = format!("{what}\n\nValue: \"{}\"", perceive::clip(t, 200));
            }
        }
        _ => {}
    }
    let before = winmgr::foreground_root();
    let answer = safety::prompt(
        "XtraOS admin helper — approval needed",
        &format!(
            "The AI agent wants to:\n\n{what}\n\nThis is an ADMINISTRATOR window ({}). \
             Actions here can change the whole PC.",
            procs.join(", ")
        ),
        vec![
            "Allow once".into(),
            "Allow this window for 10 minutes".into(),
            "Deny".into(),
        ],
        2,
        Duration::from_secs(60),
    )?;
    if !before.is_null() && winmgr::exists(before) && !safety::is_own_window(before) {
        winmgr::focus(before);
        std::thread::sleep(Duration::from_millis(150));
    }
    match answer {
        Some(0) => Ok(()),
        Some(1) => {
            for &h in &hwnds {
                ctx.grants
                    .insert(h as usize, (winmgr::pid_of(h), now + GRANT));
            }
            Ok(())
        }
        Some(_) => Err(safety::blocked("denied by the user in the admin helper")),
        None => Err(safety::blocked("not approved within 60 s")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn helper_args_are_strict() {
        let ok = parse_args(&args(
            r"--elevated-helper --pipe \\.\pipe\xtraos-elev-0123456789abcdef0123456789abcdef --core-pid 42",
        ))
        .unwrap();
        assert_eq!(ok.core_pid, 42);
        assert!(!ok.test);
        assert!(parse_args(&args(
            r"--elevated-helper --pipe \\.\pipe\other --core-pid 4"
        ))
        .is_err());
        assert!(parse_args(&args(
            r"--elevated-helper --pipe \\.\pipe\xtraos-elev-xyz --core-pid 4"
        ))
        .is_err());
        assert!(parse_args(&args(
            r"--elevated-helper --pipe \\.\pipe\xtraos-elev-0123456789abcdef0123456789abcdef"
        ))
        .is_err());
        assert!(parse_args(&args(
            r"--elevated-helper --pipe \\.\pipe\xtraos-elev-0123456789abcdef0123456789abcdef --core-pid 4 --run cmd"
        ))
        .is_err());
    }

    #[test]
    fn pipe_sddl_parses_and_has_medium_label() {
        let s = helper_pipe_sddl(&winsec::current_user_sid().unwrap());
        assert!(s.contains("(D;;GA;;;NU)") && s.ends_with("S:(ML;;NW;;;ME)"));
        let sa = winsec::OwnedSecurityAttributes::from_sddl(&s).unwrap();
        assert!(!sa.attrs.lpSecurityDescriptor.is_null());
    }

    #[test]
    fn refusal_rules() {
        let me = 0x2000;
        let high = Some(0x3000);
        assert!(refuse_reason("cmd.exe", 10, 99, high, me, false).is_none());
        assert!(
            refuse_reason("cmd.exe", 99, 99, high, me, false).is_some(),
            "core's own"
        );
        assert!(refuse_reason("consent.exe", 10, 99, high, me, false).is_some());
        assert!(refuse_reason("sechealthui.exe", 10, 99, high, me, false).is_some());
        assert!(
            refuse_reason("notepad.exe", 10, 99, Some(0x2000), me, false).is_some(),
            "not admin"
        );
        assert!(
            refuse_reason("notepad.exe", 10, 99, Some(0x2000), me, true).is_none(),
            "test mode"
        );
        assert!(
            refuse_reason("cmd.exe", 10, 99, None, me, false).is_none(),
            "unreadable = admin"
        );
    }

    #[test]
    fn refs_route_by_range() {
        assert!(owns_refs(&json!({"ref": "e50000001"})));
        assert!(owns_refs(&json!({"to_ref": "e50000001", "from_ref": "e3"})));
        assert!(!owns_refs(&json!({"ref": "e49999999"})));
        assert!(!owns_refs(&json!({"window": "w5"})));
        assert!(known_code("BLOCKED_BY_POLICY") == "BLOCKED_BY_POLICY");
        assert_eq!(known_code("WHATEVER"), "INTERNAL");
    }

    #[test]
    fn grants_expire_and_need_the_window() {
        let now = Instant::now();
        let bogus = 0x1234usize as HWND;
        assert!(!grant_valid(None, bogus, now));
        assert!(
            !grant_valid(Some(&(1, now + GRANT)), bogus, now),
            "window gone"
        );
    }
}
