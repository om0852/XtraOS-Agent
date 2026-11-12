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
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, RegisterHotKey, SetFocus, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_ESCAPE,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::input::{self, ActError, HWND, POINT};
use crate::policy::{self, Decision, Facts};
use crate::{perceive, winmgr};

// ---------------------------------------------------------------------------------------------
// State

static STOPPED: AtomicBool = AtomicBool::new(false);
/// Real user activity seen by the low-level hooks (tick; 0 = none yet). Ignores the core's own
/// injected input and mouse jitter under JITTER_PX (a resting hand / noisy sensor blocked the
/// agent forever in the P2-M3 gate run).
static USER_TICK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static HOOKS: AtomicBool = AtomicBool::new(false);
static ANCHOR: Mutex<(i32, i32)> = Mutex::new((i32::MIN, i32::MIN));
pub const JITTER_PX: i32 = 6;
static UI: OnceLock<Ui> = OnceLock::new();
static OVERLAY_TEXT: Mutex<String> = Mutex::new(String::new());
static PROMPTS: Mutex<Option<HashMap<u64, Prompt>>> = Mutex::new(None);
static PROMPT_CV: Condvar = Condvar::new();
static NEXT_PROMPT: AtomicU64 = AtomicU64::new(1);
static POLICY: Mutex<Option<policy::Store>> = Mutex::new(None);
static RATE: Mutex<Option<(Instant, policy::RateLimiter)>> = Mutex::new(None);
static TASK: Mutex<Option<(Instant, policy::TaskLimiter)>> = Mutex::new(None);
/// Policy reload decisions waiting to be written to the action log (ipc drains them after
/// every request — also when the request itself was then blocked).
static POLICY_NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn take_policy_notes() -> Vec<String> {
    std::mem::take(&mut *POLICY_NOTES.lock().unwrap_or_else(|e| e.into_inner()))
}

const WM_APP_ACTIVITY: u32 = WM_APP + 1;
const WM_APP_PROMPT: u32 = WM_APP + 2;
const WM_APP_CLOSE_PROMPT: u32 = WM_APP + 3;
const WM_APP_STATE: u32 = WM_APP + 4;
const WM_APP_ACTIVITY_END: u32 = WM_APP + 5;
const WM_TRAY: u32 = WM_APP + 10;
pub const ID_STOP: usize = 1001;
pub const ID_RESUME: usize = 1002;
pub const ID_QUIT: usize = 1003;
pub const ID_VIEWER: usize = 1004;
/// P8-M3: tray Agents submenu items (only through TrackPopupMenu, never WM_COMMAND).
const ID_AGENT_BASE: usize = 1100;
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
    /// Random id a paired device answers by (ADR 0076).
    uid: String,
    title: String,
    message: String,
    buttons: Vec<String>,
    cancel: usize,
    hwnd: usize,
    result: Option<usize>,
    /// Policy confirmations are cancelled by the kill switch; agent questions too.
    open: bool,
    /// When it was opened: a non-safe answer in the first moment is ignored.
    opened: Instant,
}

/// A prompt that pops up under a moving cursor can catch a click meant for something else
/// (seen 2026-11-10 while the owner used the PC during a test). For this long after opening,
/// only the safe answer counts.
const PROMPT_ARM_MS: u128 = 800;

/// Does a button press count? The safe answer always; another one only from real input and
/// only after the prompt has been open PROMPT_ARM_MS (pure).
pub fn answer_counts(i: usize, cancel: usize, open_ms: u128, hardware: bool) -> bool {
    i == cancel || (hardware && open_ms >= PROMPT_ARM_MS)
}

fn err(code: &'static str, message: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: message.into(),
        hint,
    }
}

pub fn stopped_err() -> ActError {
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
        // P8-M3 (ADR 0061): the user paused only this agent.
        crate::agents::check_current()
    }
}

/// Milliseconds since the user's last real input (keys, buttons, wheel, or moving the mouse
/// more than JITTER_PX), from the hooks. None when the hooks are not running.
pub fn user_idle_ms() -> Option<u32> {
    if !HOOKS.load(Ordering::Relaxed) {
        return None;
    }
    let t = USER_TICK.load(Ordering::Relaxed);
    let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    Some(if t == 0 {
        u32::MAX
    } else {
        now.wrapping_sub(t)
    })
}

fn mark_user() {
    let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    USER_TICK.store(now.max(1), Ordering::Relaxed);
}

