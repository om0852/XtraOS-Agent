//! Real OS input (Phase 1 M2, ADR 0010): mouse, keyboard, wheel, drag, clipboard.
//!
//! Every action runs under the safety guards proven in Phase 0 (ADR 0005/0006):
//! - USER_ACTIVE: a mouse button / modifier is physically held at start, or the user moves the
//!   cursor during the action → abort;
//! - TARGET_OBSCURED: with `window`, the point must be inside that window (root) right before
//!   every button event;
//! - keyboard needs `window` and it must be the foreground window before every key;
//! - pressed modifiers / buttons are released on every exit path (Drop guards).
//!
//! M4 (ADRs 0014, 0015): `ref` targets act in the background first (perceive.rs). Real input is
//! polite: it waits until the user has been idle `idle_ms`, and mouse tools put the cursor back.
//!
//! Win32 docs:
//! SendInput  https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-sendinput
//! MOUSEINPUT https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-mouseinput
//! KEYBDINPUT https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-keybdinput
//! Clipboard  https://learn.microsoft.com/windows/win32/dataxchg/using-the-clipboard
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::thread::sleep;
use std::time::Duration;

use serde_json::{json, Value};

pub type HWND = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MOUSEINPUT {
    dx: i32,
    dy: i32,
    mouseData: u32,
    dwFlags: u32,
    time: u32,
    dwExtraInfo: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KEYBDINPUT {
    wVk: u16,
    wScan: u16,
    dwFlags: u32,
    time: u32,
    dwExtraInfo: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
union INPUT_U {
    mi: MOUSEINPUT,
    ki: KEYBDINPUT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct INPUT {
    kind: u32,
    u: INPUT_U,
}

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;

const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_HWHEEL: u32 = 0x1000;
const MOUSEEVENTF_VIRTUALDESK: u32 = 0x4000;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;
const WHEEL_DELTA: i32 = 120;

const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_UNICODE: u32 = 0x0004;

const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;
const GA_ROOT: u32 = 2;
const MAPVK_VK_TO_VSC: u32 = 0;
const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

pub const VK_LBUTTON: i32 = 0x01;
pub const VK_RBUTTON: i32 = 0x02;
pub const VK_MBUTTON: i32 = 0x04;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_LWIN: u16 = 0x5B;
const VK_RETURN: u16 = 0x0D;
const VK_TAB: u16 = 0x09;

#[link(name = "user32")]
extern "system" {
    fn SendInput(n: u32, inputs: *const INPUT, size: i32) -> u32;
    fn GetCursorPos(p: *mut POINT) -> i32;
    fn WindowFromPoint(p: POINT) -> HWND;
    fn GetAncestor(h: HWND, flags: u32) -> HWND;
    fn GetForegroundWindow() -> HWND;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetSystemMetrics(i: i32) -> i32;
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
    fn VkKeyScanW(ch: u16) -> i16;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> i32;
    fn OpenClipboard(owner: HWND) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
    fn GetLastInputInfo(info: *mut LASTINPUTINFO) -> i32;
    fn WaitForInputIdle(process: *mut c_void, ms: u32) -> u32;
}

#[repr(C)]
struct LASTINPUTINFO {
    cbSize: u32,
    dwTime: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(mem: *mut c_void) -> i32;
    fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    fn GetTickCount() -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn CloseHandle(h: *mut c_void) -> i32;
}

const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;

/// Lets typing follow the target app's pace: WaitForInputIdle returns once the app has
/// processed the keystrokes queued so far (M6 bench: `type` used to return while the app was
/// still working through them, so a following observe could see half the text).
/// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-waitforinputidle
struct InputIdle(*mut c_void);

impl InputIdle {
    fn open(window: HWND) -> Option<Self> {
        let pid = crate::winmgr::pid_of(window);
        let mut h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | SYNCHRONIZE, 0, pid) };
        if h.is_null() {
            h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
        }
        (!h.is_null()).then_some(InputIdle(h))
    }

    fn wait(&self, ms: u32) {
        // SendInput hands events to the system input thread, which delivers them to the app's
        // queue a moment later; checking "idle" at once can pass before they even arrive.
        sleep(Duration::from_millis(3));
        unsafe { WaitForInputIdle(self.0, ms) };
    }
}

impl Drop for InputIdle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// ---------------------------------------------------------------------------------------------
// Errors

#[derive(Debug, Clone, PartialEq)]
pub struct ActError {
    pub code: &'static str,
    pub message: String,
    pub hint: &'static str,
}

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

fn user_active(what: &str) -> ActError {
    err(
        "USER_ACTIVE",
        format!("aborted: {what}"),
        "The user is using the mouse/keyboard. Wait, then retry.",
    )
}

// ---------------------------------------------------------------------------------------------
// Setup + low-level senders

static DPI: Once = Once::new();
static ACTION_LOCK: Mutex<()> = Mutex::new(());
/// Tick (GetTickCount) right after our last SendInput; 0 = never.
static LAST_INJECT: AtomicU32 = AtomicU32::new(0);

/// Counts OS actions; a background watcher stops as soon as any newer action starts.
static ACTION_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One OS action at a time, process-wide (input and background actions share it).
pub fn action_lock() -> MutexGuard<'static, ()> {
    let g = ACTION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    next_action();
    g
}

