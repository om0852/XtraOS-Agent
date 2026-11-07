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

/// A prepared step: the macro step, new skill arguments (typed text), notes.
pub type Step = (Value, Vec<(String, Value)>, Vec<String>);

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
            *g = Some(Recording {
                name: name.clone(),
                started: Instant::now(),
                steps: Vec::new(),
                args: Map::new(),
                notes: Vec::new(),
                full: false,
            });
            Ok(
                json!({"text": format!("recording '{name}': your successful actions are saved as steps (max {MAX_STEPS}, 30 min); stop with record_session stop")}),
            )
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
                return Ok(
                    json!({"text": format!("recording '{}' had no steps: nothing saved", r.name)}),
                );
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
            Ok(json!({"text": msg, "path": path.display().to_string(), "steps": r.steps.len()}))
        }
        o => Err(bad(format!(
            "unknown op '{o}' (start, stop, status, discard)"
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
    for k in ["ref", "from_ref", "to_ref", "scope_ref"] {
        let Some(v) = args.get(k).cloned() else {
            continue;
        };
        if let Some((h, role, name, password)) = perceive::ref_locator(&v) {
            if password {
                return None; // never record anything aimed at a password field
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
    Some((
        json!({"tool": method, "args": Value::Object(args)}),
        new_args,
        notes,
    ))
}

/// Keeps a prepared step after its action succeeded.
pub fn commit(step: Step) {
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
    let (s, a, n) = step;
    r.steps.push(s);
    for (k, v) in a {
        r.args.insert(k, v);
    }
    r.notes.extend(n);
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
}
