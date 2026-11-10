//! Windows & apps (Phase 1 M3, ADR 0012): list, focus, launch, close, bounds, and the
//! elevation (UIPI) boundary used by input too.
//!
//! Win32 docs:
//! EnumWindows            https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-enumwindows
//! DwmGetWindowAttribute  https://learn.microsoft.com/windows/win32/api/dwmapi/nf-dwmapi-dwmgetwindowattribute
//! ShellExecuteExW        https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw
//! SetForegroundWindow    https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setforegroundwindow
//! Integrity levels       https://learn.microsoft.com/windows/win32/secauthz/mandatory-integrity-control
#![allow(non_snake_case, clippy::upper_case_acronyms)]
// HWNDs are opaque OS handles: this module never dereferences them, it only passes them to
// Win32, which validates them (IsWindow). So taking them in safe public fns is sound.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::input::{ActError, HWND};

type BOOL = i32;
type HANDLE = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

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

#[repr(C)]
struct SID_AND_ATTRIBUTES {
    Sid: *mut c_void,
    Attributes: u32,
}

const GWL_EXSTYLE: i32 = -20;
const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
const GW_OWNER: u32 = 4;
const GA_ROOT: u32 = 2;
const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
const DWMWA_CLOAKED: u32 = 14;
const SW_MAXIMIZE: i32 = 3;
const SW_MINIMIZE: i32 = 6;
const SW_RESTORE: i32 = 9;
const SW_SHOWNORMAL: i32 = 1;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const WM_CLOSE: u32 = 0x0010;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
const SEE_MASK_FLAG_NO_UI: u32 = 0x0000_0400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_INTEGRITY_LEVEL: u32 = 25;
const TOKEN_ELEVATION: u32 = 20;
const ERROR_CANCELLED: i32 = 1223;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PATH_NOT_FOUND: i32 = 3;
const ERROR_NO_ASSOCIATION: i32 = 1155;

type EnumProc = unsafe extern "system" fn(HWND, isize) -> BOOL;

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(cb: EnumProc, lparam: isize) -> BOOL;
    fn IsWindow(h: HWND) -> BOOL;
    fn IsWindowVisible(h: HWND) -> BOOL;
    fn IsIconic(h: HWND) -> BOOL;
    fn IsZoomed(h: HWND) -> BOOL;
    fn GetWindowTextW(h: HWND, buf: *mut u16, n: i32) -> i32;
    fn GetWindowTextLengthW(h: HWND) -> i32;
    fn GetWindowLongPtrW(h: HWND, idx: i32) -> isize;
    fn GetWindow(h: HWND, cmd: u32) -> HWND;
    fn GetAncestor(h: HWND, flags: u32) -> HWND;
    fn GetWindowThreadProcessId(h: HWND, pid: *mut u32) -> u32;
    fn GetForegroundWindow() -> HWND;
    fn SetForegroundWindow(h: HWND) -> BOOL;
    fn BringWindowToTop(h: HWND) -> BOOL;
    fn ShowWindow(h: HWND, cmd: i32) -> BOOL;
    fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> BOOL;
    fn GetWindowRect(h: HWND, r: *mut RECT) -> BOOL;
    fn AttachThreadInput(a: u32, b: u32, attach: BOOL) -> BOOL;
    fn PostMessageW(h: HWND, msg: u32, w: usize, l: isize) -> BOOL;
    fn FindWindowExW(parent: HWND, after: HWND, class: *const u16, title: *const u16) -> HWND;
    fn GetClassNameW(h: HWND, buf: *mut u16, n: i32) -> i32;
    fn IsWindowEnabled(h: HWND) -> BOOL;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(h: HWND, attr: u32, out: *mut c_void, size: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn QueryFullProcessImageNameW(p: HANDLE, flags: u32, buf: *mut u16, size: *mut u32) -> BOOL;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> HANDLE;
    fn GetProcessId(p: HANDLE) -> u32;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(p: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(
        t: HANDLE,
        class: u32,
        info: *mut c_void,
        len: u32,
        ret: *mut u32,
    ) -> BOOL;
    fn GetSidSubAuthorityCount(sid: *mut c_void) -> *mut u8;
    fn GetSidSubAuthority(sid: *mut c_void, i: u32) -> *mut u32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut SHELLEXECUTEINFOW) -> BOOL;
}

// ---------------------------------------------------------------------------------------------
// Errors (same shape as input)

fn err(code: &'static str, message: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: message.into(),
        hint,
    }
}

fn bad(message: impl Into<String>) -> ActError {
    err(
        "INVALID_ARGS",
        message,
        "Check the tool schema in docs/04-mcp-tool-contract.md.",
    )
}

pub fn elevation_required(what: &str) -> ActError {
    err(
        "ELEVATION_REQUIRED",
        format!("{what}: the window belongs to an elevated (administrator) process"),
        "Windows blocks normal programs from admin windows (UIPI). Start the admin helper with elevated_helper {op: \"start\"}: the user approves a Windows prompt.",
    )
}

// ---------------------------------------------------------------------------------------------
// Window refs

/// The ref handed to the agent. Handing it out also remembers which process instance owns the
/// window, so a later use can tell when Windows gave the handle to another app (P6-M1b).
pub fn window_ref(h: HWND) -> String {
    remember(h);
    format!("w{}", h as usize)
}

// ---------------------------------------------------------------------------------------------
// Reused handles (P6-M1b, ADR 0046): Windows recycles window handles. A ref that outlives its
// window can silently point at another app's window. (pid, process start time) identifies one
// process instance (as in ADR 0041).

