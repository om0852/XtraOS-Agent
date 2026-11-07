//! Safety (Phase 1 M5, ADR 0017): kill switch, policy gate, human-in-the-loop dialogs, overlay.
//!
//! One UI thread inside the core owns: the global hotkey (Ctrl+Alt+Esc), the tray icon (Stop /
//! Resume), the click-through overlay banner and the dialogs. Approvals never travel over the
//! pipe, so no client can fake a "yes". No safety UI → no OS actions (hard rule).
//!
//! Docs:
//! RegisterHotKey      https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey
//! Shell_NotifyIconW   https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw
//! Layered windows     https://learn.microsoft.com/windows/win32/winmsg/window-features#layered-windows
//! SetWindowDisplayAffinity https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, HWND as WHWND, LPARAM, LRESULT, POINT as WPOINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
    DT_CENTER, DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, HFONT,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SetFocus, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_ESCAPE,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::input::{self, ActError, HWND, POINT};
use crate::policy::{self, Decision, Facts};
use crate::{perceive, winmgr};

// ---------------------------------------------------------------------------------------------
// State

static STOPPED: AtomicBool = AtomicBool::new(false);
static UI: OnceLock<Ui> = OnceLock::new();
static OVERLAY_TEXT: Mutex<String> = Mutex::new(String::new());
static PROMPTS: Mutex<Option<HashMap<u64, Prompt>>> = Mutex::new(None);
static PROMPT_CV: Condvar = Condvar::new();
static NEXT_PROMPT: AtomicU64 = AtomicU64::new(1);
static POLICY: Mutex<Option<policy::Store>> = Mutex::new(None);
static RATE: Mutex<Option<(Instant, policy::RateLimiter)>> = Mutex::new(None);

const WM_APP_ACTIVITY: u32 = WM_APP + 1;
const WM_APP_PROMPT: u32 = WM_APP + 2;
const WM_APP_CLOSE_PROMPT: u32 = WM_APP + 3;
const WM_APP_STATE: u32 = WM_APP + 4;
const WM_TRAY: u32 = WM_APP + 10;
pub const ID_STOP: usize = 1001;
pub const ID_RESUME: usize = 1002;
const HOTKEY_ID: i32 = 1;
const OVERLAY_TIMER: usize = 1;
pub const PROMPT_BUTTON_BASE: i32 = 100;
pub const PROMPT_TEXT_ID: i32 = 99;

struct Ui {
    ok: bool,
    msg: usize,
    overlay: usize,
    hotkey: bool,
}

struct Prompt {
    title: String,
    message: String,
    buttons: Vec<String>,
    cancel: usize,
    hwnd: usize,
    result: Option<usize>,
    /// Policy confirmations are cancelled by the kill switch; agent questions too.
    open: bool,
}

fn err(code: &'static str, message: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: message.into(),
        hint,
    }
}

fn stopped_err() -> ActError {
    err(
        "STOPPED",
        "stopped by the user (kill switch)",
        "Stop entirely and tell the user. Only the user can resume, from the XtraOS tray icon.",
    )
}

pub fn blocked(why: impl Into<String>) -> ActError {
    err(
        "BLOCKED_BY_POLICY",
        why,
        "Do not look for a way around it. Tell the user what you wanted to do.",
    )
}

/// True after the kill switch until the user resumes.
pub fn stopped() -> bool {
    STOPPED.load(Ordering::SeqCst)
}

/// `Err(STOPPED)` after the kill switch.
pub fn check() -> Result<(), ActError> {
    if stopped() {
        Err(stopped_err())
    } else {
        Ok(())
    }
}

pub fn hotkey_name() -> Option<&'static str> {
    UI.get().filter(|u| u.hotkey).map(|_| "Ctrl+Alt+Esc")
}

/// The hidden UI window (tray commands arrive here). Tests post ID_RESUME to it like the menu.
pub fn ui_window() -> Option<usize> {
    UI.get().filter(|u| u.ok).map(|u| u.msg)
}

pub fn overlay_window() -> Option<usize> {
    UI.get().filter(|u| u.ok).map(|u| u.overlay)
}

fn post(msg: u32, w: usize) {
    if let Some(u) = UI.get().filter(|u| u.ok) {
        unsafe {
            let _ = PostMessageW(Some(WHWND(u.msg as _)), msg, WPARAM(w), LPARAM(0));
        }
    }
}