// Input census (ADR 0034 item 7): what the hooks see, for finding the source of input
// events during hands-off runs. Counters only; no keys, no positions are kept.
pub(crate) mod census {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    pub static KB_REAL: AtomicU32 = AtomicU32::new(0);
    pub static KB_OURS: AtomicU32 = AtomicU32::new(0);
    pub static KB_FOREIGN_INJECTED: AtomicU32 = AtomicU32::new(0);
    pub static MOVE_JITTER: AtomicU32 = AtomicU32::new(0);
    pub static MOVE_BIG: AtomicU32 = AtomicU32::new(0);
    pub static BUTTONS_WHEEL: AtomicU32 = AtomicU32::new(0);
    pub static MOUSE_OURS: AtomicU32 = AtomicU32::new(0);
    pub static MOUSE_FOREIGN_INJECTED: AtomicU32 = AtomicU32::new(0);
    /// Distinct dwExtraInfo tags of injected input from other programs (e.g. pen/touch
    /// drivers use 0xFF5157xx).
    pub static EXTRA: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    pub fn bump(c: &AtomicU32) {
        c.fetch_add(1, Ordering::Relaxed);
    }
    pub fn tag(extra: usize) {
        if let Ok(mut v) = EXTRA.try_lock() {
            if !v.contains(&extra) && v.len() < 16 {
                v.push(extra);
            }
        }
    }
}

/// Clears the input census counters.
pub fn reset_input_census() {
    use census::*;
    for c in [
        &KB_REAL,
        &KB_OURS,
        &KB_FOREIGN_INJECTED,
        &MOVE_JITTER,
        &MOVE_BIG,
        &BUTTONS_WHEEL,
        &MOUSE_OURS,
        &MOUSE_FOREIGN_INJECTED,
    ] {
        c.store(0, Ordering::Relaxed);
    }
    census::EXTRA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

/// What the low-level hooks saw since the last reset (counts only).
pub fn input_census() -> Value {
    use census::*;
    let g = |c: &std::sync::atomic::AtomicU32| c.load(Ordering::Relaxed);
    json!({
        "hooks": HOOKS.load(Ordering::Relaxed),
        "keyboard": {"real": g(&KB_REAL), "ours": g(&KB_OURS), "injected_by_others": g(&KB_FOREIGN_INJECTED)},
        "mouse": {"jitter_moves": g(&MOVE_JITTER), "big_moves": g(&MOVE_BIG), "buttons_wheel": g(&BUTTONS_WHEEL),
                  "ours": g(&MOUSE_OURS), "injected_by_others": g(&MOUSE_FOREIGN_INJECTED)},
        "injector_tags": EXTRA.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|x| format!("{x:#x}")).collect::<Vec<_>>(),
    })
}

unsafe extern "system" fn kb_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injected = k.flags.0 & LLKHF_INJECTED.0 != 0;
        let ours = injected && input::recently_injected();
        // A real Enter/Space press (not injected by any program) can confirm a focused
        // dialog button (ADR 0041).
        if !injected && matches!(k.vkCode, 0x0D | 0x20) {
            REAL_KEY_TICK.store(unsafe { GetTickCount() }.max(1), Ordering::Relaxed);
        }
        if ours {
            census::bump(&census::KB_OURS);
        } else if injected {
            census::bump(&census::KB_FOREIGN_INJECTED);
            census::tag(k.dwExtraInfo);
        } else {
            census::bump(&census::KB_REAL);
        }
        if !injected || !input::recently_injected() {
            mark_user();
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let m = &*(lp.0 as *const MSLLHOOKSTRUCT);
        let injected = m.flags & LLMHF_INJECTED != 0;
        // A real left click (not injected by any program): where and when, so an approval
        // button can check it was really clicked by the user (ADR 0041).
        if !injected && wp.0 as u32 == WM_LBUTTONUP {
            *REAL_CLICK.lock().unwrap_or_else(|e| e.into_inner()) =
                (unsafe { GetTickCount() }.max(1), m.pt.x, m.pt.y);
        }
        if injected && input::recently_injected() {
            census::bump(&census::MOUSE_OURS);
        } else if injected {
            census::bump(&census::MOUSE_FOREIGN_INJECTED);
            census::tag(m.dwExtraInfo);
        }
        if !injected || !input::recently_injected() {
            let mut a = ANCHOR.lock().unwrap_or_else(|e| e.into_inner());
            if wp.0 as u32 == WM_MOUSEMOVE {
                let far = a.0 == i32::MIN
                    || (m.pt.x - a.0).abs() > JITTER_PX
                    || (m.pt.y - a.1).abs() > JITTER_PX;
                census::bump(if far {
                    &census::MOVE_BIG
                } else {
                    &census::MOVE_JITTER
                });
                if far {
                    if a.0 != i32::MIN {
                        mark_user();
                    }
                    *a = (m.pt.x, m.pt.y);
                }
            } else {
                // buttons, wheel: always the user
                census::bump(&census::BUTTONS_WHEEL);
                *a = (m.pt.x, m.pt.y);
                mark_user();
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
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
    // The admin helper ends with the kill switch (ADR 0030); restarting it needs a new UAC.
    crate::helper::on_kill_switch();
    // Open dialogs are answered "no" (policy confirmations and agent questions alike).
    let mut g = PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = g.as_mut() {
        for p in map.values_mut() {
            if p.open {
                p.result.get_or_insert(p.cancel);
                p.open = false;
                post_close_prompt(p.hwnd);
            }
        }
    }
    PROMPT_CV.notify_all();
    post(WM_APP_STATE, 0);
}

fn resume() {
    // Resuming from the tray also starts a fresh task allowance (ADR 0038).
    if let Some((_, l)) = TASK.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        l.reset();
    }
    if STOPPED.swap(false, Ordering::SeqCst) {
        eprintln!("[xtraos-core] resumed by the user");
    }
    post(WM_APP_STATE, 0);
}

/// Name of the desktop that receives input ("Default" normally; "Winlogon" when locked or a
/// UAC prompt is up). None when it cannot be opened, which also means locked/secure.
/// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-openinputdesktop
pub fn input_desktop_name() -> Option<String> {
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS,
        DESKTOP_READOBJECTS, UOI_NAME,
    };
    unsafe {
        let d = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS).ok()?;
        let mut buf = [0u16; 64];
        let mut need = 0u32;
        let ok = GetUserObjectInformationW(
            windows::Win32::Foundation::HANDLE(d.0),
            UOI_NAME,
            Some(buf.as_mut_ptr().cast()),
            (buf.len() * 2) as u32,
            Some(&mut need),
        )
        .is_ok();
        let _ = CloseDesktop(d);
        ok.then(|| {
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..n])
        })
    }
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

