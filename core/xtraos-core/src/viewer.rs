//! Rules and activity viewer (P2-M7, ADR 0034): `xtraos-core.exe --viewer`, also opened from
//! the tray menu. Read only: it shows the effective policy and the latest action-log entries,
//! and offers to open policy.json (changes still need the user's approval in the core, ADR
//! 0026) and the log folder. It never writes anything and the log holds no secrets.
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

fn decision(d: Option<&Decision>) -> &'static str {
    match d {
        Some(Decision::Allow) => "allow",
        Some(Decision::Confirm) => "ask you first",
        Some(Decision::Deny) => "refused",
        None => "(default)",
    }
}

/// Local time "YYYY-MM-DD HH:MM:SS" for a unix-ms timestamp (Windows time-zone rules).
fn local_time(ms: u64) -> String {
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
    s.push_str("To change a rule: Open policy.json, edit, save. The core asks you to approve the change.\r\n");
    s.push_str(&format!(
        "\r\nRECENT ACTIVITY (newest first, up to {LOG_LINES})\r\n"
    ));
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
        let _ = MoveWindow(e, 8, 8, w - 16, ht - bar - 16, true);
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
                ID_REFRESH => set_report(h),
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
            1000,
            760,
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
}