pub fn stop() {
    if !STOPPED.swap(true, Ordering::SeqCst) {
        eprintln!("[xtraos-core] kill switch: stopped");
    }
    // Open dialogs are answered "no" (policy confirmations and agent questions alike).
    let mut g = PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = g.as_mut() {
        for (id, p) in map.iter_mut() {
            if p.open {
                p.result.get_or_insert(p.cancel);
                p.open = false;
                post(WM_APP_CLOSE_PROMPT, *id as usize);
            }
        }
    }
    PROMPT_CV.notify_all();
    post(WM_APP_STATE, 0);
}

fn resume() {
    if STOPPED.swap(false, Ordering::SeqCst) {
        eprintln!("[xtraos-core] resumed by the user");
    }
    post(WM_APP_STATE, 0);
}

pub fn is_own_window(h: HWND) -> bool {
    !h.is_null() && winmgr::pid_of(h) == std::process::id()
}

// ---------------------------------------------------------------------------------------------
// Policy store

pub fn set_policy_path(path: PathBuf) {
    *POLICY.lock().unwrap_or_else(|e| e.into_inner()) = Some(policy::Store::new(path));
}

pub fn policy_path() -> String {
    with_policy(|s| s.path().display().to_string()).0
}

/// Runs `f` on the current policy; returns its result and a reload note (for the action log).
fn with_policy<T>(f: impl FnOnce(&policy::Store) -> T) -> (T, Option<String>) {
    let mut g = POLICY.lock().unwrap_or_else(|e| e.into_inner());
    let store =
        g.get_or_insert_with(|| policy::Store::new(crate::pairing::data_dir().join("policy.json")));
    let reloaded = store.refresh();
    if let Some(r) = &reloaded {
        for w in &store.policy.warnings {
            eprintln!("[xtraos-core] policy: {w}");
        }
        eprintln!("[xtraos-core] {r}");
    }
    (f(store), reloaded)
}

// ---------------------------------------------------------------------------------------------
// Authorisation: every OS action passes here first (ipc.rs).

pub struct Approval {
    pub decision: &'static str,
    pub policy_note: Option<String>,
}

const HITL_METHODS: &[&str] = &["ask_human", "wait_for_human"];

fn window_facts(h: HWND) -> (String, String) {
    (
        winmgr::process_name(winmgr::pid_of(h)).to_lowercase(),
        winmgr::title(h).to_lowercase(),
    )
}

fn point_arg(p: &Value, kx: &str, ky: &str) -> Option<POINT> {
    Some(POINT {
        x: p.get(kx)?.as_i64()? as i32,
        y: p.get(ky)?.as_i64()? as i32,
    })
}

