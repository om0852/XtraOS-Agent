//! Runs OS methods: the single dispatcher used by the pipe server and by `batch` (P2-M1,
//! ADR 0022). A batch runs many steps without AI round trips, but every step still passes the
//! policy engine and the kill switch on its own — "trusted" never means "unchecked".
use serde_json::{json, Value};

use crate::input::{self, ActError};
use crate::{perceive, safety, winmgr};

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "Check the tool schema in docs/04-mcp-tool-contract.md.",
    }
}

/// Runs one OS method (after `safety::authorize`). Admin windows go through the elevated
/// helper when it runs (ADR 0030): requests with helper refs directly, others when the core
/// itself hits ELEVATION_REQUIRED.
pub fn dispatch(method: &str, p: &Value) -> Result<Value, ActError> {
    if method == "elevated_helper" {
        return crate::helper::run(p);
    }
    if method == "record_session" {
        return crate::recorder::run(p);
    }
    if method == "sandbox" {
        return crate::sandbox::run(p);
    }
    if crate::helper::owns_refs(p) {
        return crate::helper::forward(method, p);
    }
    match dispatch_local(method, p) {
        Err(e)
            if e.code == "ELEVATION_REQUIRED"
                && crate::helper::ROUTABLE.contains(&method)
                && crate::helper::connected() =>
        {
            crate::helper::forward(method, p)
        }
        r => r,
    }
}

/// Runs one OS method in this process (the core, or inside the admin helper).
pub fn dispatch_local(method: &str, p: &Value) -> Result<Value, ActError> {
    if safety::METHODS.contains(&method) {
        safety::run(method, p)
    } else if winmgr::METHODS.contains(&method) {
        winmgr::run(method, p)
    } else if perceive::METHODS.contains(&method) {
        perceive::run(method, p)
    } else if input::METHODS.contains(&method) {
        input::run(method, p)
    } else if method == "browser" {
        crate::browser::run(p)
    } else if method == "volume" {
        crate::sysaudio::run(p)
    } else if method == "system" {
        crate::sysctl::run(p)
    } else if method == "touch" || method == "pen" {
        crate::touch::run(method, p)
    } else if method == "app_map" {
        crate::appmap::run(p)
    } else if method == "list_skills" {
        crate::skills::list(p)
    } else if method == "run_skill" {
        // Inside a batch / macro: built-in skills only (no macro recursion).
        if crate::skills::plan(p)?.is_macro {
            return Err(bad(
                "a macro skill cannot run inside a batch or another macro",
            ));
        }
        crate::skills::run(p, &mut |_| {})
    } else {
        Err(bad(format!("'{method}' is not an OS method")))
    }
}

/// Tools allowed inside a batch (ADR 0022). No nesting, no human questions, no images.
pub const BATCH_TOOLS: &[&str] = &[
    "volume",
    "system",
    "browser",
    "touch",
    "pen",
    "frame_diff",
    "click",
    "type",
    "press_keys",
    "scroll",
    "drag",
    "mouse",
    "set_value",
    "clipboard",
    "focus_window",
    "window_bounds",
    "launch_app",
    "close_window",
    "wait_for",
    "observe",
    "find",
    "read_text",
    "ocr",
    "app_map",
    "run_skill",
];

pub const MAX_STEPS: usize = 50;

/// One executed step, for the caller's action log.
pub struct StepLog<'a> {
    pub tool: &'a str,
    pub ok: bool,
    pub code: Option<&'static str>,
    pub policy: Option<&'static str>,
    pub ms: f64,
}

/// Validates every step first (nothing runs if one is invalid), then runs them in order.
pub fn batch(p: &Value, log: &mut dyn FnMut(StepLog)) -> Result<Value, ActError> {
    let steps = p
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("'steps' (array of {tool, args}) is required"))?;
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(bad(format!("'steps' needs 1..={MAX_STEPS} steps")));
    }
    let stop_on_error = p
        .get("stop_on_error")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut plan: Vec<(&str, Value)> = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        let tool = s
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(format!("step {}: 'tool' is required", i + 1)))?;
        if !BATCH_TOOLS.contains(&tool) {
            return Err(bad(format!(
                "step {}: '{tool}' cannot run inside a batch (allowed: {})",
                i + 1,
                BATCH_TOOLS.join(", ")
            )));
        }
        let args = match s.get("args") {
            None | Some(Value::Null) => json!({}),
            Some(a) if a.is_object() => a.clone(),
            Some(_) => return Err(bad(format!("step {}: 'args' must be an object", i + 1))),
        };
        plan.push((tool, args));
    }

    let mut results = Vec::with_capacity(plan.len());
    let mut failed = false;
    for (i, (tool, args)) in plan.iter().enumerate() {
        if failed {
            results.push(json!({"step": i + 1, "tool": tool, "skipped": true}));
            continue;
        }
        let t0 = std::time::Instant::now();
        let mut policy = None;
        // Recorded skills carry locators instead of refs (ADR 0036): resolve them now.
        let r = crate::recorder::resolve(args).and_then(|args| {
            safety::authorize(tool, &args).and_then(|a| {
                policy = Some(a.decision);
                dispatch(tool, &args)
            })
        });
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        match r {
            Ok(v) => {
                log(StepLog {
                    tool,
                    ok: true,
                    code: None,
                    policy,
                    ms,
                });
                results.push(json!({"step": i + 1, "tool": tool, "ok": true, "result": v}));
            }
            Err(e) => {
                let blocked = e.code == "BLOCKED_BY_POLICY";
                log(StepLog {
                    tool,
                    ok: false,
                    code: Some(e.code),
                    policy: if blocked { Some("blocked") } else { policy },
                    ms,
                });
                // After the kill switch nothing else may run, whatever stop_on_error says.
                failed = stop_on_error || e.code == "STOPPED";
                results.push(json!({"step": i + 1, "tool": tool, "ok": false,
                    "error": {"code": e.code, "message": e.message, "hint": e.hint}}));
            }
        }
    }
    let ok = results.iter().all(|r| r["ok"] == true);
    let done = results.iter().filter(|r| r["ok"] == true).count();
    Ok(json!({"ok": ok, "completed": done, "total": plan.len(), "steps": results}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(p: Value) -> Result<Value, ActError> {
        batch(&p, &mut |_| {})
    }

    #[test]
    fn invalid_batches_run_nothing() {
        assert_eq!(run(json!({})).unwrap_err().code, "INVALID_ARGS");
        assert_eq!(run(json!({"steps": []})).unwrap_err().code, "INVALID_ARGS");
        let many: Vec<Value> = (0..51).map(|_| json!({"tool": "find"})).collect();
        assert_eq!(
            run(json!({"steps": many})).unwrap_err().code,
            "INVALID_ARGS"
        );
        for bad_tool in [
            "batch",
            "ask_human",
            "wait_for_human",
            "screenshot",
            "status",
            "rm",
        ] {
            let e = run(
                json!({"steps": [{"tool": "find", "args": {"query": "x"}}, {"tool": bad_tool}]}),
            )
            .unwrap_err();
            assert_eq!(e.code, "INVALID_ARGS", "{bad_tool}");
            assert!(e.message.contains("step 2"), "{}", e.message);
        }
        assert_eq!(
            run(json!({"steps": [{"tool": "click", "args": [1, 2]}]}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
    }
}
