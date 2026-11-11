//! `record_session` (P3-M2, ADR 0036): records the agent's successful actions as a macro skill
//! that `run_skill` replays. Refs, window handles and screen points are session-specific, so
//! they are stored as locators and resolved again at replay:
//!   ref    → {"locate": {"process", "title", "role", "name"}}   (exact role + name)
//!   window → {"locate_window": {"process", "title"}}
//!   x / y  → {"at": {"process", "title", "dx", "dy", "axis"}}   (relative to that window)
//! A locator that does not resolve fails the step (ELEMENT_NOT_FOUND / WINDOW_NOT_FOUND); it
//! never guesses. Typed text becomes a skill argument (default = what was typed); password
//! fields are never recorded. Every replayed step passes the policy again.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::input::ActError;
use crate::{perceive, winmgr};

/// Same as a batch (a macro replays as one batch).
pub const MAX_STEPS: usize = crate::exec::MAX_STEPS;
const MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// A prepared step: the macro step, new skill arguments (typed text), notes, and for a bug
/// report (ADR 0053) the action's window and a one-line description.
pub type Step = (Value, Vec<(String, Value)>, Vec<String>, Evidence);

#[derive(Clone, Debug, Default)]
pub struct Evidence {
    win: usize,
    what: String,
}

/// One line of a bug report (ADR 0053).
#[derive(Clone, Debug)]
pub struct Entry {
    pub what: String,
    pub ok: bool,
    pub error: Option<String>,
    pub shot: Option<String>,
    pub secs: f64,
    pub note: bool,
}

struct Report {
    dir: std::path::PathBuf,
    entries: Vec<Entry>,
    apps: Vec<String>,
    started_local: String,
    /// The last window an action of this recording used: a step or note without a window of
    /// its own is pictured with it — never with whatever else is in front (the user's apps).
    last_win: usize,
}

const RECORDABLE: &[&str] = &[
    "click",
    "type",
    "press_keys",
    "scroll",
    "drag",
    "mouse",
    "set_value",
    "focus_window",
    "launch_app",
    "close_window",
    "window_bounds",
    "clipboard",
    "run_skill",
    "browser",
    "wait_for",
    "touch",
    "pen",
    "volume",
];

struct Recording {
    name: String,
    started: Instant,
    steps: Vec<Value>,
    args: Map<String, Value>,
    notes: Vec<String>,
    full: bool,
    report: Option<Report>,
}

static REC: Mutex<Option<Recording>> = Mutex::new(None);

fn rec() -> std::sync::MutexGuard<'static, Option<Recording>> {
    REC.lock().unwrap_or_else(|e| e.into_inner())
}

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "See record_session in docs/04-mcp-tool-contract.md.",
    }
}

fn not_found(code: &'static str, m: impl Into<String>) -> ActError {
    ActError {
        code,
        message: m.into(),
        hint: "The recorded window or element is not there now: open it first, or re-record.",
    }
}

/// True while recording (the overlay shows it).
pub fn active() -> bool {
    rec()
        .as_ref()
        .is_some_and(|r| !r.full && r.started.elapsed() < MAX_AGE)
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !n.starts_with('.')
}