/// Facts about the action, gathered best-effort (bad arguments are reported by the action).
fn gather(method: &str, p: &Value) -> (Facts, String) {
    let mut f = Facts {
        method: method.to_string(),
        ..Default::default()
    };
    let mut hwnds: Vec<HWND> = Vec::new();
    let mut what = method.replace('_', " ");
    for k in ["window", "to_window"] {
        if let Some(h) = p.get(k).and_then(|v| winmgr::resolve_window(v).ok()) {
            hwnds.push(h);
        }
    }
    let targets_window = matches!(method, "observe" | "find" | "read_text" | "screenshot");
    if targets_window
        && p.get("window").is_none_or(Value::is_null)
        && p.get("ref").is_none_or(Value::is_null)
    {
        let fg = match point_arg(p.get("region").unwrap_or(&Value::Null), "x", "y") {
            Some(pt) => winmgr::root_at(POINT {
                x: pt.x + p["region"]["w"].as_i64().unwrap_or(0) as i32 / 2,
                y: pt.y + p["region"]["h"].as_i64().unwrap_or(0) as i32 / 2,
            }),
            None => winmgr::foreground_root(),
        };
        if !fg.is_null() {
            hwnds.push(fg);
        }
    }
    for k in ["ref", "from_ref", "to_ref"] {
        if let Some((h, name, pw)) = p.get(k).and_then(perceive::ref_facts) {
            hwnds.push(h);
            if k == "ref" {
                what = format!("{what} \"{}\"", perceive::clip(&name, 40));
                f.element = Some(name);
                f.password |= pw;
            }
            if k == "to_ref" {
                f.drop_process = Some(window_facts(h));
            }
            if k == "from_ref" {
                f.drag_from_explorer =
                    winmgr::process_name(winmgr::pid_of(h)).eq_ignore_ascii_case("explorer.exe");
            }
        }
    }
    // Points: the window under them; for a real click also the element's name.
    for (kx, ky) in [("x", "y"), ("from_x", "from_y"), ("to_x", "to_y")] {
        if let Some(pt) = point_arg(p, kx, ky) {
            let h = winmgr::root_at(pt);
            if h.is_null() {
                continue;
            }
            hwnds.push(h);
            if kx == "x" && method == "click" && f.element.is_none() {
                if let Some((name, pw)) = perceive::element_at(pt) {
                    if !name.is_empty() {
                        what = format!("{what} \"{}\"", perceive::clip(&name, 40));
                    }
                    f.element = Some(name);
                    f.password |= pw;
                }
            }
            if kx == "to_x" {
                f.drop_process = Some(window_facts(h));
            }
            if kx == "from_x" {
                f.drag_from_explorer =
                    winmgr::process_name(winmgr::pid_of(h)).eq_ignore_ascii_case("explorer.exe");
            }
        }
    }
    match method {
        "type" => {
            let n = p
                .get("text")
                .and_then(Value::as_str)
                .map(|t| t.chars().count())
                .unwrap_or(0);
            let submit = p.get("submit").and_then(Value::as_bool).unwrap_or(false);
            f.enter = submit
                || p.get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.contains('\n'));
            what = format!(
                "type {n} characters{}",
                if submit { " + Enter" } else { "" }
            );
            f.password |= perceive::focused_is_password();
        }
        "press_keys" => {
            let keys: Vec<String> = p
                .get("keys")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|k| k.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            for k in &keys {
                if let Ok(c) = input::parse_chord(k) {
                    f.enter |= c.key == 0x0D;
                    f.delete_key |= c.key == 0x2E;
                }
            }
            what = format!("press {}", keys.join(", "));
            f.password |= perceive::focused_is_password();
        }
        "set_value" => {
            let n = p
                .get("value")
                .and_then(Value::as_str)
                .map(|t| t.chars().count())
                .unwrap_or(0);
            what = format!("{what} ({n} characters)");
        }
        "launch_app" => {
            f.elevated = p.get("elevated").and_then(Value::as_bool).unwrap_or(false);
            let app = p.get("app").and_then(Value::as_str).unwrap_or("?");
            what = format!(
                "launch {}{}",
                perceive::clip(app, 60),
                if f.elevated { " as administrator" } else { "" }
            );
        }
        _ => {}
    }
    hwnds.sort_unstable();
    hwnds.dedup();
    f.own_window = hwnds.iter().any(|&h| is_own_window(h));
    if let Some(&h) = hwnds.first() {
        let t = winmgr::title(h);
        if !t.is_empty() && !f.own_window {
            what = format!("{what} in \"{}\"", perceive::clip(&t, 50));
        }
    }
    f.windows = hwnds.iter().map(|&h| window_facts(h)).collect();
    (f, what)
}

/// Stop flag, safety UI, rate limit, policy (+ confirm dialog), overlay. Blocking.
pub fn authorize(method: &str, p: &Value) -> Result<Approval, ActError> {
    check()?;
    if !ensure_started() {
        return Err(err(
            "INTERNAL",
            "safety UI unavailable (kill switch / overlay could not start); no actions run without it",
            "Restart the XtraOS core in the user's desktop session.",
        ));
    }
    if HITL_METHODS.contains(&method) {
        return Ok(Approval {
            decision: "allow",
            policy_note: None,
        });
    }
    let ((decision, reason, what, rate), note) = with_policy(|s| {
        let (f, what) = gather(method, p);
        let (d, why) = s.policy.decide(&f);
        (d, why, what, s.policy.rate_per_minute)
    });
    {
        let mut g = RATE.lock().unwrap_or_else(|e| e.into_inner());
        let (t0, lim) = g.get_or_insert_with(|| (Instant::now(), policy::RateLimiter::default()));
        if !lim.admit(t0.elapsed().as_millis() as u64, rate) {
            return Err(blocked(format!(
                "more than {rate} actions per minute (runaway loop protection)"
            )));
        }
    }
    let decision = match decision {
        Decision::Allow => "allow",
        Decision::Deny => return Err(blocked(format!("{what}: {reason}"))),
        Decision::Confirm => {
            activity(&format!("waiting for your approval: {what}"));
            let answer = prompt(
                "XtraOS Agent — approval needed",
                &format!(
                    "The AI agent wants to:\n\n{what}\n\nWhy you are asked: {reason}.\n\nAllow this once?"
                ),
                vec!["Allow once".into(), "Deny".into()],
                1,
                Duration::from_secs(60),
            )?;
            match answer {
                Some(0) => "confirmed",
                Some(_) => return Err(blocked(format!("{what}: denied by the user"))),
                None => return Err(blocked(format!("{what}: not approved within 60 s"))),
            }
        }
    };
    activity(&what);
    Ok(Approval {
        decision,
        policy_note: note,
    })
}

