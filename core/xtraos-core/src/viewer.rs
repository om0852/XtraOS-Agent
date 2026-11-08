//! Rules and activity window (P2-M7/M8, ADR 0034/0036): `xtraos-core.exe --viewer`, also
//! opened from the tray menu. It shows the effective policy and the latest action-log entries
//! (the log holds no secrets) and lets the USER edit the rules. Saving writes policy.json;
//! the core applies it only after the user approves the listed changes in the core's own
//! dialog (ADR 0026), and hard rules still win. The agent cannot drive this window: every
//! xtraos-core.exe window is off limits to it (ADR 0036).
use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND as WHWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CLEARTYPE_QUALITY, DEFAULT_CHARSET, FW_NORMAL, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::policy::{Class, Decision, Policy};

pub const TITLE: &str = "XtraOS Agent — rules and activity";
const LOG_LINES: usize = 200;
const ID_TEXT: i32 = 10;
const ID_REFRESH: i32 = 11;
const ID_OPEN_POLICY: i32 = 12;
const ID_OPEN_LOGS: i32 = 13;
const ID_CLASS_BASE: i32 = 100;
const ID_APPS: i32 = 200;
const ID_RATE: i32 = 201;
const ID_SAVE: i32 = 202;
const ID_STATUS: i32 = 203;
const ID_TASK_MAX: i32 = 204;
const ID_TASK_IDLE: i32 = 205;
const EDITOR_W: i32 = 420;
const ROW_H: i32 = 27;

// ---------------------------------------------------------------------------------------------
// Editing (pure, unit-tested)

const CHOICES: [(&str, Decision); 3] = [
    ("allow", Decision::Allow),
    ("ask first", Decision::Confirm),
    ("refuse", Decision::Deny),
];

fn decision_word(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Confirm => "confirm",
        Decision::Deny => "deny",
    }
}

/// App rules as editable lines `process.exe = allow|confirm|deny`, sorted.
pub fn apps_to_text(p: &Policy) -> String {
    let mut apps: Vec<_> = p.apps.iter().collect();
    apps.sort_by(|a, b| a.0.cmp(b.0));
    apps.iter()
        .map(|(a, d)| format!("{a} = {}", decision_word(**d)))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Parses the app-rule lines; blank lines and `#` comments are skipped.
pub fn parse_apps(text: &str) -> Result<Vec<(String, Decision)>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (a, d) = l
            .split_once('=')
            .ok_or_else(|| format!("line {}: use process.exe = allow|confirm|deny", i + 1))?;
        let a = a.trim().to_lowercase();
        if a.is_empty() || a.contains(['\\', '/', ' ']) || !a.ends_with(".exe") {
            return Err(format!(
                "line {}: '{a}' is not a process name like app.exe",
                i + 1
            ));
        }
        let d = Decision::parse(d.trim())
            .ok_or_else(|| format!("line {}: decision must be allow, confirm or deny", i + 1))?;
        out.push((a, d));
    }
    Ok(out)
}