/// The `record_session` method.
pub fn run(p: &Value) -> Result<Value, ActError> {
    match p.get("op").and_then(Value::as_str).unwrap_or("status") {
        "start" => {
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if !valid_name(&name) {
                return Err(bad(
                    "'name': 1-64 characters, letters, digits, - _ . (e.g. \"daily-report\")",
                ));
            }
            let mut g = rec();
            if g.as_ref().is_some_and(|r| r.started.elapsed() < MAX_AGE) {
                return Err(bad(format!(
                    "already recording '{}': stop or discard it first",
                    g.as_ref().map(|r| r.name.as_str()).unwrap_or("")
                )));
            }
            // ADR 0053: `report: true` also writes a bug report (steps, screenshots, notes).
            let report = if p.get("report").and_then(Value::as_bool).unwrap_or(false) {
                let local = local_time();
                let dir = reports_dir().join(format!("{name}-{}", local.1));
                std::fs::create_dir_all(&dir)
                    .map_err(|e| bad(format!("report folder {}: {e}", dir.display())))?;
                Some(Report {
                    dir,
                    entries: Vec::new(),
                    apps: Vec::new(),
                    started_local: local.0,
                    last_win: 0,
                })
            } else {
                None
            };
            let report_note = report
                .as_ref()
                .map(|r| format!("; bug report in {}", r.dir.display()))
                .unwrap_or_default();
            *g = Some(Recording {
                name: name.clone(),
                started: Instant::now(),
                steps: Vec::new(),
                args: Map::new(),
                notes: Vec::new(),
                full: false,
                report,
            });
            Ok(
                json!({"text": format!("recording '{name}': your successful actions are saved as steps (max {MAX_STEPS}, 30 min){report_note}; stop with record_session stop")}),
            )
        }
        "note" => {
            let text = p
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| bad("'text' is required: what you observed"))?;
            if text.chars().count() > 1000 {
                return Err(bad("'text' is limited to 1000 characters"));
            }
            let (dir, secs, n, last) = {
                let g = rec();
                let r = g.as_ref().ok_or_else(|| bad("not recording"))?;
                let rep = r
                    .report
                    .as_ref()
                    .ok_or_else(|| bad("this recording has no report: start with report: true"))?;
                (rep.dir.clone(), r.started.elapsed().as_secs_f64(), rep.entries.len() + 1, rep.last_win)
            };
            let shot = if last != 0 && winmgr::exists(last as crate::input::HWND) {
                screenshot_to(last, &dir, n)
            } else {
                None
            };
            push_entry(Entry {
                what: text.to_string(),
                ok: true,
                error: None,
                shot,
                secs,
                note: true,
            });
            Ok(json!({"text": "note added to the bug report"}))
        }
        "status" => Ok(match rec().as_ref() {
            Some(r) => {
                json!({"text": format!("recording '{}': {} step(s), {} s{}", r.name, r.steps.len(), r.started.elapsed().as_secs(),
                if r.full { " (full: stop to save)" } else { "" }), "recording": true, "steps": r.steps.len()})
            }
            None => json!({"text": "not recording", "recording": false}),
        }),
        "discard" => {
            let was = rec().take().is_some();
            Ok(json!({"text": if was { "recording discarded" } else { "not recording" }}))
        }
        "stop" => {
            let r = rec().take().ok_or_else(|| bad("not recording"))?;
            if r.steps.is_empty() {
                let report = write_report(&r, None);
                let mut msg = format!("recording '{}' had no successful steps: no skill saved", r.name);
                if let Some(p) = &report {
                    msg.push_str(&format!("\nbug report: {p}"));
                }
                return Ok(json!({"text": msg, "report": report}));
            }
            let overwrite = p.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
            let dir = crate::pairing::data_dir().join("skills");
            std::fs::create_dir_all(&dir).map_err(|e| bad(format!("skills folder: {e}")))?;
            let path = dir.join(format!("{}.json", r.name));
            if path.exists() && !overwrite {
                *rec() = Some(r); // keep it so the agent can retry with overwrite or discard
                return Err(bad(
                    "a skill with that name exists: stop with overwrite: true, or discard",
                ));
            }
            let def = json!({
                "name": r.name,
                "description": format!("recorded {} step(s)", r.steps.len()),
                "recorded": true,
                "args": r.args,
                "steps": r.steps,
            });
            let text = serde_json::to_string_pretty(&def).map_err(|e| bad(e.to_string()))?;
            std::fs::write(&path, text).map_err(|e| bad(format!("could not save: {e}")))?;
            let mut msg = format!(
                "saved skill '{}' ({} step(s), args: {}); replay with run_skill {{name: \"{}\"}}",
                r.name,
                r.steps.len(),
                if r.args.is_empty() {
                    "none".to_string()
                } else {
                    r.args.keys().cloned().collect::<Vec<_>>().join(", ")
                },
                r.name
            );
            for n in &r.notes {
                msg.push_str(&format!("\nnote: {n}"));
            }
            let report = write_report(&r, Some(&r.name));
            if let Some(p) = &report {
                msg.push_str(&format!("\nbug report: {p}"));
            }
            Ok(
                json!({"text": msg, "path": path.display().to_string(), "steps": r.steps.len(), "report": report}),
            )
        }
        o => Err(bad(format!(
            "unknown op '{o}' (start, stop, status, discard, note)"
        ))),
    }
}

fn window_locator(h: crate::input::HWND) -> Value {
    json!({"process": winmgr::process_name(winmgr::pid_of(h)), "title": winmgr::title(h)})
}