// ---------------------------------------------------------------------------------------------
// Human in the loop

/// Shows a dialog and waits. Ok(Some(i)) = button i, Ok(None) = timeout (dialog closed).
pub fn prompt(
    title: &str,
    message: &str,
    buttons: Vec<String>,
    cancel: usize,
    timeout: Duration,
) -> Result<Option<usize>, ActError> {
    let id = open_prompt(title, message, buttons, cancel)?;
    let r = wait_prompt(id, timeout);
    if !matches!(r, Ok(Some(_))) {
        close_prompt(id);
    }
    forget_prompt(id);
    r
}

fn open_prompt(
    title: &str,
    message: &str,
    buttons: Vec<String>,
    cancel: usize,
) -> Result<u64, ActError> {
    check()?;
    if !ensure_started() {
        return Err(err("INTERNAL", "safety UI unavailable", ""));
    }
    let id = NEXT_PROMPT.fetch_add(1, Ordering::SeqCst);
    PROMPTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(
            id,
            Prompt {
                title: title.into(),
                message: message.into(),
                buttons,
                cancel,
                hwnd: 0,
                result: None,
                open: true,
            },
        );
    post(WM_APP_PROMPT, id as usize);
    Ok(id)
}

fn wait_prompt(id: u64, timeout: Duration) -> Result<Option<usize>, ActError> {
    let deadline = Instant::now() + timeout;
    let mut g = PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if stopped() {
            return Err(stopped_err());
        }
        match g.as_ref().and_then(|m| m.get(&id)) {
            None => {
                return Err(err(
                    "INVALID_ARGS",
                    "unknown request_id",
                    "Use the id from ask_human.",
                ))
            }
            Some(p) if p.result.is_some() => return Ok(p.result),
            _ => {}
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        g = PROMPT_CV
            .wait_timeout(g, (deadline - now).min(Duration::from_millis(250)))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

fn close_prompt(id: u64) {
    if let Some(p) = PROMPTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.get_mut(&id))
    {
        p.open = false;
    }
    post(WM_APP_CLOSE_PROMPT, id as usize);
}

fn forget_prompt(id: u64) {
    if let Some(m) = PROMPTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&id);
    }
}

/// Open dialog windows (for tests acting as the human).
pub fn prompt_windows() -> Vec<usize> {
    PROMPTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|m| {
            m.values()
                .filter(|p| p.open && p.hwnd != 0)
                .map(|p| p.hwnd)
                .collect()
        })
        .unwrap_or_default()
}

pub const METHODS: &[&str] = &["ask_human", "wait_for_human"];

pub fn run(method: &str, p: &Value) -> Result<Value, ActError> {
    match method {
        "ask_human" => ask_human(p),
        "wait_for_human" => wait_for_human(p),
        m => Err(err(
            "INVALID_ARGS",
            format!("'{m}' is not a safety method"),
            "",
        )),
    }
}

fn bad(m: impl Into<String>) -> ActError {
    err(
        "INVALID_ARGS",
        m,
        "Check the tool schema in docs/04-mcp-tool-contract.md.",
    )
}

fn timeout_arg(p: &Value, default_ms: u64) -> Result<Duration, ActError> {
    match p.get("timeout_ms") {
        None | Some(Value::Null) => Ok(Duration::from_millis(default_ms)),
        Some(v) => v
            .as_u64()
            .filter(|&n| (1_000..=600_000).contains(&n))
            .map(Duration::from_millis)
            .ok_or_else(|| bad("'timeout_ms' must be 1000..=600000")),
    }
}