type Identity = (u32, u64);

static HANDED_OUT: Mutex<Option<HashMap<usize, Identity>>> = Mutex::new(None);

fn identity(h: HWND) -> Option<Identity> {
    if h.is_null() || unsafe { IsWindow(h) } == 0 {
        return None;
    }
    let pid = pid_of(h);
    (pid != 0).then(|| (pid, process_start_time(pid).unwrap_or(0)))
}

fn remember(h: HWND) {
    let Some(id) = identity(h) else { return };
    let mut guard = HANDED_OUT.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() > 4096 {
        map.retain(|&k, _| unsafe { IsWindow(k as HWND) } != 0);
    }
    map.insert(h as usize, id);
}

/// Pure: a handle never handed out is accepted; a remembered one must be the same instance.
fn same_instance(remembered: Option<Identity>, now: Identity) -> bool {
    remembered.is_none_or(|r| r == now)
}

fn check_not_reused(h: HWND) -> Result<(), ActError> {
    let Some(now) = identity(h) else {
        return Ok(());
    };
    let remembered = HANDED_OUT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&(h as usize)).copied());
    if same_instance(remembered, now) {
        return Ok(());
    }
    // No window_ref() here: it would remember the new owner and hide the reuse.
    let mut t = title(h);
    if t.chars().count() > 60 {
        t = t.chars().take(59).collect::<String>() + "…";
    }
    Err(err(
        "STALE_REF",
        format!(
            "the window of w{} closed; its handle now belongs to {} \"{}\"",
            h as usize,
            process_name(now.0),
            t
        ),
        "Call list_windows and use the new ref.",
    ))
}

// ---------------------------------------------------------------------------------------------
// Window by text (P6-M1a, ADR 0046)

/// Pure matcher: indexes of `(title, process)` candidates that match `text`. An exact title
/// wins; otherwise the title contains the text or the process name equals it (with or without
/// `.exe`). Case-insensitive.
pub fn match_windows(text: &str, candidates: &[(String, String)]) -> Vec<usize> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Vec::new();
    }
    let lower: Vec<(String, String)> = candidates
        .iter()
        .map(|(title, process)| (title.to_lowercase(), process.to_lowercase()))
        .collect();
    let exact: Vec<usize> = (0..lower.len()).filter(|&i| lower[i].0 == t).collect();
    if !exact.is_empty() {
        return exact;
    }
    (0..lower.len())
        .filter(|&i| {
            let (title, process) = &lower[i];
            title.contains(&t) || *process == t || process.strip_suffix(".exe") == Some(t.as_str())
        })
        .collect()
}

fn resolve_by_text(text: &str) -> Result<HWND, ActError> {
    let windows: Vec<HWND> = listable_windows()
        .into_iter()
        .filter(|&h| !crate::safety::is_own_window(h))
        .collect();
    let candidates: Vec<(String, String)> = windows
        .iter()
        .map(|&h| (title(h), process_name(pid_of(h))))
        .collect();
    let hits = match_windows(text, &candidates);
    match hits.len() {
        0 => Err(err(
            "WINDOW_NOT_FOUND",
            format!("no window matches \"{}\"", text.trim()),
            "Call list_windows; pass a ref, part of a title, or a process name like \"chrome\".",
        )),
        1 => Ok(windows[hits[0]]),
        n => {
            let list: Vec<String> = hits
                .iter()
                .take(5)
                .map(|&i| {
                    let t: String = candidates[i].0.chars().take(50).collect();
                    format!("{} \"{}\"", window_ref(windows[i]), t)
                })
                .collect();
            Err(err(
                "INVALID_ARGS",
                format!(
                    "'window' \"{}\" matches {n} windows: {}{}",
                    text.trim(),
                    list.join(", "),
                    if n > 5 { ", …" } else { "" }
                ),
                "Pass the ref of the window you mean, or a longer part of its title.",
            ))
        }
    }
}

/// Parses `"w123"` or `123` (pure; no OS calls).
pub fn parse_ref(v: &Value) -> Option<usize> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| n as usize),
        Value::String(s) => s.strip_prefix('w').and_then(|d| d.parse::<usize>().ok()),
        _ => None,
    }
    .filter(|&n| n != 0)
}

/// The window that shows the desktop icons: Progman, or a WorkerW (wallpaper slideshow), the
/// one holding SHELLDLL_DefView. It is not in list_windows (no Alt+Tab entry).
pub fn desktop_window() -> Option<HWND> {
    let defview = wide("SHELLDLL_DefView");
    for class in ["Progman", "WorkerW"] {
        let class = wide(class);
        let mut h: HWND = std::ptr::null_mut();
        loop {
            h = unsafe { FindWindowExW(std::ptr::null_mut(), h, class.as_ptr(), std::ptr::null()) };
            if h.is_null() {
                break;
            }
            let view = unsafe {
                FindWindowExW(h, std::ptr::null_mut(), defview.as_ptr(), std::ptr::null())
            };
            if !view.is_null() {
                return Some(h);
            }
        }
    }
    None
}