/// Builds the step for an action that is about to run (locators taken now, while refs and
/// windows still exist). None = not recording or not recordable.
pub fn prepare(method: &str, p: &Value) -> Option<Step> {
    if !active() || !RECORDABLE.contains(&method) {
        return None;
    }
    if method == "browser" && p["action"].as_str().is_some_and(crate::browser::is_observe) {
        return None;
    }
    if method == "run_skill" && crate::skills::plan(p).map(|pl| pl.is_macro).unwrap_or(true) {
        return None; // macros cannot nest; built-in skills are recorded
    }
    let mut args = p.as_object().cloned().unwrap_or_default();
    let mut new_args = Vec::new();
    let mut notes = Vec::new();
    // Bug report evidence (ADR 0053): the action's window and its target element.
    let mut win: crate::input::HWND = std::ptr::null_mut();
    let mut target: Option<(&'static str, String)> = None;
    for k in ["ref", "from_ref", "to_ref", "scope_ref"] {
        let Some(v) = args.get(k).cloned() else {
            continue;
        };
        if let Some((h, role, name, password)) = perceive::ref_locator(&v) {
            if password {
                return None; // never record anything aimed at a password field
            }
            if win.is_null() {
                win = h;
                target = Some((role, name.clone()));
            }
            if name.is_empty() {
                notes.push(format!(
                    "{method}: the {k} element has no name; replay may not find it"
                ));
            }
            let mut l = window_locator(h);
            l["role"] = json!(role);
            l["name"] = json!(name);
            args.insert(k.into(), json!({"locate": l}));
        }
    }
    for k in ["window", "to_window"] {
        if let Some(h) = args.get(k).and_then(|v| winmgr::resolve_window(v).ok()) {
            if win.is_null() {
                win = h;
            }
            args.insert(k.into(), json!({"locate_window": window_locator(h)}));
        }
    }
    for (kx, ky) in [("x", "y"), ("from_x", "from_y"), ("to_x", "to_y")] {
        let (Some(x), Some(y)) = (
            args.get(kx).and_then(Value::as_i64),
            args.get(ky).and_then(Value::as_i64),
        ) else {
            continue;
        };
        if method == "window_bounds" {
            continue; // window position, not a point on screen
        }
        let h = winmgr::root_at(crate::input::POINT {
            x: x as i32,
            y: y as i32,
        });
        if h.is_null() {
            continue;
        }
        if win.is_null() {
            win = h;
        }
        let f = winmgr::frame(h);
        let base = window_locator(h);
        for (key, axis, val, origin) in [(kx, "x", x, f.left), (ky, "y", y, f.top)] {
            let mut a = base.clone();
            a["axis"] = json!(axis);
            a["d"] = json!(val as i32 - origin);
            args.insert(key.into(), json!({"at": a}));
        }
    }
    for k in ["text", "value"] {
        if (method == "type" || method == "set_value" || method == "clipboard")
            && args.get(k).is_some_and(Value::is_string)
        {
            let n = format!(
                "{k}{}",
                rec()
                    .as_ref()
                    .map(|r| r.args.len() + new_args.len() + 1)
                    .unwrap_or(1)
            );
            new_args.push((
                n.clone(),
                json!({"default": args[k].clone(), "required": false}),
            ));
            args.insert(k.into(), json!(format!("${{{n}}}")));
        }
    }
    // No window of its own (e.g. an unknown ref): the report uses the recording's last window.
    let title = if win.is_null() { String::new() } else { winmgr::title(win) };
    let what = describe(method, p, target.as_ref().map(|(r, n)| (*r, n.as_str())), &title);
    Some((
        json!({"tool": method, "args": Value::Object(args)}),
        new_args,
        notes,
        Evidence {
            win: win as usize,
            what,
        },
    ))
}

/// One line for the bug report (pure): `click Button "Save" in "report.txt - Notepad"`.
pub fn describe(method: &str, p: &Value, target: Option<(&str, &str)>, title: &str) -> String {
    let clip = |s: &str, n| crate::perceive::clip(s, n);
    let mut s = method.replace('_', " ");
    match method {
        "type" => {
            let t = p["text"].as_str().unwrap_or("");
            s = format!("type {} chars \"{}\"", t.chars().count(), clip(t, 40));
            if p["submit"] == true {
                s.push_str(" + Enter");
            }
        }
        "set_value" => {
            let t = p["value"].as_str().unwrap_or("");
            s = format!("set value \"{}\"", clip(t, 40));
        }
        "press_keys" => {
            let k: Vec<String> = match &p["keys"] {
                Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                Value::String(x) => vec![x.clone()],
                _ => Vec::new(),
            };
            s = format!("press {}", k.join(", "));
        }
        "launch_app" => s = format!("launch {}", clip(p["app"].as_str().unwrap_or("?"), 60)),
        "run_skill" => s = format!("run skill {}", p["name"].as_str().unwrap_or("?")),
        "browser" => s = format!("browser {}", p["action"].as_str().unwrap_or("?")),
        "click" if p.get("x").is_some() && target.is_none() => {
            s = format!("click at ({}, {})", p["x"], p["y"]);
        }
        _ => {}
    }
    if let Some((role, name)) = target {
        s.push_str(&format!(" {role} \"{}\"", clip(name, 50)));
    }
    if !title.is_empty() && method != "launch_app" {
        s.push_str(&format!(" in \"{}\"", clip(title, 50)));
    }
    s
}

/// Keeps a prepared step after its action succeeded.
pub fn commit(step: Step) {
    report_step(&step.3, None);
    let mut g = rec();
    let Some(r) = g.as_mut() else { return };
    if r.full || r.started.elapsed() >= MAX_AGE {
        return;
    }
    if r.steps.len() >= MAX_STEPS {
        r.full = true;
        r.notes
            .push(format!("stopped recording at {MAX_STEPS} steps"));
        return;
    }
    let (s, a, n, _) = step;
    r.steps.push(s);
    for (k, v) in a {
        r.args.insert(k, v);
    }
    r.notes.extend(n);
}

/// An action that failed while recording a bug report: reported, never replayed (ADR 0053).
pub fn failed(step: Step, e: &ActError) {
    report_step(&step.3, Some(format!("{}: {}", e.code, e.message)));
}

// ---------------------------------------------------------------------------------------------
// Bug report (ADR 0053)

fn push_entry(e: Entry) {
    if let Some(rep) = rec().as_mut().and_then(|r| r.report.as_mut()) {
        rep.entries.push(e);
    }
}

/// Adds one step (ok or failed) with a screenshot of its window, taken after the UI settled.
fn report_step(ev: &Evidence, error: Option<String>) {
    let Some((dir, secs, n, last)) = rec().as_ref().and_then(|r| {
        r.report
            .as_ref()
            .filter(|_| !r.full && r.started.elapsed() < MAX_AGE)
            .map(|rep| (rep.dir.clone(), r.started.elapsed().as_secs_f64(), rep.entries.len() + 1, rep.last_win))
    }) else {
        return;
    };
    std::thread::sleep(Duration::from_millis(300));
    let alive = |w: usize| w != 0 && winmgr::exists(w as crate::input::HWND);
    let win = if alive(ev.win) {
        ev.win
    } else if alive(last) {
        last
    } else {
        0 // nothing of this recording to picture: no screenshot rather than another app
    };
    let shot = screenshot_to(win, &dir, n);
    let app = if win == 0 {
        String::new()
    } else {
        let h = win as crate::input::HWND;
        format!(
            "{} \"{}\"",
            winmgr::process_name(winmgr::pid_of(h)),
            crate::perceive::clip(&winmgr::title(h), 60)
        )
    };
    if let Some(rep) = rec().as_mut().and_then(|r| r.report.as_mut()) {
        if win != 0 {
            rep.last_win = win;
        }
        if !app.is_empty() && !rep.apps.contains(&app) {
            rep.apps.push(app);
        }
        rep.entries.push(Entry {
            what: ev.what.clone(),
            ok: error.is_none(),
            error,
            shot,
            secs,
            note: false,
        });
    }
}

/// `step-NN.png` of a window in the report folder; None when it cannot be captured
/// (XtraOS's own windows, elevated windows, a closed window).
fn screenshot_to(win: usize, dir: &std::path::Path, n: usize) -> Option<String> {
    if win == 0 {
        return None;
    }
    let (img, _, _) = crate::capture::grab(&json!({"window": winmgr::window_ref(win as crate::input::HWND)})).ok()?;
    let (w, h, rgb) = crate::capture::downscale_rgb(&img, 0.6);
    let png = crate::capture::png_rgb(w, h, &rgb).ok()?;
    let name = format!("step-{n:02}.png");
    std::fs::write(dir.join(&name), png).ok()?;
    Some(name)
}

/// The user's Documents\XtraOS reports (%TEMP% when Documents is not available).
fn reports_dir() -> std::path::PathBuf {
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    let docs = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KNOWN_FOLDER_FLAG(0), None) }
        .ok()
        .and_then(|p| {
            let s = unsafe { p.to_string() }.ok();
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
            s
        })
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir());
    match docs {
        Some(d) => d.join("XtraOS reports"),
        None => std::env::temp_dir().join("XtraOS reports"),
    }
}