fn ask_human(p: &Value) -> Result<Value, ActError> {
    let message = p
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| bad("'message' is required"))?;
    if message.chars().count() > 1000 {
        return Err(bad("'message' is limited to 1000 characters"));
    }
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("confirm");
    let options: Vec<String> = p
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|o| o.as_str().map(|s| perceive::clip(s, 40)))
                .collect()
        })
        .unwrap_or_default();
    let (buttons, cancel): (Vec<String>, usize) = match kind {
        "confirm" => (vec!["Yes".into(), "No".into()], 1),
        "info" => (vec!["OK".into()], 0),
        "choice" => {
            if !(2..=5).contains(&options.len()) {
                return Err(bad("'choice' needs 2..5 'options'"));
            }
            let mut b = options.clone();
            b.push("Cancel".into());
            let n = b.len() - 1;
            (b, n)
        }
        "task" => (vec!["Done".into(), "I can't do this".into()], 1),
        k => {
            return Err(bad(format!(
                "unknown kind '{k}' (confirm, choice, info, task)"
            )))
        }
    };
    let title = if kind == "task" {
        "XtraOS Agent — needs your help"
    } else {
        "XtraOS Agent — the AI agent asks"
    };
    let body = format!("The AI agent says:\n\n{message}");
    activity("waiting for you");
    if kind == "task" {
        let id = open_prompt(title, &body, buttons, cancel)?;
        return Ok(json!({"request_id": format!("h{id}"), "status": "open"}));
    }
    let timeout = timeout_arg(p, 120_000)?;
    match prompt(title, &body, buttons.clone(), cancel, timeout)? {
        None => Err(err(
            "TIMEOUT",
            "the user did not answer in time (the question was closed)",
            "Carry on without the answer or ask again later.",
        )),
        Some(i) => {
            let answer = match kind {
                "confirm" => {
                    if i == 0 {
                        "yes".to_string()
                    } else {
                        "no".to_string()
                    }
                }
                "info" => "ok".to_string(),
                _ => {
                    if i == cancel {
                        "cancel".to_string()
                    } else {
                        buttons[i].clone()
                    }
                }
            };
            Ok(json!({"answer": answer}))
        }
    }
}