/// Resolves a `window` argument to an existing top-level window handle. `"desktop"` is the
/// desktop icon view.
pub fn resolve_window(v: &Value) -> Result<HWND, ActError> {
    if v.as_str() == Some("desktop") {
        return desktop_window().ok_or_else(|| {
            err(
                "WINDOW_NOT_FOUND",
                "no desktop icon view (is Explorer running?)",
                "",
            )
        });
    }
    let n = match parse_ref(v) {
        Some(n) => n,
        // P6-M1a: part of a title, or a process name.
        None => match v.as_str() {
            Some(text) if !text.trim().is_empty() => return resolve_by_text(text),
            _ => {
                return Err(bad(
                    "window must be a ref like \"w123\", part of a title, a process name, or \"desktop\"",
                ))
            }
        },
    };
    let h = n as HWND;
    if unsafe { IsWindow(h) } == 0 {
        return Err(err(
            "WINDOW_NOT_FOUND",
            format!("no window w{n}"),
            "It may have closed. Call list_windows.",
        ));
    }
    check_not_reused(h)?;
    let root = unsafe { GetAncestor(h, GA_ROOT) };
    if crate::safety::is_own_window(root) {
        return Err(crate::safety::blocked(
            "that is an XtraOS window (overlay/dialog/tray): only the user may use it",
        ));
    }
    Ok(root)
}

// ---------------------------------------------------------------------------------------------
// Process facts

pub fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    pid
}

/// Creation time of a process (FILETIME as u64): with the PID it identifies one process
/// instance, so a recycled PID never inherits anything (ADR 0041).
pub fn process_start_time(pid: u32) -> Option<u64> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetProcessTimes(p: HANDLE, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> BOOL;
    }
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return None;
        }
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        let ok = GetProcessTimes(p, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(p);
        (ok != 0).then_some(c)
    }
}

/// Full image path of a process by handle (needs PROCESS_QUERY_LIMITED_INFORMATION).
pub fn image_path_of_handle(p: *mut c_void) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut n = buf.len() as u32;
    if unsafe { QueryFullProcessImageNameW(p, 0, buf.as_mut_ptr(), &mut n) } == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..n as usize]))
}

pub fn process_name(pid: u32) -> String {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return "?".into();
        }
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, 0, buf.as_mut_ptr(), &mut n);
        CloseHandle(p);
        if ok == 0 {
            return "?".into();
        }
        let full = String::from_utf16_lossy(&buf[..n as usize]);
        full.rsplit(['\\', '/'])
            .next()
            .unwrap_or(&full)
            .to_ascii_lowercase()
    }
}

/// Integrity level RID of a process (0x2000 medium, 0x3000 high, 0x4000 system), if readable.
fn integrity_of_process(p: HANDLE) -> Option<u32> {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(p, TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let mut need = 0u32;
        GetTokenInformation(
            tok,
            TOKEN_INTEGRITY_LEVEL,
            std::ptr::null_mut(),
            0,
            &mut need,
        );
        // u64 buffer: 8-byte aligned for the SID_AND_ATTRIBUTES cast below (ADR 0041).
        let mut buf = vec![0u64; (need.max(1) as usize).div_ceil(8)];
        let ok = GetTokenInformation(
            tok,
            TOKEN_INTEGRITY_LEVEL,
            buf.as_mut_ptr().cast(),
            need,
            &mut need,
        );
        CloseHandle(tok);
        if ok == 0 {
            return None;
        }
        let label = &*(buf.as_ptr() as *const SID_AND_ATTRIBUTES);
        let count = *GetSidSubAuthorityCount(label.Sid) as u32;
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(label.Sid, count - 1))
    }
}

fn token_elevated(p: HANDLE) -> Option<bool> {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(p, TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let mut v = 0u32;
        let mut n = 0u32;
        let ok = GetTokenInformation(tok, TOKEN_ELEVATION, (&mut v as *mut u32).cast(), 4, &mut n);
        CloseHandle(tok);
        if ok == 0 {
            None
        } else {
            Some(v != 0)
        }
    }
}

pub fn own_integrity() -> u32 {
    integrity_of_process(unsafe { GetCurrentProcess() }).unwrap_or(0x2000)
}

/// Integrity level of a process by handle (needs PROCESS_QUERY_LIMITED_INFORMATION).
pub fn integrity_of_handle(p: *mut c_void) -> Option<u32> {
    integrity_of_process(p)
}

/// Integrity level of the process that owns a window (None if unreadable).
pub fn integrity_of_window(h: HWND) -> Option<u32> {
    let pid = pid_of(h);
    if pid == 0 {
        return None;
    }
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return None;
        }
        let il = integrity_of_process(p);
        CloseHandle(p);
        il
    }
}

/// True if the window's process runs at a higher integrity level than the core, or its token
/// is unreadable (conservative: we must not try to drive it; UIPI would drop input silently).
/// Top-level window under a screen point.
pub fn root_at(p: crate::input::POINT) -> HWND {
    crate::input::root_at(p)
}

// ---------------------------------------------------------------------------------------------
// Resource discipline (ADR 0028): which windows did the agent open?

static AGENT_WINDOWS: std::sync::Mutex<Vec<(usize, String)>> = std::sync::Mutex::new(Vec::new());

fn agent_windows() -> std::sync::MutexGuard<'static, Vec<(usize, String)>> {
    AGENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Remember a window the agent opened, with the launch key (app + args) used for reuse.
pub fn mark_opened(h: HWND, key: &str) {
    let mut g = agent_windows();
    g.retain(|(w, _)| *w != h as usize && unsafe { IsWindow(*w as HWND) } != 0);
    g.push((h as usize, key.to_string()));
}

pub fn opened_by_agent(h: HWND) -> bool {
    agent_windows().iter().any(|(w, _)| *w == h as usize)
}