/// Marks the start of an OS action; returns its generation.
pub fn next_action() -> u64 {
    ACTION_GEN.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn action_generation() -> u64 {
    ACTION_GEN.load(Ordering::SeqCst)
}

/// Tick of the core's last injected input event (0 = never). Lets tests prove that a
/// background action injected nothing.
pub fn last_inject_tick() -> u32 {
    LAST_INJECT.load(Ordering::Relaxed)
}

/// Per-monitor DPI aware v2 for the whole process: all coordinates are physical pixels.
pub fn init_dpi() {
    DPI.call_once(|| unsafe {
        SetProcessDpiAwarenessContext(-4);
    });
}

/// Every injected event goes through here: nothing is sent after the kill switch (M5).
fn send(inputs: &[INPUT]) -> Result<(), ActError> {
    crate::safety::check()?;
    send_always(inputs)
}

/// Only for releasing held buttons/keys: must work even after the kill switch.
fn send_always(inputs: &[INPUT]) -> Result<(), ActError> {
    let n = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    LAST_INJECT.store(unsafe { GetTickCount() }.max(1), Ordering::Relaxed);
    if n as usize != inputs.len() {
        return Err(err(
            "INTERNAL",
            format!("SendInput injected {n}/{} events", inputs.len()),
            "Input may be blocked by a higher-integrity window (UIPI) or the secure desktop.",
        ));
    }
    Ok(())
}

fn mouse(flags: u32, dx: i32, dy: i32, data: u32) -> INPUT {
    INPUT {
        kind: INPUT_MOUSE,
        u: INPUT_U {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        kind: INPUT_KEYBOARD,
        u: INPUT_U {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

pub fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) };
    p
}

fn held(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
}

/// Absolute coordinates normalised over the virtual desktop (0..=65535).
pub fn normalise(p: POINT, vx: i32, vy: i32, vw: i32, vh: i32) -> (i32, i32) {
    let nx = ((p.x - vx) as f64 * 65535.0 / (vw - 1).max(1) as f64).round() as i32;
    let ny = ((p.y - vy) as f64 * 65535.0 / (vh - 1).max(1) as f64).round() as i32;
    (nx, ny)
}

fn virtual_desktop() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Validated at argument-parsing time, before any guard reads live input state.
fn on_desktop(p: POINT) -> Result<POINT, ActError> {
    let (vx, vy, vw, vh) = virtual_desktop();
    if p.x < vx || p.y < vy || p.x >= vx + vw || p.y >= vy + vh {
        return Err(bad(format!(
            "point ({},{}) is outside the desktop {vx},{vy} {vw}x{vh}",
            p.x, p.y
        )));
    }
    Ok(p)
}

fn move_abs(p: POINT) -> Result<(), ActError> {
    let p = on_desktop(p)?;
    let (vx, vy, vw, vh) = virtual_desktop();
    let (nx, ny) = normalise(p, vx, vy, vw, vh);
    send(&[mouse(
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        nx,
        ny,
        0,
    )])
}

pub fn root_at(p: POINT) -> HWND {
    unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn flags(self) -> (u32, u32) {
        match self {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Politeness (ADR 0014): wait for the user to be idle; put the cursor back afterwards.

/// Classifies the newest input event: `None` = it was our own injection (the user has not
/// touched anything since), else `Some(ms since the user's last input)`. Pure for testing.
pub fn user_idle(now: u32, last_input: u32, last_inject: u32) -> Option<u32> {
    // Our own SendInput updates GetLastInputInfo too; events up to 50 ms after our last
    // injection are attributed to it (the OS stamps them slightly later).
    if last_inject != 0 && (last_input.wrapping_sub(last_inject) as i32) <= 50 {
        return None;
    }
    Some(now.wrapping_sub(last_input))
}

fn last_input_tick() -> u32 {
    let mut i = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe { GetLastInputInfo(&mut i) };
    i.dwTime
}

/// Options shared by every real-input tool.
#[derive(Clone, Copy, Debug)]
pub struct Polite {
    pub idle_ms: u32,
    pub timeout_ms: u32,
    pub restore_cursor: bool,
}

pub fn polite_args(p: &Value, restore_default: bool) -> Result<Polite, ActError> {
    Ok(Polite {
        idle_ms: opt_int(p, "idle_ms", 800, 0, 60_000)? as u32,
        timeout_ms: opt_int(p, "timeout_ms", 5_000, 0, 120_000)? as u32,
        restore_cursor: match p.get("restore_cursor") {
            None | Some(Value::Null) => restore_default,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| bad("'restore_cursor' must be true or false"))?,
        },
    })
}

/// Waits (at most timeout_ms) until the user has been idle for `idle_ms`; else USER_ACTIVE.
pub fn wait_user_idle(o: &Polite) -> Result<(), ActError> {
    if o.idle_ms == 0 {
        return Ok(());
    }
    let start = unsafe { GetTickCount() };
    loop {
        crate::safety::check()?;
        let now = unsafe { GetTickCount() };
        match user_idle(now, last_input_tick(), LAST_INJECT.load(Ordering::Relaxed)) {
            None => return Ok(()),
            Some(ms) if ms >= o.idle_ms => return Ok(()),
            Some(ms) => {
                if now.wrapping_sub(start) >= o.timeout_ms {
                    return Err(err(
                        "USER_ACTIVE",
                        format!(
                            "the user is working (last input {ms} ms ago; the core's own last input {}); nothing was sent",
                            match LAST_INJECT.load(Ordering::Relaxed) {
                                0 => "never".to_string(),
                                t => format!("{} ms ago", now.wrapping_sub(t)),
                            }
                        ),
                        "Retry later, use a ref (background, no mouse/keys), or ask_human.",
                    ));
                }
                sleep(Duration::from_millis(u64::from(
                    (o.idle_ms - ms).clamp(10, 50),
                )));
            }
        }
    }
}

/// Puts the cursor back where the user left it, unless the user has moved it since.
fn restore_cursor(o: &Polite, start: POINT, g: &MouseGuard) {
    if crate::safety::stopped() {
        return;
    }
    if o.restore_cursor && !g.user_moved() && cursor() != start && on_desktop(start).is_ok() {
        let _ = move_abs(start);
    }
}

// ---------------------------------------------------------------------------------------------
// Guards

/// Tracks where *we* last put the cursor so user movement can be detected.
struct MouseGuard {
    last: POINT,
    window: Option<HWND>,
}

impl MouseGuard {
    fn begin(window: Option<HWND>) -> Result<Self, ActError> {
        if held(VK_LBUTTON) || held(VK_RBUTTON) || held(VK_MBUTTON) {
            return Err(user_active("a mouse button is held"));
        }
        Ok(MouseGuard {
            last: cursor(),
            window,
        })
    }

    fn user_moved(&self) -> bool {
        let c = cursor();
        (c.x - self.last.x).abs() > 2 || (c.y - self.last.y).abs() > 2
    }

    fn check_target(&self, p: POINT, window: Option<HWND>) -> Result<(), ActError> {
        let under = root_at(p);
        if crate::safety::is_own_window(under) {
            return Err(crate::safety::blocked(
                "that point is on an XtraOS window (overlay/dialog): only the user may click it",
            ));
        }
        if !under.is_null() && crate::winmgr::is_elevated(under) {
            return Err(crate::winmgr::elevation_required("input at this point"));
        }
        if let Some(w) = window.or(self.window) {
            if under != w {
                return Err(err("TARGET_OBSCURED", format!("({},{}) is not inside the target window", p.x, p.y),
                    "The window may be covered, moved or minimised. Bring it to the front and re-read positions."));
            }
        }
        Ok(())
    }

    fn move_to(&mut self, p: POINT) -> Result<(), ActError> {
        if self.user_moved() {
            return Err(user_active("the mouse was moved by the user"));
        }
        move_abs(p)?;
        // Remember where we AIMED, not where the cursor happens to be: re-reading the cursor
        // would adopt a user's movement as ours and the guard would never fire (test in10).
        self.last = p;
        // The OS applies the move asynchronously; under load the cursor can lag > 8 ms. Wait
        // until it arrives (else a click would land at the old position, and a too-early check
        // would report a false USER_ACTIVE — both seen in in01). Never arriving = user input.
        for _ in 0..12 {
            if !self.user_moved() {
                return Ok(());
            }
            sleep(Duration::from_millis(5));
        }
        Err(user_active("the mouse was moved by the user"))
    }

    /// Before a button event: user still idle and point still inside the target window.
    fn before_button(&self, p: POINT, window: Option<HWND>) -> Result<(), ActError> {
        if self.user_moved() {
            return Err(user_active("the mouse was moved by the user"));
        }
        self.check_target(p, window)
    }
}

/// Releases a mouse button if still down when dropped.
struct ButtonHold {
    up: u32,
    down: bool,
}

impl ButtonHold {
    fn press(b: Button) -> Result<Self, ActError> {
        let (d, u) = b.flags();
        send(&[mouse(d, 0, 0, 0)])?;
        Ok(ButtonHold { up: u, down: true })
    }
    fn release(&mut self) -> Result<(), ActError> {
        if self.down {
            self.down = false;
            send_always(&[mouse(self.up, 0, 0, 0)])?;
        }
        Ok(())
    }
}

impl Drop for ButtonHold {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

/// Holds modifier keys; releases them in reverse order when dropped.
struct ModHold(Vec<u16>);

impl ModHold {
    fn press(mods: &[u16]) -> Result<Self, ActError> {
        let mut h = ModHold(Vec::new());
        for &m in mods {
            send(&[key_event(m, false)])?;
            h.0.push(m);
        }
        Ok(h)
    }
}

impl Drop for ModHold {
    fn drop(&mut self) {
        for &m in self.0.iter().rev() {
            let _ = send_always(&[key_event(m, true)]);
        }
    }
}

fn modifiers_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, 0x5C]
        .iter()
        .any(|&vk| held(vk as i32))
}

fn foreground_is(window: HWND) -> Result<(), ActError> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_null() || unsafe { GetAncestor(fg, GA_ROOT) } != window {
        return Err(err(
            "NEEDS_HUMAN",
            "the target window is not the foreground window; no keys were sent",
            "Call focus_window first (or ask the user to click the window), then retry.",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Keys

fn is_extended(vk: u16) -> bool {
    matches!(
        vk,
        0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0x5C | 0x5D | 0x6F | 0x90 | 0xA3 | 0xA5
    )
}

fn key_event(vk: u16, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    key(vk, scan, flags)
}

/// Virtual-key code for a key name (case-insensitive), without OS lookups.
pub fn named_vk(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    let vk = match n.as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" | "menu" => VK_MENU,
        "win" | "meta" | "cmd" | "super" => VK_LWIN,
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" | "escape" => 0x1B,
        "backspace" | "back" => 0x08,
        "space" => 0x20,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" | "arrowleft" => 0x25,
        "up" | "arrowup" => 0x26,
        "right" | "arrowright" => 0x27,
        "down" | "arrowdown" => 0x28,
        "insert" | "ins" => 0x2D,
        "delete" | "del" => 0x2E,
        "capslock" => 0x14,
        "printscreen" | "prtsc" => 0x2C,
        "apps" | "contextmenu" => 0x5D,
        _ => {
            if let Some(num) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
                if (1..=24).contains(&num) {
                    return Some(0x6F + num);
                }
            }
            let mut chars = n.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_lowercase() => Some(c.to_ascii_uppercase() as u16),
                (Some(c), None) if c.is_ascii_digit() => Some(c as u16),
                _ => None,
            };
        }
    };
    Some(vk)
}

fn is_modifier(vk: u16) -> bool {
    matches!(vk, VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN)
}

#[derive(Debug, PartialEq)]
pub struct Chord {
    pub mods: Vec<u16>,
    pub key: u16,
}

/// Parses "Ctrl+Shift+P", "Alt+F4", "Ctrl++", "Enter". Symbols use the current layout.
pub fn parse_chord(s: &str) -> Result<Chord, ActError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(bad("empty key"));
    }
    let (rest, last) = if s.len() > 1 && s.ends_with("++") {
        (&s[..s.len() - 2], "+")
    } else if s == "+" {
        ("", "+")
    } else {
        match s.rfind('+') {
            Some(i) => (&s[..i], &s[i + 1..]),
            None => ("", s),
        }
    };
    let mut mods = Vec::new();
    for part in rest.split('+').filter(|p| !p.is_empty()) {
        match named_vk(part) {
            Some(vk) if is_modifier(vk) => {
                if !mods.contains(&vk) {
                    mods.push(vk)
                }
            }
            _ => return Err(bad(format!("'{part}' is not a modifier in '{s}'"))),
        }
    }
    let key = match named_vk(last) {
        Some(vk) => vk,
        None => symbol_vk(last).ok_or_else(|| bad(format!("unknown key '{last}' in '{s}'")))?,
    };
    Ok(Chord { mods, key })
}

/// Single printable symbol → (vk, extra shift) via the active keyboard layout.
fn symbol_vk(sym: &str) -> Option<u16> {
    let mut it = sym.encode_utf16();
    let (Some(u), None) = (it.next(), it.next()) else {
        return None;
    };
    let r = unsafe { VkKeyScanW(u) };
    if r == -1 {
        None
    } else {
        Some((r as u16) & 0xFF)
    }
}

/// Unicode text → key events (Enter/Tab as real keys, everything else as UNICODE packets).
pub fn text_events(text: &str) -> Vec<Vec<INPUTDESC>> {
    let mut out = Vec::new();
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' => out.push(vec![INPUTDESC::Vk(VK_RETURN)]),
            '\t' => out.push(vec![INPUTDESC::Vk(VK_TAB)]),
            c => {
                let mut buf = [0u16; 2];
                out.push(
                    c.encode_utf16(&mut buf)
                        .iter()
                        .map(|&u| INPUTDESC::Unicode(u))
                        .collect(),
                );
            }
        }
    }
    out
}

