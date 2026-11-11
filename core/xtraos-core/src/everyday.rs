//! Everyday pack 2 (ADR 0068): window.snap, files.recent, files.read_text, disk.usage.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "See list_skills for the arguments." }
}

// ---------------------------------------------------------------------------------------------
// window.snap

pub const SNAP_POSITIONS: &[&str] =
    &["left", "right", "top", "bottom", "top-left", "top-right", "bottom-left", "bottom-right", "center", "maximize"];

/// The visible-frame rectangle (x, y, w, h) for a snap position inside a work area
/// (left, top, right, bottom). None for "maximize" (done by the window state) or unknown.
pub fn snap_rect(to: &str, work: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let (l, t, r, b) = work;
    let (w, h) = (r - l, b - t);
    let (hw, hh) = (w / 2, h / 2);
    Some(match to {
        "left" => (l, t, hw, h),
        "right" => (l + hw, t, w - hw, h),
        "top" => (l, t, w, hh),
        "bottom" => (l, t + hh, w, h - hh),
        "top-left" => (l, t, hw, hh),
        "top-right" => (l + hw, t, w - hw, hh),
        "bottom-left" => (l, t + hh, hw, h - hh),
        "bottom-right" => (l + hw, t + hh, w - hw, h - hh),
        "center" => {
            let (cw, ch) = (w * 6 / 10, h * 7 / 10);
            (l + (w - cw) / 2, t + (h - ch) / 2, cw, ch)
        }
        _ => return None,
    })
}

fn work_area(h: crate::input::HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::HWND as WHWND;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    unsafe {
        let m = MonitorFromWindow(WHWND(h as _), MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(m, &mut mi).as_bool().then(|| (mi.rcWork.left, mi.rcWork.top, mi.rcWork.right, mi.rcWork.bottom))
    }
}

pub fn window_snap(a: &Value) -> Result<Value, ActError> {
    let to = a.get("to").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    if !SNAP_POSITIONS.contains(&to.as_str()) {
        return Err(bad(format!("'to' must be one of: {}", SNAP_POSITIONS.join(", "))));
    }
    let win = a.get("window").filter(|v| !v.is_null()).ok_or_else(|| bad("'window' is required"))?;
    let h = crate::winmgr::resolve_window(win)?;
    let r = crate::winmgr::window_ref(h);
    if to == "maximize" {
        crate::winmgr::run("window_bounds", &json!({"window": r, "state": "maximize"}))?;
        return Ok(json!({"text": format!("ok {r} maximized")}));
    }
    // A maximized window is restored first (window_bounds does it when moving).
    let work = work_area(h).ok_or_else(|| bad("could not read the window's monitor"))?;
    let (x, y, w, hh) = snap_rect(&to, work).expect("checked above");
    crate::winmgr::run("window_bounds", &json!({"window": r, "x": x, "y": y, "w": w, "h": hh}))?;
    Ok(json!({"text": format!("ok {r} snapped {to}: {x},{y} {w}x{hh}"), "x": x, "y": y, "w": w, "h": hh}))
}

// ---------------------------------------------------------------------------------------------
// files.recent

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// The user's Desktop, Documents and Downloads, where they really are (OneDrive may move them).
pub fn user_folders() -> Vec<PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads};
    let mut v: Vec<PathBuf> = Vec::new();
    for id in [&FOLDERID_Desktop, &FOLDERID_Documents, &FOLDERID_Downloads] {
        if let Some(p) = known_folder(id) {
            // One folder inside another (e.g. Desktop under Documents) is walked once.
            if p.is_dir() && !v.iter().any(|x| p.starts_with(x) || x.starts_with(&p)) {
                v.push(p);
            }
        }
    }
    v
}

/// Newest first, changed since `since` (UTC ms), matching, at most `limit` (pure).
pub fn recent_pick(files: Vec<(PathBuf, u64, u64)>, since: u64, pattern: &str, limit: usize) -> Vec<(PathBuf, u64, u64)> {
    let mut v: Vec<_> = files
        .into_iter()
        .filter(|(p, _, t)| {
            *t >= since
                && p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    crate::skills::wildcard(pattern, &n) && !crate::skills::unfinished(&n) && !n.starts_with("~$")
                })
        })
        .collect();
    v.sort_by(|a, b| b.2.cmp(&a.2));
    v.truncate(limit);
    v
}