fn forget(h: HWND) {
    agent_windows().retain(|(w, _)| *w != h as usize);
}

/// A still-open, visible window the agent opened with this key.
fn reusable(key: &str) -> Option<HWND> {
    let mut g = agent_windows();
    g.retain(|(w, _)| unsafe { IsWindow(*w as HWND) } != 0);
    g.iter()
        .find(|(w, k)| k == key && unsafe { IsWindowVisible(*w as HWND) } != 0)
        .map(|(w, _)| *w as HWND)
}

/// Agent-opened windows that are still open.
pub fn agent_open_windows() -> Vec<HWND> {
    let mut g = agent_windows();
    g.retain(|(w, _)| unsafe { IsWindow(*w as HWND) } != 0);
    g.iter().map(|(w, _)| *w as HWND).collect()
}

pub fn exists(h: HWND) -> bool {
    !h.is_null() && unsafe { IsWindow(h) } != 0
}

pub fn is_minimized(h: HWND) -> bool {
    unsafe { IsIconic(h) != 0 }
}

/// Full window rectangle including invisible resize borders (see `frame` for the visible one).
pub fn window_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) };
    r
}

pub fn is_elevated(h: HWND) -> bool {
    let pid = pid_of(h);
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return true;
        }
        let il = integrity_of_process(p);
        CloseHandle(p);
        match il {
            Some(level) => level > own_integrity(),
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Window facts

pub fn title(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

fn cloaked(h: HWND) -> bool {
    let mut v = 0u32;
    let hr = unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, (&mut v as *mut u32).cast(), 4) };
    hr == 0 && v != 0
}

/// Visible frame (no invisible resize borders); falls back to GetWindowRect.
pub fn frame(h: HWND) -> RECT {
    let mut r = RECT::default();
    let hr = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut r as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if hr != 0 {
        unsafe { GetWindowRect(h, &mut r) };
    }
    r
}

fn state(h: HWND) -> &'static str {
    unsafe {
        if IsIconic(h) != 0 {
            "minimized"
        } else if IsZoomed(h) != 0 {
            "maximized"
        } else {
            "normal"
        }
    }
}

fn is_listable(h: HWND) -> bool {
    unsafe {
        IsWindowVisible(h) != 0
            && GetWindow(h, GW_OWNER).is_null()
            && (GetWindowLongPtrW(h, GWL_EXSTYLE) & WS_EX_TOOLWINDOW) == 0
            && GetWindowTextLengthW(h) > 0
            && !cloaked(h)
    }
}

unsafe extern "system" fn collect(h: HWND, lparam: isize) -> BOOL {
    let v = &mut *(lparam as *mut Vec<HWND>);
    if is_listable(h) {
        v.push(h);
    }
    1
}

/// Top-level, Alt+Tab-like windows in Z order (front first).
pub fn listable_windows() -> Vec<HWND> {
    let mut v: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(collect, (&mut v as *mut Vec<HWND>) as isize) };
    v
}

fn describe(h: HWND, fg: HWND) -> Value {
    let r = frame(h);
    let pid = pid_of(h);
    json!({
        "ref": window_ref(h),
        "process": process_name(pid),
        "pid": pid,
        "title": title(h),
        "x": r.left, "y": r.top, "w": r.right - r.left, "h": r.bottom - r.top,
        "state": state(h),
        "focused": h == fg,
        "elevated": is_elevated(h),
        "agent": opened_by_agent(h),
    })
}

// ---------------------------------------------------------------------------------------------
// Owned dialogs (P6-M4, ADR 0047): a message box is a separate top-level window owned by the
// app window, so list_windows and the app's UIA tree never show it.

/// Chrome, Edge and Electron apps (P6-M5, ADR 0048).
pub fn is_chromium_class(class: &str) -> bool {
    class.starts_with("Chrome_WidgetWin_")
}

pub fn is_chromium(h: HWND) -> bool {
    is_chromium_class(&class_name(h))
}

/// Shows a minimized window without activating it; minimizes it again (without activating)
/// when dropped, also on early returns (observe reveal, ADR 0048).
pub struct Revealed(HWND);

pub fn reveal_no_activate(h: HWND) -> Revealed {
    const SW_SHOWNOACTIVATE: i32 = 4;
    unsafe { ShowWindow(h, SW_SHOWNOACTIVATE) };
    Revealed(h)
}

impl Drop for Revealed {
    fn drop(&mut self) {
        const SW_SHOWMINNOACTIVE: i32 = 7;
        unsafe { ShowWindow(self.0, SW_SHOWMINNOACTIVE) };
    }
}

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn owner(h: HWND) -> HWND {
    unsafe { GetWindow(h, GW_OWNER) }
}

pub fn is_enabled(h: HWND) -> bool {
    unsafe { IsWindowEnabled(h) != 0 }
}

/// Pure rule: an owned window is a dialog when it is a standard dialog (`#32770`) or its owner
/// is disabled (a modal is open). Tooltips and menus are neither.
pub fn counts_as_dialog(class: &str, owner_disabled: bool) -> bool {
    class == "#32770" || owner_disabled
}

/// `h` owns `d`, directly or through a chain of owners (at most 8 steps).
fn owned_by(d: HWND, h: HWND) -> bool {
    let mut o = owner(d);
    for _ in 0..8 {
        if o.is_null() {
            return false;
        }
        if o == h {
            return true;
        }
        o = owner(o);
    }
    false
}