/// New policy.json text: `existing` with classes, apps and rate replaced; other keys kept.
pub fn updated_json(
    existing: Option<&str>,
    classes: &[(Class, Decision)],
    apps: &[(String, Decision)],
    rate: u32,
    task: (u32, u32),
) -> Result<String, String> {
    if !(5..=1_000_000).contains(&task.0) {
        return Err("max actions per task must be 5..1000000".into());
    }
    if !(1..=1440).contains(&task.1) {
        return Err("task pause must be 1..1440 minutes".into());
    }
    if classes
        .iter()
        .any(|(c, d)| *c == Class::CredentialEntry && *d != Decision::Deny)
    {
        return Err("credential_entry is always refused (the user types passwords)".into());
    }
    if classes
        .iter()
        .any(|(c, d)| *c == Class::Purchase && *d == Decision::Allow)
    {
        return Err("purchase can be at most 'ask first'".into());
    }
    if !(10..=10_000).contains(&rate) {
        return Err("actions per minute must be 10..10000".into());
    }
    let mut v: serde_json::Value = match existing {
        Some(t) if !t.trim().is_empty() => serde_json::from_str(t)
            .map_err(|e| format!("policy.json is not valid JSON ({e}); fix or delete it first"))?,
        _ => serde_json::json!({}),
    };
    let o = v
        .as_object_mut()
        .ok_or("policy.json must contain a JSON object")?;
    let mut cs = serde_json::Map::new();
    for (c, d) in classes {
        cs.insert(c.name().into(), serde_json::json!(decision_word(*d)));
    }
    let mut ap = serde_json::Map::new();
    for (a, d) in apps {
        ap.insert(a.clone(), serde_json::json!(decision_word(*d)));
    }
    o.insert("classes".into(), serde_json::Value::Object(cs));
    o.insert("apps".into(), serde_json::Value::Object(ap));
    o.insert("rate_per_minute".into(), serde_json::json!(rate));
    o.insert("max_actions_per_task".into(), serde_json::json!(task.0));
    o.insert("task_idle_minutes".into(), serde_json::json!(task.1));
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

fn decision(d: Option<&Decision>) -> &'static str {
    match d {
        Some(Decision::Allow) => "allow",
        Some(Decision::Confirm) => "ask you first",
        Some(Decision::Deny) => "refused",
        None => "(default)",
    }
}

/// Local time "YYYY-MM-DD HH:MM:SS" for a unix-ms timestamp (Windows time-zone rules).
pub(crate) fn local_time(ms: u64) -> String {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        y: u16,
        mo: u16,
        dow: u16,
        d: u16,
        h: u16,
        mi: u16,
        s: u16,
        ms: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn FileTimeToLocalFileTime(ft: *const u64, local: *mut u64) -> i32;
        fn FileTimeToSystemTime(ft: *const u64, st: *mut SystemTime) -> i32;
    }
    let ft: u64 = (ms + 11_644_473_600_000) * 10_000;
    let mut local = 0u64;
    let mut st = SystemTime::default();
    unsafe {
        if FileTimeToLocalFileTime(&ft, &mut local) == 0
            || FileTimeToSystemTime(&local, &mut st) == 0
        {
            return ms.to_string();
        }
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        st.y, st.mo, st.d, st.h, st.mi, st.s
    )
}

/// The newest `n` action-log lines (newest first) from `<dir>/actions-*.jsonl`.
pub fn recent_actions(dir: &Path, n: usize) -> Vec<String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|f| f.to_str())
                        .is_some_and(|f| f.starts_with("actions-") && f.ends_with(".jsonl"))
                })
                .collect()
        })
        .unwrap_or_default();
    // actions-<unix-ms>-<pid>.jsonl: newest first by the timestamp in the name.
    files.sort_by_key(|p| {
        std::cmp::Reverse(
            p.file_name()
                .and_then(|f| f.to_str())
                .and_then(|f| f.split('-').nth(1))
                .and_then(|t| t.parse::<u64>().ok())
                .unwrap_or(0),
        )
    });
    let mut out = Vec::new();
    for f in files {
        let Ok(t) = std::fs::read_to_string(&f) else {
            continue;
        };
        for line in t.lines().rev() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let mut s = format!(
                "{}  {:<22} {}",
                local_time(v["ts_ms"].as_u64().unwrap_or(0)),
                v["method"].as_str().unwrap_or("?"),
                if v["ok"] == true {
                    "ok".to_string()
                } else {
                    v["code"].as_str().unwrap_or("failed").to_string()
                }
            );
            if let Some(p) = v["policy"].as_str() {
                s.push_str(&format!("  [{}]", crate::perceive::clip(p, 60)));
            }
            if let Some(c) = v["client"].as_str() {
                s.push_str(&format!("  ({c})"));
            }
            out.push(s);
            if out.len() >= n {
                return out;
            }
        }
    }
    out
}