/// The policy in force, without checking the file for changes (never shows a dialog).
pub(crate) fn peek_policy<T>(f: impl FnOnce(&policy::Policy) -> T) -> T {
    let mut g = POLICY.lock().unwrap_or_else(|e| e.into_inner());
    let store =
        g.get_or_insert_with(|| policy::Store::new(crate::pairing::data_dir().join("policy.json")));
    f(&store.policy)
}

/// Runs `f` on the current policy; returns its result and a reload note (for the action log).
pub(crate) fn with_policy<T>(f: impl FnOnce(&policy::Store) -> T) -> (T, Option<String>) {
    let mut g = POLICY.lock().unwrap_or_else(|e| e.into_inner());
    let store =
        g.get_or_insert_with(|| policy::Store::new(crate::pairing::data_dir().join("policy.json")));
    // A changed policy file applies only after the user approves it (ADR 0026): otherwise the
    // agent could loosen its own rules by editing the file.
    let mut note = None;
    if let Some(cand) = store.changed() {
        let diff = policy::summary(&store.policy, &cand);
        let hash = cand
            .source
            .as_ref()
            .map(|s| s.1.clone())
            .unwrap_or_else(|| "defaults".into());
        let shown = if diff.is_empty() {
            "(no rule changes)".to_string()
        } else {
            diff.join("\n")
        };
        let ok = if diff.is_empty() {
            Ok(Some(0)) // nothing loosened or tightened: no need to ask
        } else {
            prompt(
            "XtraOS Agent — your safety rules were changed",
            &format!(
                "The XtraOS policy file was changed (by you or by a program):\n\n{shown}\n\nApply these rules?"
            ),
            vec!["Apply new rules".into(), "Keep current rules".into()],
            1,
            Duration::from_secs(120),
            )
        };
        if matches!(ok, Ok(Some(0))) {
            for w in &cand.warnings {
                eprintln!("[xtraos-core] policy: {w}");
            }
            store.apply(cand);
            note = Some(format!("policy {hash} applied (user approved)"));
        } else {
            note = Some(format!("policy {hash} rejected: current rules kept"));
        }
        eprintln!("[xtraos-core] {}", note.as_deref().unwrap_or(""));
        if let Some(n) = &note {
            POLICY_NOTES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(n.clone());
        }
    }
    (f(store), note)
}

// ---------------------------------------------------------------------------------------------
// Authorisation: every OS action passes here first (ipc.rs).

pub struct Approval {
    pub decision: &'static str,
    pub policy_note: Option<String>,
}

/// Not checked as one action here: human questions, and `batch` (each step is authorised
/// on its own in exec::batch).
const HITL_METHODS: &[&str] = &["ask_human", "wait_for_human", "batch"];

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
    let (f, what, _) = gather_targets(method, p);
    (f, what)
}