unsafe extern "system" fn collect_visible(h: HWND, lparam: isize) -> BOOL {
    let v = &mut *(lparam as *mut Vec<HWND>);
    if IsWindowVisible(h) != 0 && !cloaked(h) {
        v.push(h);
    }
    1
}

/// The front-most dialog owned by `h` and whether it blocks `h` (owner disabled). XtraOS's own
/// windows never count.
pub fn owned_dialog(h: HWND) -> Option<(HWND, bool)> {
    let mut v: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(collect_visible, (&mut v as *mut Vec<HWND>) as isize) };
    let blocked = !is_enabled(h);
    v.into_iter()
        .find(|&d| {
            d != h
                && owned_by(d, h)
                && !crate::safety::is_own_window(d)
                && counts_as_dialog(&class_name(d), !is_enabled(owner(d)))
        })
        .map(|d| (d, blocked))
}

/// `h` is itself an owned dialog: its owner's ref.
pub fn dialog_owner(h: HWND) -> Option<HWND> {
    let o = owner(h);
    (!o.is_null() && counts_as_dialog(&class_name(h), !is_enabled(o))).then_some(o)
}

/// `status.dialog`: the foreground window's dialog, or the foreground window when it is one.
pub fn foreground_dialog() -> Value {
    let fg = foreground_root();
    if fg.is_null() || crate::safety::is_own_window(fg) {
        return Value::Null;
    }
    let (d, o, blocking) = if let Some(o) = dialog_owner(fg) {
        (fg, o, !is_enabled(o))
    } else if let Some((d, b)) = owned_dialog(fg) {
        (d, fg, b)
    } else {
        return Value::Null;
    };
    json!({
        "window": window_ref(d),
        "title": title(d),
        "process": process_name(pid_of(d)),
        "owner": window_ref(o),
        "blocking": blocking,
    })
}

pub fn foreground_root() -> HWND {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_null() {
        fg
    } else {
        unsafe { GetAncestor(fg, GA_ROOT) }
    }
}

// ---------------------------------------------------------------------------------------------
// Actions

pub const METHODS: &[&str] = &[
    "cleanup",
    "list_windows",
    "focus_window",
    "launch_app",
    "close_window",
    "window_bounds",
];

pub fn run(method: &str, p: &Value) -> Result<Value, ActError> {
    crate::input::init_dpi();
    crate::input::next_action();
    crate::safety::check()?;
    match method {
        "list_windows" => list_windows(p),
        "cleanup" => cleanup(None),
        "cleanup_one" => cleanup(Some(window_param(p)?)),
        "focus_window" => focus_window(p),
        "launch_app" => launch_app(p),
        "close_window" => close_window(p),
        "window_bounds" => window_bounds(p),
        m => Err(bad(format!("'{m}' is not a window method"))),
    }
}

fn window_param(p: &Value) -> Result<HWND, ActError> {
    resolve_window(p.get("window").ok_or_else(|| bad("'window' is required"))?)
}

fn opt_u64(p: &Value, k: &str, default: u64, max: u64) -> Result<u64, ActError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|&n| n <= max)
            .ok_or_else(|| bad(format!("'{k}' must be 0..={max}"))),
    }
}

fn list_windows(p: &Value) -> Result<Value, ActError> {
    let filter = p
        .get("filter")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let fg = foreground_root();
    let windows: Vec<Value> = listable_windows()
        .into_iter()
        .map(|h| describe(h, fg))
        .filter(|w| match &filter {
            Some(f) => {
                w["title"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(f.as_str())
                    || w["process"].as_str().unwrap_or("").contains(f.as_str())
            }
            None => true,
        })
        .collect();
    Ok(json!({ "windows": windows }))
}

/// Brings a window to the foreground within Windows' focus rules (S9 / ADR 0006).
pub fn focus(h: HWND) -> bool {
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
            sleep(Duration::from_millis(150));
        }
        if foreground_root() == h {
            return true;
        }
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let me = GetCurrentThreadId();
        let attached =
            fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
        BringWindowToTop(h);
        SetForegroundWindow(h);
        if attached {
            AttachThreadInput(me, fg_thread, 0);
        }
    }
    for _ in 0..20 {
        if foreground_root() == h {
            return true;
        }
        sleep(Duration::from_millis(25));
    }
    false
}