fn wait_for_human(p: &Value) -> Result<Value, ActError> {
    let id = p
        .get("request_id")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix('h'))
        .and_then(|d| d.parse::<u64>().ok())
        .ok_or_else(|| bad("'request_id' (from ask_human kind task) is required"))?;
    let timeout = timeout_arg(p, 120_000)?;
    match wait_prompt(id, timeout)? {
        None => Err(err(
            "TIMEOUT",
            "the user has not finished yet (the request stays open)",
            "Call wait_for_human again, or tell the user what you are waiting for.",
        )),
        Some(i) => {
            forget_prompt(id);
            Ok(json!({"status": if i == 0 { "done" } else { "denied" }}))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Overlay

/// Shows "XtraOS Agent · <what>" on the overlay banner (never typed text).
pub fn activity(what: &str) {
    *OVERLAY_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = perceive::clip(what, 90);
    post(WM_APP_ACTIVITY, 0);
}

// ---------------------------------------------------------------------------------------------
// UI thread

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn scale(px: i32) -> i32 {
    let dpi = unsafe { GetDpiForSystem() } as i32;
    px * dpi.max(96) / 96
}

fn font(pt: i32, bold: bool) -> HFONT {
    let dpi = unsafe { GetDpiForSystem() } as i32;
    unsafe {
        CreateFontW(
            -(pt * dpi.max(96) / 72),
            0,
            0,
            0,
            if bold {
                FW_SEMIBOLD.0 as i32
            } else {
                FW_NORMAL.0 as i32
            },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            Default::default(),
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

thread_local! {
    static UI_FONT: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Starts the UI thread once. False = it could not start (then no actions run).
pub fn ensure_started() -> bool {
    UI.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("xtraos-safety-ui".into())
            .spawn(move || ui_thread(tx));
        if spawned.is_err() {
            return Ui {
                ok: false,
                msg: 0,
                overlay: 0,
                hotkey: false,
            };
        }
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or(Ui {
            ok: false,
            msg: 0,
            overlay: 0,
            hotkey: false,
        })
    })
    .ok
}

fn tray_data(msg: WHWND, tip: &str) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: msg,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
        ..Default::default()
    };
    for (i, c) in tip.encode_utf16().take(nid.szTip.len() - 1).enumerate() {
        nid.szTip[i] = c;
    }
    nid
}

fn tray_tip() -> &'static str {
    if stopped() {
        "XtraOS Agent — STOPPED (right-click to resume)"
    } else {
        "XtraOS Agent — running (Ctrl+Alt+Esc stops)"
    }
}

fn ui_thread(tx: std::sync::mpsc::Sender<Ui>) {
    input::init_dpi();
    unsafe {
        let hinst = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
        let reg = |name: PCWSTR, proc_: WNDPROC, bg: bool| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: proc_,
                hInstance: hinst,
                lpszClassName: name,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: if bg {
                    windows::Win32::Graphics::Gdi::HBRUSH(
                        (windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as _,
                    )
                } else {
                    Default::default()
                },
                ..Default::default()
            };
            RegisterClassExW(&wc);
        };
        reg(w!("XtraOSCoreUi"), Some(ui_proc), false);
        reg(w!("XtraOSOverlay"), Some(overlay_proc), false);
        reg(w!("XtraOSPrompt"), Some(prompt_proc), true);

        let fail = |tx: &std::sync::mpsc::Sender<Ui>| {
            let _ = tx.send(Ui {
                ok: false,
                msg: 0,
                overlay: 0,
                hotkey: false,
            });
        };
        let Ok(msg) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("XtraOSCoreUi"),
            w!("XtraOS Agent"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        ) else {
            return fail(&tx);
        };
        let (ow, oh) = (scale(620), scale(34));
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let Ok(overlay) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("XtraOSOverlay"),
            w!("XtraOS Agent overlay"),
            WS_POPUP,
            (sw - ow) / 2,
            scale(6),
            ow,
            oh,
            None,
            None,
            Some(hinst),
            None,
        ) else {
            return fail(&tx);
        };
        let _ = SetLayeredWindowAttributes(overlay, COLORREF(0), 235, LWA_ALPHA);
        // The agent's screenshots must never show (or be confused by) the banner.
        let _ = SetWindowDisplayAffinity(overlay, WINDOW_DISPLAY_AFFINITY(0x11));
        UI_FONT.with(|f| f.set(font(10, false).0 as isize));
        TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(w!("TaskbarCreated"))));
        let hotkey = RegisterHotKey(
            Some(msg),
            HOTKEY_ID,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_ESCAPE.0 as u32,
        )
        .is_ok();
        if !hotkey {
            eprintln!(
                "[xtraos-core] Ctrl+Alt+Esc is taken by another program: use the tray icon to stop"
            );
        }
        let nid = tray_data(msg, tray_tip());
        let tray = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
        if !tray && !hotkey {
            return fail(&tx); // no way for the user to stop us → refuse to run actions
        }
        let _ = tx.send(Ui {
            ok: true,
            msg: msg.0 as usize,
            overlay: overlay.0 as usize,
            hotkey,
        });

        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            let mut handled = false;
            for h in prompt_windows() {
                if IsDialogMessageW(WHWND(h as _), &m).as_bool() {
                    handled = true;
                    break;
                }
            }
            if !handled {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

unsafe extern "system" fn ui_proc(h: WHWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY if wp.0 as i32 == HOTKEY_ID => {
            stop();
            LRESULT(0)
        }
        WM_COMMAND => {
            match wp.0 & 0xFFFF {
                ID_STOP => stop(),
                ID_RESUME => resume(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_TRAY => {
            let ev = (lp.0 & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                tray_menu(h);
            }
            LRESULT(0)
        }
        WM_APP_ACTIVITY | WM_APP_STATE => {
            refresh_overlay(msg == WM_APP_ACTIVITY);
            if msg == WM_APP_STATE {
                let nid = tray_data(h, tray_tip());
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == OVERLAY_TIMER => {
            let _ = KillTimer(Some(h), OVERLAY_TIMER);
            if !stopped() {
                if let Some(o) = overlay_window() {
                    let _ = ShowWindow(WHWND(o as _), SW_HIDE);
                }
            }
            LRESULT(0)
        }
        WM_APP_PROMPT => {
            create_prompt(wp.0 as u64);
            LRESULT(0)
        }
        WM_APP_CLOSE_PROMPT => {
            let hwnd = PROMPTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|m| m.get(&(wp.0 as u64)))
                .map(|p| p.hwnd);
            if let Some(hw) = hwnd.filter(|&x| x != 0) {
                let _ = DestroyWindow(WHWND(hw as _));
            }
            LRESULT(0)
        }
        _ => {
            if msg != 0 && msg == TASKBAR_CREATED.with(|t| t.get()) {
                let nid = tray_data(h, tray_tip()); // Explorer restarted: re-add the icon
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            }
            DefWindowProcW(h, msg, wp, lp)
        }
    }
}

unsafe fn tray_menu(h: WHWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let s = stopped();
    let stop_label = wide("Stop the agent now\tCtrl+Alt+Esc");
    let resume_label = wide("Resume the agent");
    let _ = AppendMenuW(
        menu,
        if s { MF_STRING | MF_GRAYED } else { MF_STRING },
        ID_STOP,
        PCWSTR(stop_label.as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        if s { MF_STRING } else { MF_STRING | MF_GRAYED },
        ID_RESUME,
        PCWSTR(resume_label.as_ptr()),
    );
    let mut pt = WPOINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(h); // required so the menu closes when clicking elsewhere
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        None,
        h,
        None,
    );
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(Some(h), WM_NULL, WPARAM(0), LPARAM(0));
    match cmd.0 as usize {
        ID_STOP => stop(),
        ID_RESUME => resume(),
        _ => {}
    }
}

unsafe fn refresh_overlay(activity: bool) {
    let (Some(o), Some(u)) = (overlay_window(), ui_window()) else {
        return;
    };
    let o = WHWND(o as _);
    let _ = InvalidateRect(Some(o), None, true);
    if stopped() || activity {
        let _ = SetWindowPos(
            o,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        if !stopped() {
            SetTimer(Some(WHWND(u as _)), OVERLAY_TIMER, 2500, None);
        }
    } else {
        let _ = ShowWindow(o, SW_HIDE);
    }
}

unsafe extern "system" fn overlay_proc(h: WHWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            let mut r = RECT::default();
            let _ = GetClientRect(h, &mut r);
            let s = stopped();
            let bg = CreateSolidBrush(if s {
                COLORREF(0x0020_20B0)
            } else {
                COLORREF(0x0030_2820)
            });
            FillRect(dc, &r, bg);
            let _ = DeleteObject(bg.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));
            let f = UI_FONT.with(|f| f.get());
            let old = SelectObject(dc, windows::Win32::Graphics::Gdi::HGDIOBJ(f as _));
            let text = if s {
                "XtraOS Agent STOPPED — resume from the XtraOS tray icon".to_string()
            } else {
                format!(
                    "XtraOS Agent · {} · Ctrl+Alt+Esc stops",
                    OVERLAY_TEXT.lock().unwrap_or_else(|e| e.into_inner())
                )
            };
            let mut t: Vec<u16> = text.encode_utf16().collect();
            r.left += scale(10);
            r.right -= scale(10);
            DrawTextW(
                dc,
                &mut t,
                &mut r,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            SelectObject(dc, old);
            let _ = EndPaint(h, &ps);
            LRESULT(0)
        }
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT: never takes a click
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

unsafe fn create_prompt(id: u64) {
    let (title, message, buttons, cancel) = {
        let g = PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(p) = g.as_ref().and_then(|m| m.get(&id)) else {
            return;
        };
        if !p.open {
            return;
        }
        (
            p.title.clone(),
            p.message.clone(),
            p.buttons.clone(),
            p.cancel,
        )
    };
    let hinst = GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default();
    let (w, pad, bh) = (scale(480), scale(16), scale(30));
    let f = HFONT(UI_FONT.with(|f| f.get()) as _);
    // Measure the message height.
    let dc = windows::Win32::Graphics::Gdi::GetDC(None);
    let old = SelectObject(dc, f.into());
    let mut mr = RECT {
        left: 0,
        top: 0,
        right: w - 2 * pad,
        bottom: 0,
    };
    let mut mt: Vec<u16> = message.encode_utf16().collect();
    DrawTextW(
        dc,
        &mut mt,
        &mut mr,
        windows::Win32::Graphics::Gdi::DT_CALCRECT | windows::Win32::Graphics::Gdi::DT_WORDBREAK,
    );
    SelectObject(dc, old);
    windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
    let text_h = mr.bottom.min(scale(400));
    let client_h = pad + text_h + pad + bh + pad;
    let mut wr = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: client_h,
    };
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_TOPMOST | WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT;
    let _ = AdjustWindowRectEx(&mut wr, style, false, ex);
    let (ww, wh) = (wr.right - wr.left, wr.bottom - wr.top);
    let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let tw = wide(&title);
    let Ok(hw) = CreateWindowExW(
        ex,
        w!("XtraOSPrompt"),
        PCWSTR(tw.as_ptr()),
        style,
        (sw - ww) / 2,
        (sh - wh) / 3,
        ww,
        wh,
        None,
        None,
        Some(hinst),
        None,
    ) else {
        return;
    };
    let mw = wide(&message);
    if let Ok(st) = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("STATIC"),
        PCWSTR(mw.as_ptr()),
        WS_CHILD | WS_VISIBLE,
        pad,
        pad,
        w - 2 * pad,
        text_h,
        Some(hw),
        Some(HMENU(PROMPT_TEXT_ID as _)),
        Some(hinst),
        None,
    ) {
        SendMessageW(st, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
    }
    // Buttons right-aligned; the cancel (safe) button is the default and has the focus.
    let n = buttons.len() as i32;
    let bw = scale(120);
    let gap = scale(8);
    let mut x = w - pad - n * bw - (n - 1) * gap;
    let mut cancel_hwnd = None;
    for (i, b) in buttons.iter().enumerate() {
        let bt = wide(b);
        let style = if i == cancel {
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32)
        } else {
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32)
        };
        if let Ok(bh_) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(bt.as_ptr()),
            style,
            x,
            pad + text_h + pad,
            bw,
            bh,
            Some(hw),
            Some(HMENU((PROMPT_BUTTON_BASE + i as i32) as _)),
            Some(hinst),
            None,
        ) {
            SendMessageW(bh_, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
            if i == cancel {
                cancel_hwnd = Some(bh_);
            }
        }
        x += bw + gap;
    }
    SetWindowLongPtrW(hw, GWLP_USERDATA, id as isize);
    if let Some(p) = PROMPTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.get_mut(&id))
    {
        p.hwnd = hw.0 as usize;
    }
    let _ = ShowWindow(hw, SW_SHOW);
    winmgr::focus(hw.0 as HWND);
    if let Some(c) = cancel_hwnd {
        let _ = SetFocus(Some(c));
    }
    let _ = FlashWindow(hw, true);
}

fn answer(id: u64, i: usize) {
    if let Some(p) = PROMPTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.get_mut(&id))
    {
        p.result.get_or_insert(i);
        p.open = false;
    }
    PROMPT_CV.notify_all();
}