pub fn files_recent(a: &Value) -> Result<Value, ActError> {
    let days = a.get("days").and_then(Value::as_u64).unwrap_or(2).clamp(1, 30);
    let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100) as usize;
    let pattern = a.get("pattern").and_then(Value::as_str).unwrap_or("*").to_string();
    let now = crate::routines::utc_now_ms().max(0) as u64;
    let since = now.saturating_sub(days * 86_400_000);
    let mut all = Vec::new();
    let mut complete = true;
    let folders = user_folders();
    for f in &folders {
        let (files, cut) = crate::skills::walk_files(f, std::time::Duration::from_secs(4));
        complete &= !cut;
        all.extend(files);
    }
    let picked = recent_pick(all, since, &pattern, limit);
    let mut text = String::new();
    for (p, size, t) in &picked {
        let local = crate::routines::show_time(*t as i64 + (crate::routines::local_now_ms() - now as i64));
        text.push_str(&format!("{local}  {:>9}  {}\n", human_size(*size), p.display()));
    }
    if picked.is_empty() {
        text.push_str(&format!("no files changed in the last {days} day(s)\n"));
    }
    text.push_str(&format!(
        "-- in {}{}",
        folders.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join("; "),
        if complete { "" } else { " (time limit reached: not every folder was searched)" }
    ));
    Ok(json!({
        "text": text,
        "files": picked.iter().map(|(p, s, _)| json!({"path": p.display().to_string(), "bytes": s})).collect::<Vec<_>>(),
        "complete": complete,
    }))
}

pub fn human_size(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f >= K * K * K {
        format!("{:.1} GB", f / (K * K * K))
    } else if f >= K * K {
        format!("{:.1} MB", f / (K * K))
    } else if f >= K {
        format!("{:.1} KB", f / K)
    } else {
        format!("{n} B")
    }
}

// ---------------------------------------------------------------------------------------------
// files.read_text

const MAX_TEXT_FILE: u64 = 5 * 1024 * 1024;

/// Text from file bytes (pure): UTF-8 (BOM optional), UTF-16 LE/BE with BOM. None: binary.
pub fn decode_text(b: &[u8]) -> Option<String> {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).ok();
    }
    let utf16 = |rest: &[u8], le: bool| -> Option<String> {
        if rest.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = rest.chunks(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).ok()
    };
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    // No BOM: UTF-8 without NUL bytes (a NUL means a binary file or BOM-less UTF-16).
    if b.iter().take(8192).any(|&c| c == 0) {
        return None;
    }
    String::from_utf8(b.to_vec()).ok()
}

pub fn files_read_text(a: &Value) -> Result<Value, ActError> {
    let raw = a.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("'path' is required"))?;
    let path = crate::skills::checked(raw)?;
    let max = a.get("max_chars").and_then(Value::as_u64).unwrap_or(20_000).clamp(100, 200_000) as usize;
    let md = std::fs::metadata(&path).map_err(|_| bad(format!("{} does not exist", path.display())))?;
    if !md.is_file() {
        return Err(bad(format!("{} is not a file", path.display())));
    }
    if md.len() > MAX_TEXT_FILE {
        return Err(bad(format!("{} is {} (read_text reads ≤ 5 MB)", path.display(), human_size(md.len()))));
    }
    let bytes = std::fs::read(&path).map_err(|e| ActError {
        code: "NEEDS_HUMAN",
        message: format!("could not read {}: {e}", path.display()),
        hint: "The file may be open in another program or not allowed for this user.",
    })?;
    let text = decode_text(&bytes).ok_or_else(|| {
        bad(format!(
            "{} is not a text file (binary). Office files: office.word_read / excel_read / ppt_read",
            path.display()
        ))
    })?;
    let chars = text.chars().count();
    let cut = chars > max;
    let body: String = text.chars().take(max).collect();
    Ok(json!({
        "text": format!("{} ({} lines, {chars} characters){}\n{body}", path.display(), text.lines().count(), if cut { format!("; first {max} characters") } else { String::new() }),
        "chars": chars,
        "truncated": cut,
    }))
}

// ---------------------------------------------------------------------------------------------
// disk.usage