fn focus_window(p: &Value) -> Result<Value, ActError> {
    let h = window_param(p)?;
    let verify_ms = opt_u64(p, "verify_ms", 300, 2000)?;
    if !focus(h) {
        return Err(err(
            "NEEDS_HUMAN",
            format!(
                "Windows did not allow {} to come to the front",
                window_ref(h)
            ),
            "Ask the user to click the window once, then retry.",
        ));
    }
    // P6-M2 (ADR 0046): look again a moment later; another window may have taken the front.
    if verify_ms > 0 {
        sleep(Duration::from_millis(verify_ms));
    }
    let fg = foreground_root();
    if fg == h {
        return Ok(json!({"ref": window_ref(h), "focused": true}));
    }
    let taken_by = if fg.is_null() {
        Value::Null
    } else {
        json!({"ref": window_ref(fg), "process": process_name(pid_of(fg)), "title": title(fg)})
    };
    Ok(json!({"ref": window_ref(h), "focused": false, "taken_by": taken_by}))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `chrome` → `chrome.exe`; AUMIDs (contain `!`) → `shell:AppsFolder\…`; paths unchanged.
pub fn normalise_app(app: &str) -> String {
    let a = app.trim();
    if a.contains('!') && !a.contains('\\') {
        return format!("shell:AppsFolder\\{a}");
    }
    let has_path = a.contains('\\') || a.contains('/') || a.contains(':');
    let has_ext = std::path::Path::new(a).extension().is_some();
    if !has_path && !has_ext {
        format!("{a}.exe")
    } else {
        a.to_string()
    }
}

fn launch_app(p: &Value) -> Result<Value, ActError> {
    let app = p
        .get("app")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad("'app' is required"))?;
    let args: String = match p.get("args") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(quote_arg)
                    .ok_or_else(|| bad("'args' items must be strings"))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(" "),
        _ => return Err(bad("'args' must be a string or an array of strings")),
    };
    // A service from the app map (ADR 0026) launches what the map says; explicit args win.
    let (app, args) = match crate::appmap::resolve_launch(app) {
        Some((a, map_args)) if args.is_empty() => (
            a,
            map_args
                .iter()
                .map(|x| quote_arg(x))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Some((a, _)) => (a, args),
        None => (app.to_string(), args),
    };
    let app = app.as_str();
    let elevated = p.get("elevated").and_then(Value::as_bool).unwrap_or(false);
    let timeout = opt_u64(p, "timeout_ms", 15_000, 60_000)?;
    // Reuse instead of opening the same thing twice (ADR 0028).
    let key = format!("{}|{}|{}", app.to_lowercase(), args, elevated);
    if p.get("reuse").and_then(Value::as_bool).unwrap_or(true) {
        if let Some(h) = reusable(&key) {
            focus(h);
            let mut d = describe(h, foreground_root());
            d["reused"] = json!(true);
            return Ok(d);
        }
    }

    let before: HashSet<usize> = listable_windows().into_iter().map(|h| h as usize).collect();
    let file = wide(&normalise_app(app));
    let params = wide(&args);
    let verb = wide(if elevated { "runas" } else { "open" });
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        hwnd: std::ptr::null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: if args.is_empty() {
            std::ptr::null()
        } else {
            params.as_ptr()
        },
        lpDirectory: std::ptr::null(),
        nShow: SW_SHOWNORMAL,
        hInstApp: std::ptr::null_mut(),
        lpIDList: std::ptr::null_mut(),
        lpClass: std::ptr::null(),
        hkeyClass: std::ptr::null_mut(),
        dwHotKey: 0,
        hIconOrMonitor: std::ptr::null_mut(),
        hProcess: std::ptr::null_mut(),
    };
    // Blocks while a UAC prompt is shown (secure desktop: only the human can answer it).
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let code = unsafe { GetLastError() } as i32;
        return Err(match code {
            ERROR_CANCELLED => err(
                "NEEDS_HUMAN",
                "the user declined the administrator (UAC) prompt",
                "Ask the user before retrying.",
            ),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_NO_ASSOCIATION => {
                bad(format!("app '{app}' was not found"))
            }
            _ => err(
                "INTERNAL",
                format!("launch failed (Win32 error {code})"),
                "",
            ),
        });
    }
    let pid = if info.hProcess.is_null() {
        0
    } else {
        unsafe { GetProcessId(info.hProcess) }
    };
    if !info.hProcess.is_null() {
        unsafe { CloseHandle(info.hProcess) };
    }

    let deadline = Instant::now() + Duration::from_millis(timeout);
    loop {
        let new: Vec<HWND> = listable_windows()
            .into_iter()
            .filter(|h| !before.contains(&(*h as usize)))
            .collect();
        if let Some(&h) = new
            .iter()
            .find(|&&h| pid != 0 && pid_of(h) == pid)
            .or_else(|| new.first())
        {
            mark_opened(h, &key);
            let mut d = describe(h, foreground_root());
            d["launched_pid"] = json!(pid);
            return Ok(d);
        }
        if Instant::now() >= deadline {
            return Err(err(
                "TIMEOUT",
                format!("'{app}' started but no new window appeared within {timeout} ms"),
                "It may have reused an existing window (e.g. a browser tab). Call list_windows.",
            ));
        }
        sleep(Duration::from_millis(100));
    }
}

/// Quotes one command-line argument per the MSVCRT rules (spaces, quotes, trailing backslashes).
pub fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

fn close_window(p: &Value) -> Result<Value, ActError> {
    let h = window_param(p)?;
    let timeout = opt_u64(p, "timeout_ms", 5_000, 60_000)?;
    if is_elevated(h) {
        return Err(elevation_required("close_window"));
    }
    if unsafe { PostMessageW(h, WM_CLOSE, 0, 0) } == 0 {
        return Err(err("INTERNAL", "could not send WM_CLOSE", ""));
    }
    let deadline = Instant::now() + Duration::from_millis(timeout);
    while Instant::now() < deadline {
        if unsafe { IsWindow(h) } == 0 || unsafe { IsWindowVisible(h) } == 0 {
            forget(h);
            return Ok(json!({"ref": window_ref(h), "closed": true}));
        }
        sleep(Duration::from_millis(50));
    }
    Err(err(
        "NEEDS_HUMAN",
        format!("{} is still open after {timeout} ms", window_ref(h)),
        "The app may be asking to save. Ask the user; XtraOS never force-closes apps.",
    ))
}