/// Testable description of keyboard input for one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum INPUTDESC {
    Vk(u16),
    Unicode(u16),
}

fn send_char(units: &[INPUTDESC]) -> Result<(), ActError> {
    send(&char_events(units))
}

fn char_events(units: &[INPUTDESC]) -> Vec<INPUT> {
    let mut evs = Vec::with_capacity(units.len() * 2);
    for &u in units {
        match u {
            INPUTDESC::Vk(vk) => {
                evs.push(key_event(vk, false));
                evs.push(key_event(vk, true));
            }
            INPUTDESC::Unicode(c) => {
                evs.push(key(0, c, KEYEVENTF_UNICODE));
                evs.push(key(0, c, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
            }
        }
    }
    evs
}

// ---------------------------------------------------------------------------------------------
// Param helpers

fn int(p: &Value, k: &str) -> Result<i32, ActError> {
    p.get(k)
        .and_then(Value::as_i64)
        .map(|v| v as i32)
        .ok_or_else(|| bad(format!("'{k}' (integer) is required")))
}

fn opt_int(p: &Value, k: &str, default: i64, min: i64, max: i64) -> Result<i64, ActError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v
                .as_i64()
                .ok_or_else(|| bad(format!("'{k}' must be an integer")))?;
            if n < min || n > max {
                return Err(bad(format!("'{k}' must be between {min} and {max}")));
            }
            Ok(n)
        }
    }
}