unsafe extern "system" fn prompt_proc(h: WHWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let id = GetWindowLongPtrW(h, GWLP_USERDATA) as u64;
    match msg {
        WM_COMMAND => {
            let ctl = (wp.0 & 0xFFFF) as i32;
            let cancel = PROMPTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|m| m.get(&id))
                .map(|p| (p.cancel, p.buttons.len()));
            if let Some((cancel, n)) = cancel {
                let i = ctl - PROMPT_BUTTON_BASE;
                if (0..n as i32).contains(&i) {
                    answer(id, i as usize);
                    let _ = DestroyWindow(h);
                } else if ctl == IDCANCEL.0 || ctl == IDOK.0 {
                    // Esc, or Enter without a focused button: the safe answer.
                    answer(id, cancel);
                    let _ = DestroyWindow(h);
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let cancel = PROMPTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|m| m.get(&id))
                .map(|p| p.cancel);
            if let Some(c) = cancel {
                answer(id, c);
            }
            let _ = DestroyWindow(h);
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_flag_round_trip() {
        // Pure flag behaviour (no UI needed).
        assert!(check().is_ok() || stopped());
    }

    #[test]
    fn ask_human_validates_arguments() {
        assert_eq!(
            run("ask_human", &json!({})).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run(
                "ask_human",
                &json!({"message": "x", "kind": "choice", "options": ["a"]})
            )
            .unwrap_err()
            .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("ask_human", &json!({"message": "x", "kind": "poll"}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("wait_for_human", &json!({"request_id": "nope"}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
    }
}