/// ("2026-11-11 14:05", "20261111-140512") in local time.
fn local_time() -> (String, String) {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (
        format!("{:04}-{:02}-{:02} {:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute),
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        ),
    )
}

fn windows_version() -> String {
    #[repr(C)]
    struct OsVersionInfoW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(v: *mut OsVersionInfoW) -> i32;
    }
    let mut v = OsVersionInfoW {
        size: std::mem::size_of::<OsVersionInfoW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        csd: [0; 128],
    };
    if unsafe { RtlGetVersion(&mut v) } != 0 {
        return "Windows".into();
    }
    let name = if v.major == 10 && v.build >= 22000 { "Windows 11" } else { "Windows" };
    format!("{name} {}.{}.{}", v.major, v.minor, v.build)
}

/// report.md (pure).
pub fn render_report(
    name: &str,
    started: &str,
    duration_s: f64,
    versions: &str,
    apps: &[String],
    entries: &[Entry],
    skill: Option<&str>,
) -> String {
    let steps: Vec<&Entry> = entries.iter().filter(|e| !e.note).collect();
    let notes: Vec<&Entry> = entries.iter().filter(|e| e.note).collect();
    let failed = steps.iter().filter(|e| !e.ok).count();
    let mut s = format!(
        "# Bug report: {name}\n\n- Recorded: {started} ({:.0} s)\n- {versions}\n- Steps: {} ({failed} failed)\n",
        duration_s,
        steps.len()
    );
    if !apps.is_empty() {
        s.push_str(&format!("- Apps: {}\n", apps.join("; ")));
    }
    s.push_str("\n> Screenshots show what was on screen. Look at them before sharing this report.\n");
    s.push_str("\n## Steps to reproduce\n\n");
    if steps.is_empty() {
        s.push_str("(no actions were recorded)\n");
    }
    for (i, e) in steps.iter().enumerate() {
        s.push_str(&format!(
            "{}. {} — {} ({:.1} s)\n",
            i + 1,
            e.what,
            match &e.error {
                None => "ok".to_string(),
                Some(err) => format!("**FAILED**: {err}"),
            },
            e.secs
        ));
        if let Some(p) = &e.shot {
            s.push_str(&format!("\n   ![step {}]({p})\n\n", i + 1));
        }
    }
    s.push_str("\n## Observations\n\n");
    if notes.is_empty() {
        s.push_str("(none)\n");
    }
    for e in &notes {
        s.push_str(&format!("- {} ({:.1} s)\n", e.what, e.secs));
        if let Some(p) = &e.shot {
            s.push_str(&format!("\n  ![note]({p})\n\n"));
        }
    }
    s.push_str("\n## Replay\n\n");
    match skill {
        Some(n) => s.push_str(&format!(
            "The successful steps are saved as the XtraOS skill `{n}`: `run_skill {{\"name\": \"{n}\"}}` replays them.\n"
        )),
        None => s.push_str("No replay skill was saved (no successful steps).\n"),
    }
    s
}