fn point(p: &Value, kx: &str, ky: &str) -> Result<POINT, ActError> {
    on_desktop(POINT {
        x: int(p, kx)?,
        y: int(p, ky)?,
    })
}

fn window_arg(p: &Value, k: &str, required: bool) -> Result<Option<HWND>, ActError> {
    match p.get(k) {
        None | Some(Value::Null) if !required => Ok(None),
        None | Some(Value::Null) => Err(bad(format!(
            "'{k}' (window handle) is required for keyboard input"
        ))),
        Some(v) => {
            // "w123" or 123 (ADR 0012). UIPI would silently drop input into an elevated
            // window, so refuse before anything is sent.
            let h = crate::winmgr::resolve_window(v)?;
            if crate::winmgr::is_elevated(h) {
                return Err(crate::winmgr::elevation_required("input"));
            }
            Ok(Some(h))
        }
    }
}

fn button_arg(p: &Value) -> Result<Button, ActError> {
    match p.get("button").and_then(Value::as_str).unwrap_or("left") {
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "middle" => Ok(Button::Middle),
        b => Err(bad(format!("unknown button '{b}'"))),
    }
}

fn modifiers_arg(p: &Value) -> Result<Vec<u16>, ActError> {
    let mut out = Vec::new();
    if let Some(list) = p.get("modifiers") {
        let arr = list
            .as_array()
            .ok_or_else(|| bad("'modifiers' must be an array"))?;
        for m in arr {
            let name = m
                .as_str()
                .ok_or_else(|| bad("modifier names are strings"))?;
            match named_vk(name) {
                Some(vk) if is_modifier(vk) => out.push(vk),
                _ => return Err(bad(format!("'{name}' is not a modifier"))),
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Actions

pub const METHODS: &[&str] = &[
    "click",
    "mouse",
    "scroll",
    "drag",
    "type",
    "press_keys",
    "clipboard",
];

/// Runs one input method. Blocking; call from a blocking thread. Serialised process-wide.
pub fn run(method: &str, p: &Value) -> Result<Value, ActError> {
    init_dpi();
    let _one_at_a_time = action_lock();
    crate::safety::check()?; // queued behind another action when the kill switch fired
    match method {
        "click" => click(p),
        "mouse" => mouse_action(p),
        "scroll" => scroll(p),
        "drag" => drag(p),
        "type" => type_text(p),
        "press_keys" => press_keys(p),
        "clipboard" => clipboard(p),
        m => Err(bad(format!("'{m}' is not an input method"))),
    }
}

fn click(p: &Value) -> Result<Value, ActError> {
    let button = button_arg(p)?;
    let clicks = opt_int(p, "clicks", 1, 1, 3)?;
    let mods = modifiers_arg(p)?;
    let o = polite_args(p, true)?;
    if let Some(r) = p.get("ref").filter(|v| !v.is_null()) {
        let t = crate::perceive::resolve(r)?;
        // A plain left click is done through the element itself (no cursor, no keys).
        if button == Button::Left && clicks == 1 && mods.is_empty() {
            if let Some(v) = crate::perceive::click_background(&t)? {
                return Ok(v);
            }
        }
        let pt = t.center()?;
        return click_at(pt, Some(t.window), button, clicks, &mods, &o);
    }
    let pt = point(p, "x", "y")?;
    let window = window_arg(p, "window", false)?;
    click_at(pt, window, button, clicks, &mods, &o)
}

fn click_at(
    pt: POINT,
    window: Option<HWND>,
    button: Button,
    clicks: i64,
    mods: &[u16],
    o: &Polite,
) -> Result<Value, ActError> {
    wait_user_idle(o)?;
    if modifiers_held() {
        return Err(user_active("a modifier key is held"));
    }
    let start = cursor();
    let mut g = MouseGuard::begin(window)?;
    g.check_target(pt, None)?;
    g.move_to(pt)?;
    {
        let _mods = ModHold::press(mods)?;
        let (down, up) = button.flags();
        for i in 0..clicks {
            g.before_button(pt, None)?;
            send(&[mouse(down, 0, 0, 0)])?;
            sleep(Duration::from_millis(12));
            send(&[mouse(up, 0, 0, 0)])?;
            if i + 1 < clicks {
                sleep(Duration::from_millis(40));
            }
        }
    }
    restore_cursor(o, start, &g);
    Ok(json!({"at": [pt.x, pt.y], "clicks": clicks, "mode": "input"}))
}

fn mouse_action(p: &Value) -> Result<Value, ActError> {
    let window = window_arg(p, "window", false)?;
    let action = p
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("'action' is required"))?;
    let button = button_arg(p)?;
    // Validate every argument before reading live input state (guards).
    let single = match action {
        "move" | "down" | "up" => Some(point(p, "x", "y")?),
        "path" => None,
        a => return Err(bad(format!("unknown mouse action '{a}'"))),
    };
    if action == "path" {
        return mouse_path(p, window, button);
    }
    let pt = single.expect("validated above");
    let o = polite_args(p, false)?;
    wait_user_idle(&o)?;
    let start = cursor();
    let mut g = MouseGuard::begin(window)?;
    match action {
        "move" => {
            g.check_target(pt, None)?;
            g.move_to(pt)?;
            restore_cursor(&o, start, &g);
            Ok(json!({"at": [pt.x, pt.y], "mode": "input"}))
        }
        _ => {
            g.check_target(pt, None)?;
            g.move_to(pt)?;
            g.before_button(pt, None)?;
            let (d, u) = button.flags();
            send(&[mouse(if action == "down" { d } else { u }, 0, 0, 0)])?;
            if action == "up" {
                restore_cursor(&o, start, &g);
            }
            Ok(json!({"at": [pt.x, pt.y], "action": action, "mode": "input"}))
        }
    }
}

fn mouse_path(p: &Value, window: Option<HWND>, button: Button) -> Result<Value, ActError> {
    {
        {
            let pts: Vec<POINT> = p
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("'points' [[x,y],...] is required for path"))?
                .iter()
                .map(|v| {
                    match v.as_array().map(|a| {
                        (
                            a.first().and_then(Value::as_i64),
                            a.get(1).and_then(Value::as_i64),
                        )
                    }) {
                        Some((Some(x), Some(y))) => on_desktop(POINT {
                            x: x as i32,
                            y: y as i32,
                        }),
                        _ => Err(bad("each point is [x, y]")),
                    }
                })
                .collect::<Result<_, _>>()?;
            if pts.len() < 2 || pts.len() > 2000 {
                return Err(bad("'points' needs 2..2000 points"));
            }
            let hold = p.get("hold").and_then(Value::as_bool).unwrap_or(true);
            let step_ms = opt_int(p, "step_ms", 8, 0, 200)? as u64;
            let o = polite_args(p, false)?;
            wait_user_idle(&o)?;
            let start = cursor();
            let mut g = MouseGuard::begin(window)?;
            g.check_target(pts[0], None)?;
            g.move_to(pts[0])?;
            let mut b = if hold {
                g.before_button(pts[0], None)?;
                Some(ButtonHold::press(button)?)
            } else {
                None
            };
            for &pt in &pts[1..] {
                g.move_to(pt)?;
                sleep(Duration::from_millis(step_ms));
            }
            if let Some(b) = b.as_mut() {
                g.before_button(*pts.last().unwrap(), None)?;
                b.release()?;
            }
            restore_cursor(&o, start, &g);
            Ok(json!({"points": pts.len(), "mode": "input"}))
        }
    }
}

fn scroll(p: &Value) -> Result<Value, ActError> {
    let dy = opt_int(p, "dy", 0, -50, 50)? as i32;
    let dx = opt_int(p, "dx", 0, -50, 50)? as i32;
    let o = polite_args(p, true)?;
    let (pt, window) = if let Some(r) = p.get("ref").filter(|v| !v.is_null()) {
        let t = crate::perceive::resolve(r)?;
        if let Some(v) = crate::perceive::scroll_background(&t, dx, dy)? {
            return Ok(v);
        }
        if dx == 0 && dy == 0 {
            return Err(bad(
                "this element cannot be scrolled into view; give 'dy'/'dx' to wheel over it",
            ));
        }
        (t.center()?, Some(t.window))
    } else {
        (point(p, "x", "y")?, window_arg(p, "window", false)?)
    };
    if dx == 0 && dy == 0 {
        return Err(bad("give 'dy' and/or 'dx' (notches)"));
    }
    wait_user_idle(&o)?;
    let start = cursor();
    let mut g = MouseGuard::begin(window)?;
    g.check_target(pt, None)?;
    g.move_to(pt)?;
    for _ in 0..dy.abs() {
        g.before_button(pt, None)?;
        // positive dy scrolls down = negative wheel delta
        send(&[mouse(
            MOUSEEVENTF_WHEEL,
            0,
            0,
            (-dy.signum() * WHEEL_DELTA) as u32,
        )])?;
        sleep(Duration::from_millis(25));
    }
    for _ in 0..dx.abs() {
        g.before_button(pt, None)?;
        send(&[mouse(
            MOUSEEVENTF_HWHEEL,
            0,
            0,
            (dx.signum() * WHEEL_DELTA) as u32,
        )])?;
        sleep(Duration::from_millis(25));
    }
    restore_cursor(&o, start, &g);
    Ok(json!({"at": [pt.x, pt.y], "dx": dx, "dy": dy, "mode": "input"}))
}

/// A drag end point: `<k>_ref` (element centre, its window) or `<k>_x`/`<k>_y`.
fn drag_end(p: &Value, k: &str) -> Result<(POINT, Option<HWND>), ActError> {
    match p.get(format!("{k}_ref")).filter(|v| !v.is_null()) {
        Some(r) => {
            let t = crate::perceive::resolve(r)?;
            Ok((t.center()?, Some(t.window)))
        }
        None => Ok((point(p, &format!("{k}_x"), &format!("{k}_y"))?, None)),
    }
}

fn drag(p: &Value) -> Result<Value, ActError> {
    let (from, from_win) = drag_end(p, "from")?;
    let (to, to_win) = drag_end(p, "to")?;
    let window = window_arg(p, "window", false)?.or(from_win);
    let to_window = window_arg(p, "to_window", false)?.or(to_win).or(window);
    let steps = opt_int(p, "steps", 20, 1, 500)? as i32;
    let step_ms = opt_int(p, "step_ms", 12, 0, 200)? as u64;
    let hold_ms = opt_int(p, "hold_ms", 60, 0, 2000)? as u64;
    let button = button_arg(p)?;
    let o = polite_args(p, true)?;
    wait_user_idle(&o)?;
    if modifiers_held() {
        return Err(user_active("a modifier key is held"));
    }
    let start = cursor();
    let mut g = MouseGuard::begin(window)?;
    g.check_target(from, None)?;
    g.move_to(from)?;
    g.before_button(from, None)?;
    let mut hold = ButtonHold::press(button)?;
    sleep(Duration::from_millis(hold_ms));
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let pt = POINT {
            x: from.x + ((to.x - from.x) as f64 * t).round() as i32,
            y: from.y + ((to.y - from.y) as f64 * t).round() as i32,
        };
        g.move_to(pt)?; // aborts (and the Drop releases the button) if the user moves the mouse
        sleep(Duration::from_millis(step_ms));
    }
    sleep(Duration::from_millis(hold_ms));
    g.before_button(to, to_window)?;
    hold.release()?;
    restore_cursor(&o, start, &g);
    Ok(json!({"from": [from.x, from.y], "to": [to.x, to.y], "steps": steps, "mode": "input"}))
}

fn type_text(p: &Value) -> Result<Value, ActError> {
    let window = window_arg(p, "window", true)?.expect("required");
    let text = p
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("'text' is required"))?;
    if text.chars().count() > 10_000 {
        return Err(bad("'text' is limited to 10,000 characters per call"));
    }
    let submit = p.get("submit").and_then(Value::as_bool).unwrap_or(false);
    wait_user_idle(&polite_args(p, false)?)?;
    if modifiers_held() {
        return Err(user_active(
            "a modifier key is held (typed text would become shortcuts)",
        ));
    }
    foreground_is(window)?;
    let chars = text_events(text);
    let idle = InputIdle::open(window);
    // Chunks of 8 characters, each followed by waiting until the app has processed them
    // (adaptive: fast apps get the text at once, slow ones are not flooded). The foreground
    // is re-checked before every chunk, so nothing goes to a window the user switched to.
    for chunk in chars.chunks(8) {
        foreground_is(window)?;
        let evs: Vec<INPUT> = chunk.iter().flat_map(|u| char_events(u)).collect();
        send(&evs)?;
        match &idle {
            Some(w) => w.wait(250),
            None => sleep(Duration::from_millis(4 * chunk.len() as u64)),
        }
    }
    // Wait until the app has the whole text — before Enter (else e.g. a browser may search a
    // partial query) and before returning (so a following observe sees all of it): the app
    // goes idle, then the focused field's value stops changing (≤ 2 s).
    if let Some(w) = &idle {
        w.wait(1000);
    }
    let settled = settle_focused_value();
    if submit {
        foreground_is(window)?;
        send_char(&[INPUTDESC::Vk(VK_RETURN)])?;
        if let Some(w) = &idle {
            w.wait(1000);
        }
    }
    // Never echo the text back (it may be sensitive); report only the count.
    Ok(
        json!({"typed_chars": chars.len(), "submitted": submit, "settled": settled, "mode": "input"}),
    )
}

