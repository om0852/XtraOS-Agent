//! Task checkpoints (P7-M6, ADR 0057): `task.save / load / list / done`. Long tasks keep their
//! progress in XtraOS's own data folder so another agent or session can resume. Never
//! credentials: a state with a secret-looking key is refused.
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::input::ActError;

const MAX_STATE: usize = 32 * 1024;
const MAX_TASKS: usize = 200;
const MAX_AGE_MS: u64 = 30 * 24 * 3600 * 1000;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "task.save {id, state, note?} · task.load {id} · task.list · task.done {id}" }
}

fn not_found(id: &str) -> ActError {
    ActError { code: "ELEMENT_NOT_FOUND", message: format!("no checkpoint '{id}'"), hint: "task.list shows the saved checkpoints." }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 1–64 letters, digits, - _ . (pure).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The first key anywhere in `v` that looks like a credential (pure).
pub fn secret_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let n: String = k.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                let hit = ["password", "passwd", "secret", "token", "apikey", "otp", "cvv", "pin", "passcode"]
                    .iter()
                    .any(|s| n == *s || n.ends_with(s) && (*s != "pin" && *s != "otp") || n.starts_with("password"));
                if hit {
                    return Some(k.clone());
                }
                if let Some(s) = secret_key(x) {
                    return Some(s);
                }
            }
            None
        }
        Value::Array(a) => a.iter().find_map(secret_key),
        _ => None,
    }
}

fn path_of(base: &Path, id: &str) -> PathBuf {
    base.join(format!("{id}.json"))
}

fn age_text(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        s if s < 60 => format!("{s} s ago"),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

fn id_arg(a: &Value) -> Result<String, ActError> {
    let id = a.get("id").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !valid_id(&id) {
        return Err(bad("'id': 1-64 letters, digits, - _ . (e.g. \"invoices-nov\")"));
    }
    Ok(id)
}

/// The skill, against a given folder (tests use a temp folder).
pub fn run_in(base: &Path, op: &str, a: &Value) -> Result<Value, ActError> {
    std::fs::create_dir_all(base).map_err(|e| bad(format!("checkpoint folder: {e}")))?;
    match op {
        "task.save" => {
            let id = id_arg(a)?;
            let state = a.get("state").cloned().ok_or_else(|| bad("'state' is required (any JSON: what is done, what is next)"))?;
            if let Some(k) = secret_key(&state) {
                return Err(ActError {
                    code: "BLOCKED_BY_POLICY",
                    message: format!("the state has a key \"{k}\" that looks like a credential; checkpoints never store secrets"),
                    hint: "Leave credentials out; the user enters them when needed.",
                });
            }
            let size = serde_json::to_vec(&state).map(|v| v.len()).unwrap_or(0);
            if size > MAX_STATE {
                return Err(bad(format!("the state is {size} bytes; at most {MAX_STATE}")));
            }
            let note = a.get("note").and_then(Value::as_str).unwrap_or("").chars().take(200).collect::<String>();
            let p = path_of(base, &id);
            let saves = if p.exists() {
                std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["saves"].as_u64()).unwrap_or(0)
            } else {
                let count = std::fs::read_dir(base).map(|d| d.flatten().count()).unwrap_or(0);
                if count >= MAX_TASKS {
                    return Err(bad(format!("{MAX_TASKS} checkpoints exist; task.done the finished ones first")));
                }
                0
            };
            let doc = json!({"id": id, "note": note, "state": state, "saved_ms": now_ms(), "saves": saves + 1});
            std::fs::write(&p, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| bad(format!("could not save: {e}")))?;
            Ok(json!({"text": format!("saved checkpoint '{id}' ({size} bytes, save #{})", saves + 1), "id": id}))
        }
        "task.load" => {
            let id = id_arg(a)?;
            let t = std::fs::read_to_string(path_of(base, &id)).map_err(|_| not_found(&id))?;
            let v: Value = serde_json::from_str(&t).map_err(|e| bad(format!("checkpoint '{id}' is damaged: {e}")))?;
            let age = now_ms().saturating_sub(v["saved_ms"].as_u64().unwrap_or(0));
            Ok(json!({
                "text": format!("checkpoint '{id}' — {} (saved {}, {} save(s))\n{}", v["note"].as_str().unwrap_or(""), age_text(age), v["saves"], serde_json::to_string_pretty(&v["state"]).unwrap_or_default()),
                "id": id, "note": v["note"], "state": v["state"], "saved_ms": v["saved_ms"], "saves": v["saves"],
            }))
        }
        "task.list" => {
            let now = now_ms();
            let mut rows = Vec::new();
            for e in std::fs::read_dir(base).map_err(|e| bad(e.to_string()))?.flatten() {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                let Ok(v) = std::fs::read_to_string(&p).map_err(|_| ()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ())) else { continue };
                let saved = v["saved_ms"].as_u64().unwrap_or(0);
                if now.saturating_sub(saved) > MAX_AGE_MS {
                    let _ = std::fs::remove_file(&p); // older than 30 days
                    continue;
                }
                rows.push((saved, v["id"].as_str().unwrap_or("?").to_string(), v["note"].as_str().unwrap_or("").to_string()));
            }
            rows.sort_by_key(|r| std::cmp::Reverse(r.0));
            let lines: Vec<String> = rows.iter().map(|(s, id, n)| format!("{id}  ({})  {n}", age_text(now.saturating_sub(*s)))).collect();
            Ok(json!({
                "text": if lines.is_empty() { "no checkpoints".to_string() } else { format!("{}\n-- {} checkpoint(s); task.load {{id}} to resume", lines.join("\n"), lines.len()) },
                "tasks": rows.iter().map(|(s, id, n)| json!({"id": id, "note": n, "saved_ms": s})).collect::<Vec<_>>(),
            }))
        }
        "task.done" => {
            let id = id_arg(a)?;
            std::fs::remove_file(path_of(base, &id)).map_err(|_| not_found(&id))?;
            Ok(json!({"text": format!("checkpoint '{id}' removed")}))
        }
        o => Err(bad(format!("unknown task op '{o}'"))),
    }
}