/// Writes report.md for a finished recording; returns its path.
fn write_report(r: &Recording, skill: Option<&str>) -> Option<String> {
    let rep = r.report.as_ref()?;
    let md = render_report(
        &r.name,
        &rep.started_local,
        r.started.elapsed().as_secs_f64(),
        &format!("XtraOS {} · {}", env!("CARGO_PKG_VERSION"), windows_version()),
        &rep.apps,
        &rep.entries,
        skill,
    );
    let path = rep.dir.join("report.md");
    std::fs::write(&path, md).ok()?;
    Some(path.display().to_string())
}

// ---------------------------------------------------------------------------------------------
// Replay: locators → refs / windows / coordinates

fn find_window(l: &Value) -> Result<crate::input::HWND, ActError> {
    let process = l["process"].as_str().unwrap_or("").to_lowercase();
    let title = l["title"].as_str().unwrap_or("");
    let wins: Vec<_> = winmgr::listable_windows()
        .into_iter()
        .filter(|&h| winmgr::process_name(winmgr::pid_of(h)).eq_ignore_ascii_case(&process))
        .collect();
    wins.iter()
        .copied()
        .find(|&h| winmgr::title(h) == title)
        .or_else(|| wins.first().copied()) // same app, newest window (titles change: "file - App")
        .ok_or_else(|| {
            not_found(
                "WINDOW_NOT_FOUND",
                format!("no {process} window (recorded: \"{title}\")"),
            )
        })
}