/// Polls the focused field's value length until it is unchanged twice in a row (40 ms apart,
/// at most 2 s). None when the focused element exposes no value (e.g. a terminal).
fn settle_focused_value() -> Option<bool> {
    let mut last = crate::perceive::focused_value_len()?;
    let t0 = std::time::Instant::now();
    let mut stable = 0;
    while t0.elapsed() < Duration::from_secs(2) {
        sleep(Duration::from_millis(40));
        let now = crate::perceive::focused_value_len()?;
        if now == last {
            stable += 1;
            if stable >= 2 {
                return Some(true);
            }
        } else {
            stable = 0;
            last = now;
        }
    }
    Some(false)
}

fn press_keys(p: &Value) -> Result<Value, ActError> {
    let window = window_arg(p, "window", true)?.expect("required");
    let list = p
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("'keys' (array of chords) is required"))?;
    if list.is_empty() || list.len() > 100 {
        return Err(bad("'keys' needs 1..100 chords"));
    }
    let chords: Vec<Chord> = list
        .iter()
        .map(|k| {
            k.as_str()
                .ok_or_else(|| bad("chords are strings"))
                .and_then(parse_chord)
        })
        .collect::<Result<_, _>>()?;
    wait_user_idle(&polite_args(p, false)?)?;
    if modifiers_held() {
        return Err(user_active("a modifier key is held"));
    }
    for c in &chords {
        foreground_is(window)?;
        let _mods = ModHold::press(&c.mods)?;
        send(&[key_event(c.key, false)])?;
        sleep(Duration::from_millis(10));
        send(&[key_event(c.key, true)])?;
        drop(_mods);
        sleep(Duration::from_millis(30));
    }
    Ok(json!({"chords": chords.len(), "mode": "input"}))
}