/// Policy facts, a one-line description, and the top-level windows the action touches
/// (also used by the admin helper's own approval, ADR 0030).
pub fn gather_targets(method: &str, p: &Value) -> (Facts, String, Vec<HWND>) {
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
    let targets_window = matches!(
        method,
        "observe" | "find" | "read_text" | "screenshot" | "ocr" | "frame_diff"
    );
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
            let pointer_press = method == "click"
                || (method == "mouse" && matches!(p["action"].as_str(), Some("down" | "up")))
                || (method == "touch"
                    && matches!(p["action"].as_str().unwrap_or("tap"), "tap" | "long_press"))
                || (method == "pen" && p["action"].as_str().unwrap_or("tap") == "tap");
            if kx == "x" && pointer_press && f.element.is_none() {
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
        "touch" | "pen" => {
            let first = p["points"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_array)
                .and_then(|a| {
                    Some(POINT {
                        x: a.first()?.as_i64()? as i32,
                        y: a.get(1)?.as_i64()? as i32,
                    })
                });
            if let Some(pt) = first {
                let h = winmgr::root_at(pt);
                if !h.is_null() {
                    hwnds.push(h);
                }
            }
            let fingers = p["fingers"].as_u64().unwrap_or(1);
            if method == "touch" && p["action"] == "swipe" && fingers >= 3 {
                // 3-4 finger swipes are Windows gestures (Task View, switch desktops).
                f.skill_class = Some(policy::Class::SystemSettings);
            }
            what = format!("{method} {}", p["action"].as_str().unwrap_or("tap"));
        }
        "system" => {
            let op = p["op"].as_str().unwrap_or("");
            match crate::sysctl::change_kind(p) {
                None => f.method = "system:read".into(),
                Some("system_settings") => {
                    f.method = format!("system:{op}");
                    f.skill_class = Some(policy::Class::SystemSettings);
                }
                Some(_) => f.method = format!("system:{op}"),
            }
            what = match op {
                "brightness" if p.get("value").is_some() => {
                    format!("set the screen brightness to {}", p["value"])
                }
                "wifi" | "bluetooth" if p.get("state").is_some() => {
                    format!("turn {op} {}", p["state"].as_str().unwrap_or("?"))
                }
                "lock" => "lock the PC".into(),
                "sleep" => "put the PC to sleep".into(),
                o => format!("read {o}"),
            };
        }
        "sandbox" => {
            let op = p["op"].as_str().unwrap_or("status");
            f.method = format!("sandbox:{}", crate::sandbox::op_class(p));
            what = match op {
                "call" => format!("in the sandbox: {}", p["tool"].as_str().unwrap_or("?")),
                o => format!("sandbox {o}"),
            };
        }
        "volume" => {
            let op = p["op"].as_str().unwrap_or("get");
            f.method = format!("volume:{op}");
            what = match op {
                "set" => format!("set the system volume to {}", p["value"]),
                "change" => format!("change the system volume by {}", p["by"]),
                o => format!("system volume {o}"),
            };
        }
        "elevated_helper" => {
            f.elevated = p.get("op").and_then(Value::as_str) == Some("start");
            what =
                "start the XtraOS admin helper (Windows will show an administrator prompt)".into();
        }
        "launch_app" => {
            f.elevated = p.get("elevated").and_then(Value::as_bool).unwrap_or(false);
            let app = p.get("app").and_then(Value::as_str).unwrap_or("?");
            let args = match p.get("args") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            what = format!(
                "launch {}{}{}",
                perceive::clip(app, 60),
                if args.is_empty() {
                    String::new()
                } else {
                    format!(" with arguments: {}", perceive::clip(&args, 200))
                },
                if f.elevated { " as administrator" } else { "" }
            );
        }
        _ => {}
    }
    if method == "close_window" {
        if let Some(h) = p.get("window").and_then(|v| winmgr::resolve_window(v).ok()) {
            f.closes_user_window = !winmgr::opened_by_agent(h);
        }
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
    if method == "browser" {
        crate::browser::add_facts(p, &mut f, &mut what);
    }
    (f, what, hwnds)
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
    // P8-M2 (ADR 0060): what this agent client may do at all.
    crate::clients::enforce(method, p)?;
    // ADR 0073: a team policy that is present but does not verify blocks every acting call.
    if let crate::team::State::Broken(why) = crate::team::current() {
        if !crate::clients::is_read(method, p) {
            return Err(blocked(format!(
                "your organisation's team policy could not be verified ({why}); only reading is allowed until an administrator fixes it"
            )));
        }
    }
    if HITL_METHODS.contains(&method) || is_approval_method(method) {
        return Ok(Approval {
            decision: "allow",
            policy_note: None,
        });
    }
    // Locked screen / UAC prompt: nothing on the user's desktop can be seen or touched, and the
    // OS reports that as confusing errors (covered windows, failed captures — P2-M2 gate run).
    match input_desktop_name() {
        Some(d) if d.eq_ignore_ascii_case("Default") => {}
        other => {
            return Err(err(
                "NEEDS_HUMAN",
                format!(
                    "the screen is locked or a secure prompt is showing (input desktop: {}); nothing was done",
                    other.as_deref().unwrap_or("not accessible")
                ),
                "Wait until the user unlocks the PC / answers the prompt, then retry.",
            ))
        }
    }
    // ADR 0050: while the PC is under strain, opening more asks the user first.
    crate::health::brake(method, p)?;
    // Skills: a macro runs as a batch (each step authorised); a built-in skill is checked
    // with its own class and description (protected paths refused in plan()).
    let skill = if method == "run_skill" {
        let pl = crate::skills::plan(p)?;
        if pl.is_macro {
            return Ok(Approval {
                decision: "allow",
                policy_note: None,
            });
        }
        Some(pl)
    } else {
        None
    };
    // ADR 0073: the organisation's signed team policy is a floor (stricter wins).
    let team = match crate::team::current() {
        crate::team::State::Active(t) => Some(t),
        _ => None,
    };
    let ((decision, reason, what, rate, task), _note) = with_policy(|s| {
        let (mut f, mut what) = gather(method, p);
        if let Some(pl) = &skill {
            f.method = format!("run_skill:{}", pl.name);
            f.skill_class = Some(pl.class);
            what = pl.what.clone();
        }
        let (mut d, mut why) = s.policy.decide(&f);
        let (mut rate, mut max_task) = (s.policy.rate_per_minute, s.policy.max_actions_per_task);
        if let Some(t) = &team {
            (d, why) = crate::team::stricter((d, why), t.decide(&f));
            rate = rate.min(t.rate_per_minute.unwrap_or(u32::MAX));
            max_task = max_task.min(t.max_actions_per_task.unwrap_or(u32::MAX));
        }
        // ADR 0074: a skill that promises to ask the user always does (a deny still wins).
        // A routine was approved by the user in its own dialog when it was added; its runs
        // are unattended and do not ask again.
        let routine = crate::locks::current_client().is_some_and(|c| c.starts_with("routine:"));
        if skill.as_ref().is_some_and(|pl| pl.asks) && d == Decision::Allow && !routine {
            d = Decision::Confirm;
            why = "this skill changes your files or documents and always asks first".into();
        }
        (d, why, what, rate, (max_task, s.policy.task_idle_minutes))
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
    // Max actions per task (ADR 0038): ask the user to continue a very long task.
    let (max_task, idle_min) = task;
    let check = {
        let mut g = TASK.lock().unwrap_or_else(|e| e.into_inner());
        let (t0, lim) = g.get_or_insert_with(|| (Instant::now(), policy::TaskLimiter::default()));
        lim.check(
            t0.elapsed().as_millis() as u64,
            max_task,
            idle_min as u64 * 60_000,
        )
    };
    match check {
        policy::TaskCheck::Go => {}
        policy::TaskCheck::Refused => {
            return Err(blocked(format!(
                "you stopped this long task; the agent may continue after a {idle_min}-minute pause"
            )))
        }
        policy::TaskCheck::Ask(n) => {
            activity("waiting for you: long task");
            let before = winmgr::foreground_root();
            let answer = prompt(
                "XtraOS Agent — long task",
                &format!(
                    "The AI agent has done {n} actions in this task without a break.\n\nNext: {what}\n\nLet it continue for another {max_task} actions?"
                ),
                vec!["Continue".into(), "Stop".into()],
                1,
                Duration::from_secs(60),
            )?;
            if !before.is_null() && winmgr::exists(before) && !is_own_window(before) {
                winmgr::focus(before);
            }
            let mut g = TASK.lock().unwrap_or_else(|e| e.into_inner());
            let lim = &mut g
                .get_or_insert_with(|| (Instant::now(), policy::TaskLimiter::default()))
                .1;
            if answer == Some(0) {
                lim.continue_task();
            } else {
                lim.stop();
                return Err(blocked(format!(
                    "{n} actions in this task: stopped by the user (max_actions_per_task)"
                )));
            }
        }
    }
    let decision = match decision {
        Decision::Allow => "allow",
        Decision::Deny => return Err(blocked(format!("{what}: {reason}"))),
        Decision::Confirm => {
            activity(&format!("waiting for your approval: {what}"));
            // The dialog takes the foreground; give it back afterwards so the approved action
            // (e.g. Enter in a chat) reaches the window the user was looking at.
            let before = winmgr::foreground_root();
            let answer = prompt(
                "XtraOS Agent — approval needed",
                &format!(
                    "The AI agent wants to:\n\n{what}\n\nWhy you are asked: {reason}.\n\nAllow this once?"
                ),
                vec!["Allow once".into(), "Deny".into()],
                1,
                Duration::from_secs(60),
            )?;
            if !before.is_null() && winmgr::exists(before) && !is_own_window(before) {
                winmgr::focus(before);
                std::thread::sleep(Duration::from_millis(150));
            }
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
        policy_note: None, // reload notes are logged through take_policy_notes()
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
                uid: crate::winsec::random_bytes(16)
                    .map(|b| b.iter().map(|x| format!("{x:02x}")).collect())
                    .unwrap_or_else(|_| format!("p{id}")),
                title: title.into(),
                message: message.into(),
                buttons,
                cancel,
                hwnd: 0,
                result: None,
                open: true,
                opened: Instant::now(),
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
        post_close_prompt(p.hwnd);
    }
}

/// Asks the UI thread to destroy a dialog. It carries the window itself, not the prompt id:
/// the caller forgets the prompt right after, and a dialog looked up by id then would stay
/// on screen, dead (found by the P2-M4 WA-03 test).
fn post_close_prompt(hwnd: usize) {
    if hwnd != 0 {
        post(WM_APP_CLOSE_PROMPT, hwnd);
    }
}

fn forget_prompt(id: u64) {
    if let Some(m) = PROMPTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&id);
    }
}

/// An open dialog as a paired device sees it (ADR 0076).
#[derive(Clone, Debug)]
pub struct OpenPrompt {
    pub uid: String,
    pub title: String,
    pub message: String,
    pub buttons: Vec<String>,
    pub cancel: usize,
    pub age_ms: u64,
}

/// The dialogs waiting for an answer, oldest first.
pub fn open_prompts() -> Vec<OpenPrompt> {
    let g = PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<(u64, OpenPrompt)> = g
        .as_ref()
        .map(|m| {
            m.iter()
                .filter(|(_, p)| p.open && p.result.is_none())
                .map(|(id, p)| {
                    (
                        *id,
                        OpenPrompt {
                            uid: p.uid.clone(),
                            title: p.title.clone(),
                            message: p.message.clone(),
                            buttons: p.buttons.clone(),
                            cancel: p.cancel,
                            age_ms: p.opened.elapsed().as_millis() as u64,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(id, _)| *id);
    v.into_iter().map(|(_, p)| p).collect()
}

/// Answers an open dialog by its uid (a checked answer from a paired device) and closes its
/// window. False when it is not open any more.
pub fn answer_uid(uid: &str, button: usize) -> bool {
    let found = {
        let g = PROMPTS.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref()
            .and_then(|m| m.iter().find(|(_, p)| p.uid == uid && p.open && p.result.is_none()))
            .map(|(id, p)| (*id, p.hwnd))
    };
    let Some((id, hwnd)) = found else { return false };
    answer(id, button);
    post_close_prompt(hwnd);
    true
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

/// Answering dialogs from a paired device (ADR 0076): not under the policy, like ask_human.
fn is_approval_method(m: &str) -> bool {
    crate::approvals::METHODS.contains(&m)
}

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
    // P6-M7 (ADR 0051): a notification, no answer, returns at once; throttled.
    if kind == "notify" {
        let now = Instant::now();
        let mut last = LAST_NOTIFY.lock().unwrap_or_else(|e| e.into_inner());
        if !notify_allowed(*last, now) {
            return Ok(json!({"shown": false, "throttled": true}));
        }
        *last = Some(now);
        let shown = notify("XtraOS Agent", &perceive::clip(message, 250), false);
        return Ok(json!({"shown": shown}));
    }
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
                "unknown kind '{k}' (confirm, choice, info, task, notify)"
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

/// The action finished: the banner stays 2.5 s more, then hides (ipc calls this).
pub fn activity_end() {
    post(WM_APP_ACTIVITY_END, 0);
}

/// Shows "XtraOS Agent · <what>" on the overlay banner (never typed text).
pub fn activity(what: &str) {
    let what = if crate::recorder::active() {
        format!("● REC · {what}")
    } else {
        what.to_string()
    };
    *OVERLAY_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = perceive::clip(&what, 90);
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

/// A Windows notification from the XtraOS tray icon (shown as a toast); returns at once
/// (P6-M7, ADR 0051). False when the tray icon is not available.
pub fn notify(title: &str, text: &str, warning: bool) -> bool {
    let Some(u) = UI.get().filter(|u| u.ok) else {
        return false;
    };
    let mut nid = tray_data(WHWND(u.msg as _), tray_tip());
    nid.uFlags |= NIF_INFO;
    nid.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
    for (i, c) in title.encode_utf16().take(nid.szInfoTitle.len() - 1).enumerate() {
        nid.szInfoTitle[i] = c;
    }
    for (i, c) in text.encode_utf16().take(nid.szInfo.len() - 1).enumerate() {
        nid.szInfo[i] = c;
    }
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) }.as_bool()
}

/// At most one agent notification per 10 s (a looping agent cannot flood the user). Pure.
pub fn notify_allowed(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_secs(10))
}

static LAST_NOTIFY: Mutex<Option<Instant>> = Mutex::new(None);

/// A notification that waits for the 10-second spacing (≤ 11 s) instead of being dropped
/// (ADR 0066: two reminders in the same minute both show). False: no tray icon.
pub fn notify_waiting(title: &str, text: &str) -> bool {
    let t0 = Instant::now();
    loop {
        {
            let now = Instant::now();
            let mut last = LAST_NOTIFY.lock().unwrap_or_else(|e| e.into_inner());
            if notify_allowed(*last, now) || t0.elapsed() > std::time::Duration::from_secs(11) {
                *last = Some(now);
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    notify(title, text, false)
}

static HELPER_MODE: AtomicBool = AtomicBool::new(false);

/// This process is the elevated admin helper (ADR 0030): its own tray text and dialog
/// titles, no global hotkey (the core owns Ctrl+Alt+Esc).
pub fn set_helper_mode() {
    HELPER_MODE.store(true, Ordering::SeqCst);
}

pub fn helper_mode() -> bool {
    HELPER_MODE.load(Ordering::SeqCst)
}

fn tray_tip() -> &'static str {
    if helper_mode() {
        return "XtraOS admin helper — running as administrator (right-click to stop)";
    }
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
        let hotkey = !helper_mode()
            && RegisterHotKey(
                Some(msg),
                HOTKEY_ID,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                VK_ESCAPE.0 as u32,
            )
            .is_ok();
        if !hotkey && !helper_mode() {
            eprintln!(
                "[xtraos-core] Ctrl+Alt+Esc is taken by another program: use the tray icon to stop"
            );
        }
        // Low-level input hooks: tell the user's real activity from the core's own input and
        // from tiny mouse jitter (ADR 0027). Without them the core falls back to
        // GetLastInputInfo.
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), Some(hinst), 0);
        let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(hinst), 0);
        HOOKS.store(kb.is_ok() && ms.is_ok(), Ordering::Relaxed);
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
            // Any program can post messages here, so only the safe commands are accepted:
            // Stop, and Quit (the uninstaller). Resume comes only from the tray menu itself
            // (ADR 0041).
            match wp.0 & 0xFFFF {
                ID_STOP => stop(),
                ID_QUIT => quit(h),
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
        WM_APP_ACTIVITY_END => {
            if !stopped() {
                SetTimer(Some(h), OVERLAY_TIMER, 2500, None);
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
            // wParam is the dialog window. Destroy it only if it still is one of our dialogs
            // (never a window that reused a stale handle).
            let hw = WHWND(wp.0 as _);
            let mut cls = [0u16; 32];
            let n = GetClassNameW(hw, &mut cls);
            if n > 0 && String::from_utf16_lossy(&cls[..n as usize]) == "XtraOSPrompt" {
                let _ = DestroyWindow(hw);
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
    // P8-M3 (ADR 0061): pause or resume one agent; the others keep working.
    let agents = crate::agents::menu_items();
    let agent_labels: Vec<Vec<u16>> = agents.iter().map(|(_, l)| wide(&perceive::clip(l, 110))).collect();
    let agents_title = wide(if agents.is_empty() { "Agents (none connected)" } else { "Agents" });
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    if agents.is_empty() {
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(agents_title.as_ptr()));
    } else if let Ok(sub) = CreatePopupMenu() {
        for (i, (client, _)) in agents.iter().enumerate() {
            let flags = if crate::agents::is_paused(client) { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(sub, flags, ID_AGENT_BASE + i, PCWSTR(agent_labels[i].as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(agents_title.as_ptr()));
    }
    let viewer_label = wide("Rules and activity…");
    let _ = AppendMenuW(menu, MF_STRING, ID_VIEWER, PCWSTR(viewer_label.as_ptr()));
    let quit_label = wide("Quit XtraOS core");
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(quit_label.as_ptr()));
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
        ID_QUIT => quit(h),
        ID_VIEWER => open_viewer(),
        c if c >= ID_AGENT_BASE && c < ID_AGENT_BASE + agents.len() => {
            let client = &agents[c - ID_AGENT_BASE].0;
            let on = !crate::agents::is_paused(client);
            crate::agents::set_paused(client, on);
            eprintln!("[xtraos-core] agent {client} {} by the user", if on { "paused" } else { "resumed" });
        }
        _ => {}
    }
}

/// Tray "Rules and activity…": the read-only viewer in its own process (ADR 0034).
fn open_viewer() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--viewer").spawn();
    }
}

/// Tray "Quit" (and the uninstaller): halt input, remove the icon, end the process. Clients
/// see the pipe close; the MCP server starts the core again on the next call (ADR 0019).
unsafe fn quit(h: WHWND) -> ! {
    stop();
    let nid = tray_data(h, "");
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    eprintln!("[xtraos-core] quit from the tray");
    std::process::exit(0)
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
        // While an action runs (it may wait for the user to be idle) the banner stays up;
        // the hide timer starts when the action ends (activity_end).
        let _ = KillTimer(Some(WHWND(u as _)), OVERLAY_TIMER);
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

static SYNTHETIC_OK: AtomicBool = AtomicBool::new(false);
/// Last real (non-injected) left-button release: (tick, x, y); and last real Enter/Space.
static REAL_CLICK: Mutex<(u32, i32, i32)> = Mutex::new((0, 0, 0));
static REAL_KEY_TICK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const REAL_INPUT_WINDOW_MS: u32 = 1500;

/// Unelevated test-mode admin helper only (it has no extra power): accept synthetic answers.
pub fn allow_synthetic_answers() {
    SYNTHETIC_OK.store(true, Ordering::SeqCst);
}

/// True if the user really pressed this button (ADR 0041): the safety hooks saw a real,
/// non-injected left click inside the button within the last moment, or a real Enter/Space
/// press, or Windows reports the current input message as hardware. Clicks faked by other
/// programs (BM_CLICK, posted WM_COMMAND, SendInput) satisfy none of these.
fn from_hardware_input(button: WHWND) -> bool {
    if SYNTHETIC_OK.load(Ordering::SeqCst) {
        return true;
    }
    let now = unsafe { GetTickCount() };
    let (t, x, y) = *REAL_CLICK.lock().unwrap_or_else(|e| e.into_inner());
    if t != 0 && now.wrapping_sub(t) <= REAL_INPUT_WINDOW_MS {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(button, &mut r) }.is_ok()
            && x >= r.left
            && x < r.right
            && y >= r.top
            && y < r.bottom
        {
            return true;
        }
    }
    let k = REAL_KEY_TICK.load(Ordering::Relaxed);
    if k != 0 && now.wrapping_sub(k) <= REAL_INPUT_WINDOW_MS && unsafe { GetFocus() } == button {
        return true;
    }
    #[repr(C)]
    struct InputMessageSource {
        device_type: u32,
        origin_id: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetCurrentInputMessageSource(s: *mut InputMessageSource) -> i32;
    }
    const IMO_HARDWARE: u32 = 0x0000_0001;
    let mut s = InputMessageSource {
        device_type: 0,
        origin_id: 0,
    };
    unsafe { GetCurrentInputMessageSource(&mut s) != 0 && s.origin_id == IMO_HARDWARE }
}

/// Tests only, in-process: answer the open dialog with button `i` as the human would. A
/// separate process cannot call this (ADR 0041).
#[doc(hidden)]
pub fn answer_prompt_for_tests(i: usize) -> bool {
    let open = PROMPTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| {
            m.iter()
                .find(|(_, p)| p.open && p.hwnd != 0)
                .map(|(id, p)| (*id, p.hwnd))
        });
    match open {
        Some((id, hwnd)) => {
            answer(id, i);
            post_close_prompt(hwnd);
            true
        }
        None => false,
    }
}

/// Tests only, in-process: what the tray's "Resume" does.
#[doc(hidden)]
pub fn resume_for_tests() {
    resume();
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
                .map(|p| (p.cancel, p.buttons.len(), p.opened.elapsed().as_millis()));
            if let Some((cancel, n, open_ms)) = cancel {
                let i = ctl - PROMPT_BUTTON_BASE;
                if (0..n as i32).contains(&i) {
                    // Only the safe answer may come from anywhere; any other answer must come
                    // from the user's real mouse/keyboard, never from another program's
                    // BM_CLICK / WM_COMMAND / SendInput (ADR 0041), and not in the first
                    // moment after the prompt appeared (a stray click, ADR 0050).
                    if i as usize != cancel
                        && !answer_counts(
                            i as usize,
                            cancel,
                            open_ms,
                            from_hardware_input(WHWND(lp.0 as _)),
                        )
                    {
                        return LRESULT(0);
                    }
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

    // A stray click right after a prompt appears never approves anything.
    #[test]
    fn early_or_synthetic_approvals_do_not_count() {
        assert!(answer_counts(1, 1, 0, false), "the safe answer always counts");
        assert!(!answer_counts(0, 1, 100, true), "too early");
        assert!(!answer_counts(0, 1, 5_000, false), "not from real input");
        assert!(answer_counts(0, 1, 900, true));
    }

    // P6-M7 (ADR 0051): a looping agent cannot flood the user with notifications.
    #[test]
    fn notifications_are_throttled() {
        let t = Instant::now();
        assert!(notify_allowed(None, t));
        assert!(!notify_allowed(Some(t), t + std::time::Duration::from_secs(9)));
        assert!(notify_allowed(Some(t), t + std::time::Duration::from_secs(10)));
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