/// Replaces locators in a step's args with live refs / window refs / coordinates.
pub fn resolve(args: &Value) -> Result<Value, ActError> {
    let Some(o) = args.as_object() else {
        return Ok(args.clone());
    };
    let mut out = o.clone();
    for (k, v) in o {
        if let Some(l) = v.get("locate") {
            let h = find_window(l)?;
            let r = perceive::locate_element(
                h,
                l["role"].as_str().unwrap_or(""),
                l["name"].as_str().unwrap_or(""),
            )?;
            out.insert(k.clone(), json!(r));
        } else if let Some(l) = v.get("locate_window") {
            out.insert(k.clone(), json!(winmgr::window_ref(find_window(l)?)));
        } else if let Some(a) = v.get("at") {
            let h = find_window(a)?;
            let f = winmgr::frame(h);
            let d = a["d"].as_i64().unwrap_or(0) as i32;
            let n = if a["axis"] == "y" {
                f.top + d
            } else {
                f.left + d
            };
            out.insert(k.clone(), json!(n));
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_plain_args() {
        assert!(valid_name("daily-report_2.v1"));
        assert!(!valid_name("../evil"));
        assert!(!valid_name(".hidden"));
        assert!(!valid_name(""));
        // Args without locators pass through unchanged.
        let a = json!({"keys": ["Ctrl+S"], "idle_ms": 0});
        assert_eq!(resolve(&a).unwrap(), a);
        // A locator for a window that does not exist fails cleanly.
        let l =
            json!({"window": {"locate_window": {"process": "no-such-app-xyz.exe", "title": "x"}}});
        assert_eq!(resolve(&l).unwrap_err().code, "WINDOW_NOT_FOUND");
        assert!(prepare("click", &json!({})).is_none(), "not recording");
    }

    // ADR 0053: bug report lines and Markdown.
    #[test]
    fn report_lines_and_markdown() {
        assert_eq!(
            describe("click", &json!({"ref": "e1"}), Some(("Button", "Save")), "a.txt - Notepad"),
            "click Button \"Save\" in \"a.txt - Notepad\""
        );
        assert_eq!(
            describe("type", &json!({"text": "hello world", "submit": true}), Some(("Edit", "Name")), ""),
            "type 11 chars \"hello world\" + Enter Edit \"Name\""
        );
        assert_eq!(describe("press_keys", &json!({"keys": "Ctrl+S"}), None, ""), "press Ctrl+S");
        assert_eq!(describe("launch_app", &json!({"app": "notepad"}), None, "Untitled"), "launch notepad");
        assert_eq!(describe("click", &json!({"x": 10, "y": 20}), None, ""), "click at (10, 20)");
        let long = "x".repeat(100);
        assert!(describe("type", &json!({"text": long}), None, "").len() < 70, "typed text is clipped");

        let e = |what: &str, ok: bool, note: bool| Entry {
            what: what.into(),
            ok,
            error: (!ok).then(|| "STALE_REF: e9 no longer exists".to_string()),
            shot: Some("step-01.png".into()),
            secs: 1.5,
            note,
        };
        let md = render_report(
            "save-bug",
            "2026-11-11 10:00",
            12.0,
            "XtraOS 0.1.0 · Windows 11 10.0.26200",
            &["notepad.exe \"a.txt\"".into()],
            &[e("click Button \"Save\"", true, false), e("click e9", false, false), e("total shows 41, expected 42", true, true)],
            Some("save-bug"),
        );
        assert!(md.starts_with("# Bug report: save-bug"));
        assert!(md.contains("- Steps: 2 (1 failed)"), "{md}");
        assert!(md.contains("2. click e9 — **FAILED**: STALE_REF"), "{md}");
        assert!(md.contains("## Observations\n\n- total shows 41, expected 42"), "{md}");
        assert!(md.contains("![step 1](step-01.png)"), "{md}");
        assert!(md.contains("run_skill {\"name\": \"save-bug\"}"), "{md}");
        assert!(md.contains("Look at them before sharing"), "{md}");
        let none = render_report("x", "t", 1.0, "v", &[], &[], None);
        assert!(none.contains("(no actions were recorded)") && none.contains("No replay skill"));
    }
}