/// The whole report shown in the window.
pub fn report(data_dir: &Path) -> String {
    let path = data_dir.join("policy.json");
    let exists = path.exists();
    let p = Policy::load(&path);
    let mut s = String::new();
    s.push_str("YOUR RULES\r\n");
    s.push_str(&format!(
        "Policy file: {} {}\r\n",
        path.display(),
        if exists {
            ""
        } else {
            "(not created yet: the defaults below apply)"
        }
    ));
    for w in &p.warnings {
        s.push_str(&format!("  warning: {w}\r\n"));
    }
    s.push_str("\r\nWhat the agent may do:\r\n");
    for c in Class::ALL {
        s.push_str(&format!(
            "  {:<20} {}\r\n",
            c.name(),
            decision(p.classes.get(&c))
        ));
    }
    let mut apps: Vec<_> = p.apps.iter().collect();
    apps.sort_by(|a, b| a.0.cmp(b.0));
    s.push_str("\r\nApps:\r\n");
    for (a, d) in apps {
        s.push_str(&format!("  {:<28} {}\r\n", a, decision(Some(d))));
    }
    s.push_str(&format!(
        "\r\nSpeed limit: {} actions per minute\r\n",
        p.rate_per_minute
    ));
    s.push_str(&format!(
        "Long tasks: after {} actions without a {}-minute pause, you are asked to continue\r\n",
        p.max_actions_per_task, p.task_idle_minutes
    ));
    s.push_str("To change a rule: Open policy.json, edit, save. The core asks you to approve the change.\r\n");
    s.push_str(&format!(
        "\r\nRECENT ACTIVITY (newest first, up to {LOG_LINES})\r\n"
    ));
    let v = crate::actionlog::verify_dir(&data_dir.join("logs"));
    if v.files > 0 {
        s.push_str(&format!(
            "Log integrity: {}\r\n",
            if v.ok {
                v.message.clone()
            } else {
                format!("BROKEN: {}", v.message)
            }
        ));
    }
    let acts = recent_actions(&data_dir.join("logs"), LOG_LINES);
    if acts.is_empty() {
        s.push_str("  (nothing logged yet)\r\n");
    }
    for a in acts {
        s.push_str(&format!("  {a}\r\n"));
    }
    s
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn ctl_text(h: WHWND, id: i32) -> String {
    let Ok(c) = GetDlgItem(Some(h), id) else {
        return String::new();
    };
    let n = GetWindowTextLengthW(c);
    let mut buf = vec![0u16; n as usize + 1];
    let got = GetWindowTextW(c, &mut buf);
    String::from_utf16_lossy(&buf[..got as usize])
}

unsafe fn set_ctl_text(h: WHWND, id: i32, s: &str) {
    if let Ok(c) = GetDlgItem(Some(h), id) {
        let t = wide(s);
        let _ = SetWindowTextW(c, PCWSTR(t.as_ptr()));
    }
}

/// Puts the current policy into the editor controls.
unsafe fn fill_editor(h: WHWND) {
    let p = Policy::load(&crate::pairing::data_dir().join("policy.json"));
    for (i, c) in Class::ALL.iter().enumerate() {
        let d = p.classes.get(c).copied().unwrap_or(Decision::Confirm);
        let idx = CHOICES.iter().position(|(_, x)| *x == d).unwrap_or(1);
        if let Ok(cb) = GetDlgItem(Some(h), ID_CLASS_BASE + i as i32) {
            SendMessageW(cb, CB_SETCURSEL, Some(WPARAM(idx)), Some(LPARAM(0)));
        }
    }
    set_ctl_text(h, ID_APPS, &apps_to_text(&p));
    set_ctl_text(h, ID_RATE, &p.rate_per_minute.to_string());
    set_ctl_text(h, ID_TASK_MAX, &p.max_actions_per_task.to_string());
    set_ctl_text(h, ID_TASK_IDLE, &p.task_idle_minutes.to_string());
}

/// Save: validate, merge into policy.json, report. The core asks the user before applying.
unsafe fn save(h: WHWND) {
    let mut classes = Vec::new();
    for (i, c) in Class::ALL.iter().enumerate() {
        let Ok(cb) = GetDlgItem(Some(h), ID_CLASS_BASE + i as i32) else {
            continue;
        };
        let sel = SendMessageW(cb, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0;
        if let Some((_, d)) = CHOICES.get(sel.max(0) as usize) {
            classes.push((*c, *d));
        }
    }
    let status = match parse_apps(&ctl_text(h, ID_APPS)).and_then(|apps| {
        let rate = ctl_text(h, ID_RATE)
            .trim()
            .parse::<u32>()
            .map_err(|_| "actions per minute must be a number".to_string())?;
        let num = |id: i32, what: &str| {
            ctl_text(h, id)
                .trim()
                .parse::<u32>()
                .map_err(|_| format!("{what} must be a number"))
        };
        let task = (
            num(ID_TASK_MAX, "max actions per task")?,
            num(ID_TASK_IDLE, "task pause")?,
        );
        let path = crate::pairing::data_dir().join("policy.json");
        let existing = std::fs::read_to_string(&path).ok();
        let text = updated_json(existing.as_deref(), &classes, &apps, rate, task)?;
        if existing.as_deref() == Some(text.as_str()) {
            return Ok("No changes.".to_string());
        }
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("could not save: {e}"))?;
        Ok("Saved. Before its next action, XtraOS shows you the changes and applies them only if you approve.".to_string())
    }) {
        Ok(s) => s,
        Err(e) => format!("Not saved: {e}"),
    };
    set_ctl_text(h, ID_STATUS, &status);
    set_report(h);
}

unsafe fn set_report(h: WHWND) {
    let text = wide(&report(&crate::pairing::data_dir()));
    if let Ok(edit) = GetDlgItem(Some(h), ID_TEXT) {
        let _ = SetWindowTextW(edit, PCWSTR(text.as_ptr()));
    }
}

unsafe fn layout(h: WHWND) {
    let mut r = RECT::default();
    let _ = GetClientRect(h, &mut r);
    let (w, ht) = (r.right - r.left, r.bottom - r.top);
    let bar = 40;
    if let Ok(e) = GetDlgItem(Some(h), ID_TEXT) {
        let _ = MoveWindow(e, EDITOR_W + 16, 8, w - EDITOR_W - 24, ht - bar - 16, true);
    }
    for (i, id) in [ID_REFRESH, ID_OPEN_POLICY, ID_OPEN_LOGS]
        .iter()
        .enumerate()
    {
        if let Ok(b) = GetDlgItem(Some(h), *id) {
            let _ = MoveWindow(b, 8 + i as i32 * 170, ht - bar, 160, 30, true);
        }
    }
}

fn open(target: &Path) {
    let p = wide(&target.to_string_lossy());
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(p.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SHOW_WINDOW_CMD(1),
        );
    }
}