/// LOW: under 10 % or under 5 GB free (pure).
pub fn disk_low(total: u64, free: u64) -> bool {
    total > 0 && (free * 10 < total || free < 5 * 1024 * 1024 * 1024)
}

pub fn disk_line(drive: &str, total: u64, free: u64) -> String {
    let used = if total > 0 { 100 - free * 100 / total } else { 0 };
    format!(
        "{drive}  {} free of {} ({used} % used){}",
        human_size(free),
        human_size(total),
        if disk_low(total, free) { "  LOW" } else { "" }
    )
}

pub fn disk_usage() -> Result<Value, ActError> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetDiskFreeSpaceExW(root: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    let mut lines = Vec::new();
    let mut drives = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        if unsafe { GetDriveTypeW(w.as_ptr()) } != DRIVE_FIXED {
            continue;
        }
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
            continue;
        }
        let name = root.trim_end_matches('\\').to_string();
        lines.push(disk_line(&name, total, avail));
        drives.push(json!({"drive": name, "total": total, "free": avail, "low": disk_low(total, avail)}));
    }
    Ok(json!({"text": if lines.is_empty() { "no fixed drives found".to_string() } else { lines.join("\n") }, "drives": drives}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_rectangles() {
        let work = (0, 0, 1920, 1040);
        assert_eq!(snap_rect("left", work), Some((0, 0, 960, 1040)));
        assert_eq!(snap_rect("right", work), Some((960, 0, 960, 1040)));
        assert_eq!(snap_rect("bottom", work), Some((0, 520, 1920, 520)));
        assert_eq!(snap_rect("top-right", work), Some((960, 0, 960, 520)));
        assert_eq!(snap_rect("bottom-left", work), Some((0, 520, 960, 520)));
        assert_eq!(snap_rect("center", work), Some((384, 156, 1152, 728)));
        // a second monitor to the left, odd size: the halves still cover it exactly
        let m2 = (-1281, 10, 0, 1011);
        let (lx, _, lw, _) = snap_rect("left", m2).unwrap();
        let (rx, _, rw, _) = snap_rect("right", m2).unwrap();
        assert_eq!((lx, lx + lw, rx, rx + rw), (-1281, -641, -641, 0));
        assert_eq!(snap_rect("maximize", work), None);
        assert_eq!(snap_rect("diagonal", work), None);
    }

    #[test]
    fn text_is_decoded_and_binary_refused() {
        assert_eq!(decode_text(b"hello\nworld").as_deref(), Some("hello\nworld"));
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'h', b'i']).as_deref(), Some("hi"));
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]).as_deref(), Some("hi"));
        assert_eq!(decode_text(&[0xFE, 0xFF, 0, b'h', 0, b'i']).as_deref(), Some("hi"));
        assert_eq!(decode_text("नमस्ते".as_bytes()).as_deref(), Some("नमस्ते"));
        assert_eq!(decode_text(&[0x89, b'P', b'N', b'G', 0, 0, 1]), None, "binary");
        assert_eq!(decode_text(&[0xC3, 0x28]), None, "invalid UTF-8");
    }

    #[test]
    fn disk_low_rule_and_text() {
        const G: u64 = 1024 * 1024 * 1024;
        assert!(disk_low(500 * G, 40 * G), "8 %");
        assert!(disk_low(32 * G, 4 * G), "< 5 GB");
        assert!(!disk_low(500 * G, 100 * G));
        assert!(!disk_low(0, 0));
        assert_eq!(disk_line("C:", 500 * G, 40 * G), "C:  40.0 GB free of 500.0 GB (92 % used)  LOW");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
    }

    #[test]
    fn recent_files_newest_first() {
        let f = |n: &str, t: u64| (PathBuf::from(format!("C:/u/{n}")), 10, t);
        let v = vec![f("old.docx", 100), f("new.docx", 900), f("mid.pdf", 500), f("dl.zip.crdownload", 950), f("~$new.docx", 960)];
        let got: Vec<String> = recent_pick(v.clone(), 400, "*", 10).iter().map(|x| x.0.display().to_string()).collect();
        assert_eq!(got, vec!["C:/u/new.docx", "C:/u/mid.pdf"], "no old, unfinished or Office lock files");
        assert_eq!(recent_pick(v.clone(), 0, "*.docx", 10).len(), 2);
        assert_eq!(recent_pick(v, 0, "*", 1).len(), 1);
    }
}