/// The skill against XtraOS's data folder.
pub fn run(op: &str, a: &Value) -> Result<Value, ActError> {
    run_in(&crate::pairing::data_dir().join("tasks"), op, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_secrets() {
        assert!(valid_id("invoices-nov.2") && !valid_id("../x") && !valid_id("") && !valid_id(".h"));
        assert_eq!(secret_key(&json!({"done": 3, "login": {"Password": "x"}})).as_deref(), Some("Password"));
        assert_eq!(secret_key(&json!([{"api_key": "x"}])).as_deref(), Some("api_key"));
        assert_eq!(secret_key(&json!({"access_token": "x"})).as_deref(), Some("access_token"));
        assert!(secret_key(&json!({"OTP": "1"})).is_some());
        assert!(secret_key(&json!({"rows_done": 12, "next": "row 13", "pinned": true, "spinner": 1, "options": []})).is_none(), "no false alarms");
    }

    #[test]
    fn save_load_list_done() {
        let base = std::env::temp_dir().join(format!("xtraos-tasks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let s = run_in(&base, "task.save", &json!({"id": "invoices", "note": "12 of 40 done", "state": {"next_row": 13}})).unwrap();
        assert!(s["text"].as_str().unwrap().contains("save #1"));
        run_in(&base, "task.save", &json!({"id": "invoices", "note": "13 of 40", "state": {"next_row": 14}})).unwrap();
        let l = run_in(&base, "task.load", &json!({"id": "invoices"})).unwrap();
        assert_eq!(l["state"]["next_row"], 14);
        assert_eq!(l["saves"], 2);
        let ls = run_in(&base, "task.list", &json!({})).unwrap();
        assert_eq!(ls["tasks"][0]["id"], "invoices");
        let e = run_in(&base, "task.save", &json!({"id": "x", "state": {"password": "hunter2"}})).unwrap_err();
        assert_eq!(e.code, "BLOCKED_BY_POLICY");
        let big = "x".repeat(40_000);
        assert!(run_in(&base, "task.save", &json!({"id": "big", "state": big})).is_err());
        run_in(&base, "task.done", &json!({"id": "invoices"})).unwrap();
        assert_eq!(run_in(&base, "task.load", &json!({"id": "invoices"})).unwrap_err().code, "ELEMENT_NOT_FOUND");
        assert_eq!(run_in(&base, "task.list", &json!({})).unwrap()["text"], "no checkpoints");
        let _ = std::fs::remove_dir_all(&base);
    }
}