unsafe extern "system" fn proc(h: WHWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            let edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | WINDOW_STYLE(
                        (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | ES_AUTOHSCROLL) as u32,
                    ),
                0,
                0,
                100,
                100,
                Some(h),
                Some(HMENU(ID_TEXT as _)),
                Some(inst.into()),
                None,
            );
            if let Ok(e) = edit {
                let font = CreateFontW(
                    -15,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    Default::default(),
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Consolas"),
                );
                SendMessageW(
                    e,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
                SendMessageW(e, 0x00C5, Some(WPARAM(0)), Some(LPARAM(0))); // EM_LIMITTEXT: no limit
            }
            // Editor: one list per class, app rules, rate, Save.
            let gui = windows::Win32::Graphics::Gdi::GetStockObject(
                windows::Win32::Graphics::Gdi::DEFAULT_GUI_FONT,
            );
            let child = |class: PCWSTR, text: &str, style: WINDOW_STYLE, x, y, cw, ch, id: i32| {
                let t = wide(text);
                let c = CreateWindowExW(
                    if class == w!("EDIT") {
                        WS_EX_CLIENTEDGE
                    } else {
                        WINDOW_EX_STYLE(0)
                    },
                    class,
                    PCWSTR(t.as_ptr()),
                    WS_CHILD | WS_VISIBLE | style,
                    x,
                    y,
                    cw,
                    ch,
                    Some(h),
                    Some(HMENU(id as _)),
                    Some(inst.into()),
                    None,
                );
                if let Ok(c) = c {
                    SendMessageW(c, WM_SETFONT, Some(WPARAM(gui.0 as usize)), Some(LPARAM(1)));
                }
                c
            };
            for (i, c) in Class::ALL.iter().enumerate() {
                let y = 8 + i as i32 * ROW_H;
                let _ = child(
                    w!("STATIC"),
                    c.name(),
                    WINDOW_STYLE(0),
                    8,
                    y + 3,
                    170,
                    20,
                    -1,
                );
                if let Ok(cb) = child(
                    w!("COMBOBOX"),
                    "",
                    WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                    180,
                    y,
                    230,
                    200,
                    ID_CLASS_BASE + i as i32,
                ) {
                    for (label, _) in CHOICES {
                        let t = wide(label);
                        SendMessageW(
                            cb,
                            CB_ADDSTRING,
                            Some(WPARAM(0)),
                            Some(LPARAM(t.as_ptr() as isize)),
                        );
                    }
                }
            }
            let y0 = 8 + Class::ALL.len() as i32 * ROW_H + 6;
            let _ = child(
                w!("STATIC"),
                "Apps (one per line: process.exe = allow / confirm / deny)",
                WINDOW_STYLE(0),
                8,
                y0,
                402,
                18,
                -1,
            );
            let _ = child(
                w!("EDIT"),
                "",
                WS_VSCROLL
                    | WS_TABSTOP
                    | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
                8,
                y0 + 20,
                402,
                150,
                ID_APPS,
            );
            let _ = child(
                w!("STATIC"),
                "Actions per minute",
                WINDOW_STYLE(0),
                8,
                y0 + 182,
                170,
                20,
                -1,
            );
            let _ = child(
                w!("EDIT"),
                "",
                WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                180,
                y0 + 179,
                80,
                22,
                ID_RATE,
            );
            let _ = child(
                w!("STATIC"),
                "Max actions per task",
                WINDOW_STYLE(0),
                8,
                y0 + 210,
                170,
                20,
                -1,
            );
            let _ = child(
                w!("EDIT"),
                "",
                WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                180,
                y0 + 207,
                80,
                22,
                ID_TASK_MAX,
            );
            let _ = child(
                w!("STATIC"),
                "Task ends after a pause of (min)",
                WINDOW_STYLE(0),
                8,
                y0 + 238,
                170,
                20,
                -1,
            );
            let _ = child(
                w!("EDIT"),
                "",
                WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                180,
                y0 + 235,
                80,
                22,
                ID_TASK_IDLE,
            );
            let _ = child(
                w!("BUTTON"),
                "Save rules",
                WS_TABSTOP,
                8,
                y0 + 268,
                160,
                30,
                ID_SAVE,
            );
            let _ = child(
                w!("STATIC"),
                "",
                WINDOW_STYLE(0),
                8,
                y0 + 306,
                402,
                56,
                ID_STATUS,
            );
            fill_editor(h);
            for (id, label) in [
                (ID_REFRESH, w!("Refresh")),
                (ID_OPEN_POLICY, w!("Open policy.json")),
                (ID_OPEN_LOGS, w!("Open log folder")),
            ] {
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    label,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    10,
                    10,
                    Some(h),
                    Some(HMENU(id as _)),
                    Some(inst.into()),
                    None,
                );
            }
            set_report(h);
            layout(h);
            LRESULT(0)
        }
        WM_SIZE => {
            layout(h);
            LRESULT(0)
        }
        WM_COMMAND => {
            let dir = crate::pairing::data_dir();
            match (wp.0 & 0xffff) as i32 {
                ID_REFRESH => {
                    fill_editor(h);
                    set_report(h);
                }
                ID_SAVE => save(h),
                ID_OPEN_POLICY => {
                    // Notepad on the file (it offers to create it if missing). Saving it
                    // changes nothing until the user approves the change in the core.
                    let p = wide(&dir.join("policy.json").to_string_lossy());
                    let _ = ShellExecuteW(
                        None,
                        w!("open"),
                        w!("notepad.exe"),
                        PCWSTR(p.as_ptr()),
                        PCWSTR::null(),
                        SHOW_WINDOW_CMD(1),
                    );
                }
                ID_OPEN_LOGS => {
                    let logs = dir.join("logs");
                    if logs.is_dir() {
                        open(&logs);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

/// Runs the viewer window until it is closed. Returns the exit code.
pub fn run_viewer() -> i32 {
    crate::input::init_dpi();
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("XtraOSViewer");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: inst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH((15 + 1) as _), // COLOR_BTNFACE + 1
            ..Default::default()
        };
        RegisterClassW(&wc);
        let title = wide(TITLE);
        let Ok(h) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1240,
            820,
            None,
            None,
            Some(inst.into()),
            None,
        ) else {
            return 1;
        };
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_shows_rules_and_newest_actions_first() {
        let d = std::env::temp_dir().join(format!("xtraos-viewer-{}", std::process::id()));
        let logs = d.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            d.join("policy.json"),
            r#"{"classes": {"delete": "deny"}, "apps": {"notepad.exe": "confirm"}}"#,
        )
        .unwrap();
        std::fs::write(
            logs.join("actions-1000-1.jsonl"),
            "{\"ts_ms\":1000,\"method\":\"old_call\",\"ok\":true,\"ms\":1.0}\n",
        )
        .unwrap();
        std::fs::write(
            logs.join("actions-2000-1.jsonl"),
            "{\"ts_ms\":2000,\"method\":\"click\",\"ok\":false,\"code\":\"BLOCKED_BY_POLICY\",\"ms\":1.0,\"policy\":\"blocked\",\"client\":\"claude\"}\n",
        )
        .unwrap();
        let r = report(&d);
        assert!(r.contains("delete               refused"), "{r}");
        assert!(r.contains("notepad.exe"), "{r}");
        assert!(r.contains("message_send         ask you first"), "{r}");
        let click = r.find("click").unwrap();
        let old = r.find("old_call").unwrap();
        assert!(click < old, "newest first");
        assert!(r.contains("BLOCKED_BY_POLICY  [blocked]  (claude)"), "{r}");
        assert_eq!(recent_actions(&logs, 1).len(), 1);
        let empty = report(&d.join("nope"));
        assert!(empty.contains("not created yet") && empty.contains("nothing logged yet"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn editor_merges_and_refuses_hard_rule_changes() {
        let apps = parse_apps("# mine\r\nNotepad.exe = confirm\r\n\r\nslack.exe=deny").unwrap();
        assert_eq!(
            apps,
            vec![
                ("notepad.exe".into(), Decision::Confirm),
                ("slack.exe".into(), Decision::Deny)
            ]
        );
        assert!(parse_apps("notepad = allow").is_err(), "needs .exe");
        assert!(parse_apps("C:\\x\\a.exe = allow").is_err(), "no paths");
        assert!(parse_apps("a.exe = maybe").is_err());
        let classes = vec![
            (Class::Delete, Decision::Deny),
            (Class::CredentialEntry, Decision::Deny),
        ];
        let t = updated_json(
            Some(r#"{"note": "kept", "classes": {"input": "allow"}}"#),
            &classes,
            &apps,
            300,
            (2000, 10),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["note"], "kept", "unknown keys stay");
        assert_eq!(v["classes"]["delete"], "deny");
        assert_eq!(v["apps"]["slack.exe"], "deny");
        assert_eq!(v["rate_per_minute"], 300);
        assert_eq!(v["max_actions_per_task"], 2000);
        assert!(updated_json(None, &[], &[], 300, (2, 10)).is_err());
        let p = Policy::from_json(&t);
        assert_eq!(p.classes.get(&Class::Delete), Some(&Decision::Deny));
        assert!(updated_json(
            None,
            &[(Class::CredentialEntry, Decision::Allow)],
            &[],
            300,
            (2000, 10)
        )
        .is_err());
        assert!(updated_json(
            None,
            &[(Class::Purchase, Decision::Allow)],
            &[],
            300,
            (2000, 10)
        )
        .is_err());
        assert!(updated_json(None, &[], &[], 5, (2000, 10)).is_err());
        assert!(
            updated_json(Some("{broken"), &[], &[], 300, (2000, 10)).is_err(),
            "never overwrite a broken file"
        );
        let round = apps_to_text(&Policy::from_json(&t));
        assert!(round.contains("slack.exe = deny"), "{round}");
    }
}