fn clipboard(p: &Value) -> Result<Value, ActError> {
    let op = p
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("'op' (get|set) is required"))?;
    open_clipboard()?;
    struct Close;
    impl Drop for Close {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }
    let _close = Close;
    match op {
        "get" => unsafe {
            let h = GetClipboardData(CF_UNICODETEXT);
            if h.is_null() {
                return Ok(json!({"text": Value::Null}));
            }
            let ptr = GlobalLock(h) as *const u16;
            if ptr.is_null() {
                return Err(err("INTERNAL", "could not lock clipboard data", ""));
            }
            let mut n = 0;
            while *ptr.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, n));
            GlobalUnlock(h);
            Ok(json!({"text": s}))
        },
        "set" => {
            let text = p
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("'text' is required for set"))?;
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
                if h.is_null() {
                    return Err(err("INTERNAL", "GlobalAlloc failed", ""));
                }
                let dst = GlobalLock(h) as *mut u16;
                std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                GlobalUnlock(h);
                EmptyClipboard();
                if SetClipboardData(CF_UNICODETEXT, h).is_null() {
                    GlobalFree(h);
                    return Err(err("INTERNAL", "SetClipboardData failed", ""));
                }
            }
            Ok(json!({"chars": text.chars().count()}))
        }
        o => Err(bad(format!("unknown clipboard op '{o}'"))),
    }
}