/// Closes every still-open window the agent opened — only those (ADR 0028). A browser's
/// "Leave page" prompt is confirmed only inside those windows.
fn cleanup(only: Option<HWND>) -> Result<Value, ActError> {
    let mut closed = Vec::new();
    let mut left = Vec::new();
    for h in agent_open_windows()
        .into_iter()
        .filter(|&h| only.is_none_or(|o| o == h))
    {
        let r = window_ref(h);
        let title = title(h);
        if is_elevated(h) {
            left.push(format!(
                "{r} \"{title}\" (administrator window: close it yourself)"
            ));
            continue;
        }
        let mut done = close_window(&json!({"window": r, "timeout_ms": 3000})).is_ok();
        // A browser may ask "Leave page?" (late, e.g. while a sent message still syncs). Look for
        // it for a few seconds; the browser exposes the page only while the window is in front,
        // and this window is the agent's own.
        let t0 = Instant::now();
        while !done && t0.elapsed() < Duration::from_secs(8) {
            if !exists(h) || unsafe { IsWindowVisible(h) } == 0 {
                done = true;
                forget(h);
                break;
            }
            if foreground_root() != h {
                focus(h);
                sleep(Duration::from_millis(300));
            }
            let btn = crate::perceive::run("find", &json!({"window": r, "query": "leave page"}))
                .ok()
                .and_then(|f| {
                    f["text"]
                        .as_str()?
                        .lines()
                        .find(|l| l.contains("Button \"Leave page\""))
                        .and_then(|l| l.split(' ').next())
                        .map(String::from)
                });
            if let Some(b) = btn {
                let _ = crate::input::run("click", &json!({"ref": b}));
                for _ in 0..60 {
                    if !exists(h) || unsafe { IsWindowVisible(h) } == 0 {
                        done = true;
                        forget(h);
                        break;
                    }
                    sleep(Duration::from_millis(50));
                }
            } else {
                sleep(Duration::from_millis(500));
            }
        }
        if done {
            closed.push(format!("{r} \"{title}\""));
        } else {
            left.push(format!(
                "{r} \"{title}\" (asks something: left for the user)"
            ));
        }
    }
    let text = format!(
        "closed {}: {}\nleft open {}: {}",
        closed.len(),
        if closed.is_empty() {
            "-".to_string()
        } else {
            closed.join(", ")
        },
        left.len(),
        if left.is_empty() {
            "-".to_string()
        } else {
            left.join(", ")
        }
    );
    Ok(json!({"text": text, "closed": closed.len(), "left": left.len()}))
}

fn window_bounds(p: &Value) -> Result<Value, ActError> {
    let h = window_param(p)?;
    let get = |k: &str| -> Result<Option<i32>, ActError> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_i64()
                .map(|n| Some(n as i32))
                .ok_or_else(|| bad(format!("'{k}' must be an integer"))),
        }
    };
    let (x, y, w, hgt) = (get("x")?, get("y")?, get("w")?, get("h")?);
    let st = p.get("state").and_then(Value::as_str);
    if let Some(s) = st {
        if !["minimize", "maximize", "restore"].contains(&s) {
            return Err(bad("'state' must be minimize, maximize or restore"));
        }
    }
    if matches!(w, Some(n) if n < 50) || matches!(hgt, Some(n) if n < 50) {
        return Err(bad("'w' and 'h' must be at least 50"));
    }
    let changing = st.is_some() || x.is_some() || y.is_some() || w.is_some() || hgt.is_some();
    if changing && is_elevated(h) {
        return Err(elevation_required("window_bounds"));
    }
    if let Some(s) = st {
        let cmd = match s {
            "minimize" => SW_MINIMIZE,
            "maximize" => SW_MAXIMIZE,
            _ => SW_RESTORE,
        };
        unsafe { ShowWindow(h, cmd) };
        sleep(Duration::from_millis(250));
    }
    if x.is_some() || y.is_some() || w.is_some() || hgt.is_some() {
        if state(h) != "normal" {
            unsafe { ShowWindow(h, SW_RESTORE) };
            sleep(Duration::from_millis(250));
        }
        // Convert visible-frame coordinates to window-rect coordinates (invisible borders).
        let mut wr = RECT::default();
        unsafe { GetWindowRect(h, &mut wr) };
        let fr = frame(h);
        let (bl, bt, br, bb) = (
            fr.left - wr.left,
            fr.top - wr.top,
            wr.right - fr.right,
            wr.bottom - fr.bottom,
        );
        let nx = x.unwrap_or(fr.left) - bl;
        let ny = y.unwrap_or(fr.top) - bt;
        let nw = w.unwrap_or(fr.right - fr.left) + bl + br;
        let nh = hgt.unwrap_or(fr.bottom - fr.top) + bt + bb;
        let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
        if x.is_none() && y.is_none() {
            flags |= SWP_NOMOVE;
        }
        if w.is_none() && hgt.is_none() {
            flags |= SWP_NOSIZE;
        }
        if unsafe { SetWindowPos(h, std::ptr::null_mut(), nx, ny, nw, nh, flags) } == 0 {
            let code = unsafe { GetLastError() };
            return Err(err(
                "INTERNAL",
                format!("SetWindowPos failed (Win32 error {code})"),
                "",
            ));
        }
        sleep(Duration::from_millis(120));
    }
    let r = frame(h);
    Ok(
        json!({"ref": window_ref(h), "x": r.left, "y": r.top, "w": r.right - r.left, "h": r.bottom - r.top, "state": state(h)}),
    )
}