fn open_clipboard() -> Result<(), ActError> {
    for _ in 0..20 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            return Ok(());
        }
        sleep(Duration::from_millis(25));
    }
    Err(err(
        "TIMEOUT",
        "the clipboard is busy (another app holds it)",
        "Retry in a moment.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_struct_layout_matches_win32() {
        // INPUT is 40 bytes on x64 (type + padding + 32-byte union).
        assert_eq!(std::mem::size_of::<INPUT>(), 40);
        assert_eq!(std::mem::size_of::<MOUSEINPUT>(), 32);
        assert_eq!(std::mem::size_of::<KEYBDINPUT>(), 24);
    }

    #[test]
    fn normalise_maps_corners() {
        let (vx, vy, vw, vh) = (0, 0, 1920, 1080);
        assert_eq!(normalise(POINT { x: 0, y: 0 }, vx, vy, vw, vh), (0, 0));
        assert_eq!(
            normalise(POINT { x: 1919, y: 1079 }, vx, vy, vw, vh),
            (65535, 65535)
        );
        // negative origin (monitor left of primary)
        assert_eq!(
            normalise(POINT { x: -1920, y: 0 }, -1920, 0, 3840, 1080),
            (0, 0)
        );
    }

    #[test]
    fn chords_parse() {
        assert_eq!(
            parse_chord("Ctrl+S").unwrap(),
            Chord {
                mods: vec![VK_CONTROL],
                key: b'S' as u16
            }
        );
        assert_eq!(
            parse_chord("ctrl+shift+p").unwrap(),
            Chord {
                mods: vec![VK_CONTROL, VK_SHIFT],
                key: b'P' as u16
            }
        );
        assert_eq!(
            parse_chord("Alt+F4").unwrap(),
            Chord {
                mods: vec![VK_MENU],
                key: 0x73
            }
        );
        assert_eq!(
            parse_chord("Enter").unwrap(),
            Chord {
                mods: vec![],
                key: VK_RETURN
            }
        );
        assert_eq!(
            parse_chord("Win+Up").unwrap(),
            Chord {
                mods: vec![VK_LWIN],
                key: 0x26
            }
        );
        assert_eq!(parse_chord("Ctrl++").unwrap().mods, vec![VK_CONTROL]);
        assert!(parse_chord("Foo+S").is_err());
        assert!(parse_chord("Ctrl+NotAKey").is_err());
        assert!(parse_chord("").is_err());
    }

    #[test]
    fn text_becomes_unicode_packets_with_real_enter_and_tab() {
        let ev = text_events("a\tб\n🙂");
        assert_eq!(ev[0], vec![INPUTDESC::Unicode('a' as u16)]);
        assert_eq!(ev[1], vec![INPUTDESC::Vk(VK_TAB)]);
        assert_eq!(ev[2], vec![INPUTDESC::Unicode('б' as u16)]);
        assert_eq!(ev[3], vec![INPUTDESC::Vk(VK_RETURN)]);
        assert_eq!(ev[4].len(), 2, "emoji is a surrogate pair");
        assert_eq!(text_events("x\r\ny").len(), 3, "CR is dropped, LF is Enter");
    }

    #[test]
    fn own_injections_do_not_count_as_user_activity() {
        // never injected: plain idle time
        assert_eq!(user_idle(10_000, 9_000, 0), Some(1_000));
        // newest event is ours (stamped a little after SendInput returned)
        assert_eq!(user_idle(10_000, 9_020, 9_000), None);
        assert_eq!(user_idle(10_000, 8_000, 9_000), None);
        // the user acted after our injection
        assert_eq!(user_idle(10_000, 9_500, 9_000), Some(500));
        // tick counter wrap-around
        assert_eq!(user_idle(5, u32::MAX - 4, 0), Some(10));
        assert_eq!(user_idle(5, 2, u32::MAX - 100), Some(3));
        assert_eq!(
            user_idle(5, 2, u32::MAX - 10),
            None,
            "13 ms after our injection"
        );
    }

    #[test]
    fn arguments_are_validated_before_any_input() {
        assert_eq!(
            run("click", &json!({"x": 1})).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("click", &json!({"x": 1, "y": 1, "clicks": 9}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("type", &json!({"text": "hi"})).unwrap_err().code,
            "INVALID_ARGS",
            "window is required"
        );
        assert_eq!(
            run("type", &json!({"text": "hi", "window": 1}))
                .unwrap_err()
                .code,
            "WINDOW_NOT_FOUND"
        );
        assert_eq!(
            run("press_keys", &json!({"keys": ["Nope+X"], "window": 1}))
                .unwrap_err()
                .code,
            "WINDOW_NOT_FOUND"
        );
        assert_eq!(
            run("scroll", &json!({"x": 1, "y": 1})).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("mouse", &json!({"action": "teleport"}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            run("click", &json!({"x": -99999, "y": -99999}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
    }
}