/// Whether the core itself runs elevated (reported by status; tests use it).
pub fn core_is_elevated() -> bool {
    token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shellexecuteinfo_layout_matches_win32() {
        assert_eq!(std::mem::size_of::<SHELLEXECUTEINFOW>(), 112);
    }

    #[test]
    fn refs_parse() {
        assert_eq!(parse_ref(&json!("w460650")), Some(460650));
        assert_eq!(parse_ref(&json!(460650)), Some(460650));
        assert_eq!(parse_ref(&json!("460650")), None);
        assert_eq!(parse_ref(&json!("w")), None);
        assert_eq!(parse_ref(&json!("w0")), None);
        assert_eq!(parse_ref(&json!(true)), None);
        assert_eq!(window_ref(460650usize as HWND), "w460650");
    }

    // P6-M1a (ADR 0046): `window` by text.
    #[test]
    fn windows_match_by_title_or_process() {
        let c = |t: &str, p: &str| (t.to_string(), p.to_string());
        let wins = vec![
            c("Inbox - Gmail - Google Chrome", "chrome.exe"),
            c("Untitled - Notepad", "notepad.exe"),
            c("Notepad", "notepad.exe"),
            c("Downloads - File Explorer", "explorer.exe"),
        ];
        assert_eq!(
            match_windows("gmail", &wins),
            vec![0],
            "part of a title, any case"
        );
        assert_eq!(
            match_windows("Notepad", &wins),
            vec![2],
            "an exact title wins"
        );
        assert_eq!(match_windows("untitled", &wins), vec![1]);
        assert_eq!(
            match_windows("notepad.exe", &wins),
            vec![1, 2],
            "process name"
        );
        assert_eq!(
            match_windows("explorer", &wins),
            vec![3],
            "process without .exe (and the title)"
        );
        assert_eq!(match_windows("chrome", &wins), vec![0]);
        assert!(match_windows("firefox", &wins).is_empty());
        assert!(
            match_windows("   ", &wins).is_empty(),
            "blank text matches nothing"
        );
    }

    // P6-M5 (ADR 0048): which windows get the Chromium page wait.
    #[test]
    fn chromium_windows_by_class() {
        assert!(is_chromium_class("Chrome_WidgetWin_1"));
        assert!(is_chromium_class("Chrome_WidgetWin_0"));
        assert!(!is_chromium_class("Notepad"));
        assert!(!is_chromium_class("MozillaWindowClass"));
    }

    // P6-M4 (ADR 0047): which owned windows are dialogs.
    #[test]
    fn owned_dialog_rule() {
        assert!(counts_as_dialog("#32770", false)); // message box, find dialog
        assert!(counts_as_dialog("SomeAppDialog", true)); // custom modal: owner disabled
        assert!(!counts_as_dialog("tooltips_class32", false));
        assert!(!counts_as_dialog("#32768", false)); // menu
    }

    // P6-M1b (ADR 0046): the session failure: a handle reused by another app.
    #[test]
    fn a_reused_handle_is_detected() {
        let chrome = (4321u32, 1000u64);
        assert!(
            same_instance(None, chrome),
            "a handle never handed out is accepted"
        );
        assert!(same_instance(Some(chrome), chrome));
        assert!(
            !same_instance(Some(chrome), (9876, 2000)),
            "another process"
        );
        assert!(
            !same_instance(Some(chrome), (4321, 5000)),
            "same pid, a later process"
        );
    }

    #[test]
    fn window_text_errors_are_clear() {
        let e = resolve_window(&json!("no-such-window-title-xyz")).unwrap_err();
        assert_eq!(e.code, "WINDOW_NOT_FOUND");
        assert!(e.message.contains("no-such-window-title-xyz"));
        assert_eq!(
            resolve_window(&json!("  ")).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            resolve_window(&json!(true)).unwrap_err().code,
            "INVALID_ARGS"
        );
    }

    #[test]
    fn app_names_normalise() {
        assert_eq!(normalise_app("chrome"), "chrome.exe");
        assert_eq!(normalise_app("notepad.exe"), "notepad.exe");
        assert_eq!(normalise_app(r"C:\Tools\a b\x.exe"), r"C:\Tools\a b\x.exe");
        assert_eq!(
            normalise_app("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"),
            r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        );
        assert_eq!(normalise_app("https://example.com"), "https://example.com");
    }

    #[test]
    fn args_are_quoted_like_msvcrt() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg("has space"), "\"has space\"");
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(r"C:\dir\"), r"C:\dir\");
        assert_eq!(quote_arg(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn listing_works_and_bad_args_are_rejected() {
        let r = run("list_windows", &json!({})).unwrap();
        assert!(r["windows"].is_array());
        assert_eq!(
            run("focus_window", &json!({})).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("focus_window", &json!({"window": "w1"}))
                .unwrap_err()
                .code,
            "WINDOW_NOT_FOUND"
        );
        assert_eq!(
            run("launch_app", &json!({"app": ""})).unwrap_err().code,
            "INVALID_ARGS"
        );
        // P6-M1a (ADR 0046): text is a title / process search now, so an unknown name is
        // "not found"; only a wrong type is a bad argument.
        assert_eq!(
            run("window_bounds", &json!({"window": "nope-no-such-window"}))
                .unwrap_err()
                .code,
            "WINDOW_NOT_FOUND"
        );
        assert_eq!(
            run("window_bounds", &json!({"window": true}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
    }

    #[test]
    fn own_process_is_not_elevated_relative_to_itself() {
        assert!(!core_is_elevated() || own_integrity() >= 0x3000);
    }
}
