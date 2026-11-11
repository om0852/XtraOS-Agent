//! Skill sharing (P7-M5, ADR 0058): `skill.export` writes a recorded skill as one file;
//! `skill.import` reads one, shows the user every step and keeps it only after "Import".
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::input::ActError;

const MAX_FILE: u64 = 256 * 1024;
const FORMAT: u64 = 1;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "skill.export {name, out?, include_defaults?} · skill.import {path, name?}" }
}

fn skills_dir() -> PathBuf {
    crate::pairing::data_dir().join("skills")
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The shareable document for a macro skill definition (pure). Typed-text defaults are removed
/// unless `include_defaults` (they may be personal); those arguments become required.
pub fn export_doc(def: &Value, include_defaults: bool, exported: &str) -> Value {
    let mut args = def["args"].as_object().cloned().unwrap_or_default();
    if !include_defaults {
        for (_, spec) in args.iter_mut() {
            if let Some(o) = spec.as_object_mut() {
                if o.remove("default").is_some() {
                    o.insert("required".into(), json!(true));
                }
            }
        }
    }
    json!({
        "xtraos_skill": FORMAT,
        "name": def["name"],
        "description": def["description"],
        "exported": exported,
        "xtraos_version": env!("CARGO_PKG_VERSION"),
        "args": Value::Object(args),
        "steps": def["steps"],
    })
}

/// A shared skill file, checked (pure): (name, description, steps, args).
pub fn validate(doc: &Value) -> Result<(String, String, Vec<Value>, Map<String, Value>), ActError> {
    if doc["xtraos_skill"].as_u64() != Some(FORMAT) {
        return Err(bad("not an XtraOS skill file (xtraos_skill: 1 missing)"));
    }
    let name = doc["name"].as_str().unwrap_or("").to_string();
    if !valid_name(&name) {
        return Err(bad("the file's skill name is not valid"));
    }
    let steps = doc["steps"].as_array().cloned().unwrap_or_default();
    if steps.is_empty() || steps.len() > crate::exec::MAX_STEPS {
        return Err(bad(format!("a skill has 1..{} steps; this one has {}", crate::exec::MAX_STEPS, steps.len())));
    }
    for (i, s) in steps.iter().enumerate() {
        let tool = s["tool"].as_str().unwrap_or("");
        if !crate::exec::BATCH_TOOLS.contains(&tool) {
            return Err(bad(format!("step {}: '{tool}' is not a tool a skill may use", i + 1)));
        }
        if !s["args"].is_object() {
            return Err(bad(format!("step {}: args must be an object", i + 1)));
        }
        if tool == "run_skill" {
            let n = s["args"]["name"].as_str().unwrap_or("");
            if !crate::skills::BUILTINS.iter().any(|b| b.name == n) {
                return Err(bad(format!("step {}: runs '{n}', which is not a built-in skill (skills cannot nest)", i + 1)));
            }
        }
    }
    let args = doc["args"].as_object().cloned().unwrap_or_default();
    Ok((name, doc["description"].as_str().unwrap_or("").to_string(), steps, args))
}

/// One step in plain words for the review screen (pure).
pub fn describe_step(step: &Value) -> String {
    let clip = crate::perceive::clip;
    let tool = step["tool"].as_str().unwrap_or("?");
    let a = &step["args"];
    let mut s = tool.replace('_', " ");
    let win = |l: &Value| format!("{} \"{}\"", l["process"].as_str().unwrap_or("?"), clip(l["title"].as_str().unwrap_or(""), 40));
    match tool {
        "run_skill" => s = format!("run skill {}", a["name"].as_str().unwrap_or("?")),
        "type" => s = format!("type \"{}\"", clip(a["text"].as_str().unwrap_or(""), 50)),
        "set_value" => s = format!("set value \"{}\"", clip(a["value"].as_str().unwrap_or(""), 50)),
        "press_keys" => {
            let k: Vec<String> = match &a["keys"] {
                Value::Array(x) => x.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                Value::String(x) => vec![x.clone()],
                _ => vec![],
            };
            s = format!("press {}", k.join(", "));
        }
        "launch_app" => s = format!("open {}", clip(a["app"].as_str().unwrap_or("?"), 60)),
        "browser" => s = format!("browser {}{}", a["action"].as_str().unwrap_or("?"), a["url"].as_str().map(|u| format!(" {}", clip(u, 60))).unwrap_or_default()),
        _ => {}
    }
    if let Some(o) = a.as_object() {
        for (k, v) in o {
            if let Some(l) = v.get("locate") {
                s.push_str(&format!(" {} \"{}\" in {}", l["role"].as_str().unwrap_or("?"), clip(l["name"].as_str().unwrap_or(""), 40), win(l)));
                break;
            }
            if let Some(l) = v.get("locate_window") {
                s.push_str(&format!(" in {}", win(l)));
                break;
            }
            if k == "x" {
                if let Some(at) = v.get("at") {
                    s.push_str(&format!(" at a point in {}", win(at)));
                    break;
                }
            }
        }
    }
    s
}

fn documents_skills() -> PathBuf {
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    let docs = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KNOWN_FOLDER_FLAG(0), None) }
        .ok()
        .and_then(|p| {
            let s = unsafe { p.to_string() }.ok();
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
            s
        })
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    docs.unwrap_or_else(std::env::temp_dir).join("XtraOS skills")
}

fn today() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

pub fn export(a: &Value) -> Result<Value, ActError> {
    let name = a.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    if !valid_name(&name) {
        return Err(bad("'name' is the skill to export (see list_skills)"));
    }
    let src = skills_dir().join(format!("{name}.json"));
    let def: Value = std::fs::read_to_string(&src)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| bad(format!("no recorded skill '{name}' (built-in skills cannot be exported)")))?;
    let out = match a.get("out").and_then(Value::as_str) {
        Some(o) => crate::skills::checked(o)?,
        None => documents_skills().join(format!("{name}.xtraos-skill.json")),
    };
    if out.exists() {
        return Err(bad(format!("{} already exists (never overwrites)", out.display())));
    }
    let include = a.get("include_defaults").and_then(Value::as_bool).unwrap_or(false);
    let doc = export_doc(&def, include, &today());
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| bad(format!("{}: {e}", d.display())))?;
    }
    std::fs::write(&out, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| bad(format!("could not write: {e}")))?;
    let steps = doc["steps"].as_array().map(Vec::len).unwrap_or(0);
    Ok(json!({
        "text": format!("exported '{name}' ({steps} step(s)) to {}{}", out.display(), if include { "" } else { "; typed text was left out (the arguments are required on import)" }),
        "path": out.display().to_string(),
    }))
}

/// Reads and checks a shared skill file; returns (name to save under, document, review text).
pub fn prepare_import(path: &Path, rename: Option<&str>, existing: &Path) -> Result<(String, Value, String), ActError> {
    let md = std::fs::metadata(path).map_err(|_| bad(format!("{} does not exist", path.display())))?;
    if md.len() > MAX_FILE {
        return Err(bad(format!("{} is larger than {} KB; not a skill file", path.display(), MAX_FILE / 1024)));
    }
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| bad(e.to_string()))?)
        .map_err(|e| bad(format!("not JSON: {e}")))?;
    let (file_name, desc, steps, args) = validate(&doc)?;
    let name = match rename {
        Some(n) if valid_name(n) => n.to_string(),
        Some(_) => return Err(bad("'name': 1-64 letters, digits, - _ .")),
        None => file_name,
    };
    if existing.join(format!("{name}.json")).exists() || crate::skills::BUILTINS.iter().any(|b| b.name == name) {
        return Err(bad(format!("a skill named '{name}' exists; import with another name")));
    }
    let lines: Vec<String> = steps.iter().enumerate().map(|(i, s)| format!("{}. {}", i + 1, describe_step(s))).collect();
    let arg_names: Vec<&String> = args.keys().collect();
    let review = format!(
        "Import the skill \"{name}\" from {}?\n{}\n\nIt will do these {} step(s) when run:\n{}\n{}\nEach step still asks you where XtraOS always asks (deleting, sending, buying…).",
        path.display(),
        if desc.is_empty() { String::new() } else { format!("Description: {}", crate::perceive::clip(&desc, 200)) },
        steps.len(),
        lines.join("\n"),
        if arg_names.is_empty() { String::new() } else { format!("\nInputs: {}", arg_names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) }
    );
    let def = json!({
        "name": name,
        "description": if desc.is_empty() { format!("imported, {} step(s)", steps.len()) } else { desc },
        "imported": true,
        "args": Value::Object(args),
        "steps": steps,
    });
    Ok((name, def, review))
}

pub fn import(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(a.get("path").and_then(Value::as_str).ok_or_else(|| bad("'path' is required"))?)?;
    let dir = skills_dir();
    let (name, def, review) = prepare_import(&path, a.get("name").and_then(Value::as_str), &dir)?;
    crate::safety::activity("waiting for you: review a skill to import");
    let answer = crate::safety::prompt(
        "XtraOS — import a skill?",
        &review,
        vec!["Import".into(), "Cancel".into()],
        1,
        std::time::Duration::from_secs(300),
    )?;
    if answer != Some(0) {
        return Err(ActError {
            code: "BLOCKED_BY_POLICY",
            message: format!("the user did not import '{name}'"),
            hint: "Do not retry without asking the user.",
        });
    }
    std::fs::create_dir_all(&dir).map_err(|e| bad(e.to_string()))?;
    std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(&def).unwrap_or_default()).map_err(|e| bad(format!("could not save: {e}")))?;
    Ok(json!({"text": format!("imported skill '{name}'; run it with run_skill {{name: \"{name}\"}}"), "name": name}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> Value {
        json!({
            "name": "send-report", "description": "recorded 3 step(s)",
            "args": {"text1": {"default": "Hi Asha, report attached", "required": false}},
            "steps": [
                {"tool": "launch_app", "args": {"app": "notepad"}},
                {"tool": "click", "args": {"ref": {"locate": {"process": "notepad.exe", "title": "Untitled - Notepad", "role": "Button", "name": "Save"}}}},
                {"tool": "type", "args": {"window": {"locate_window": {"process": "notepad.exe", "title": "Untitled - Notepad"}}, "text": "${text1}"}}
            ]
        })
    }

    #[test]
    fn export_removes_typed_text_unless_asked() {
        let d = export_doc(&def(), false, "2026-11-11");
        assert_eq!(d["xtraos_skill"], 1);
        assert!(d["args"]["text1"].get("default").is_none());
        assert_eq!(d["args"]["text1"]["required"], true);
        let k = export_doc(&def(), true, "2026-11-11");
        assert_eq!(k["args"]["text1"]["default"], "Hi Asha, report attached");
    }

    #[test]
    fn import_checks_and_describes() {
        let d = export_doc(&def(), false, "x");
        let (n, _, steps, _) = validate(&d).unwrap();
        assert_eq!(n, "send-report");
        assert_eq!(describe_step(&steps[0]), "open notepad");
        assert_eq!(describe_step(&steps[1]), "click Button \"Save\" in notepad.exe \"Untitled - Notepad\"");
        assert_eq!(describe_step(&steps[2]), "type \"${text1}\" in notepad.exe \"Untitled - Notepad\"");
        let mut v = d.clone();
        v["xtraos_skill"] = json!(2);
        assert!(validate(&v).is_err(), "unknown format");
        let mut v = d.clone();
        v["steps"][0]["tool"] = json!("elevated_helper");
        assert!(validate(&v).is_err(), "a tool a batch may not run");
        let mut v = d.clone();
        v["steps"][0] = json!({"tool": "run_skill", "args": {"name": "my-other-macro"}});
        assert!(validate(&v).is_err(), "no nested macros");
        let mut v = d.clone();
        v["steps"][0] = json!({"tool": "run_skill", "args": {"name": "files.delete", "args": {"paths": ["C:\\x"]}}});
        assert!(validate(&v).is_ok(), "built-in skills are fine (they still ask)");
        assert_eq!(describe_step(&v["steps"][0]), "run skill files.delete");
        let mut v = d.clone();
        v["steps"] = json!((0..60).map(|_| json!({"tool": "click", "args": {}})).collect::<Vec<_>>());
        assert!(validate(&v).is_err(), "too many steps");
    }

    #[test]
    fn prepare_import_builds_the_review() {
        let dir = std::env::temp_dir().join(format!("xtraos-share-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        let f = dir.join("send-report.xtraos-skill.json");
        std::fs::write(&f, serde_json::to_string(&export_doc(&def(), false, "x")).unwrap()).unwrap();
        let (name, saved, review) = prepare_import(&f, None, &dir.join("skills")).unwrap();
        assert_eq!(name, "send-report");
        assert_eq!(saved["imported"], true);
        assert!(review.contains("3 step(s)") && review.contains("2. click Button \"Save\"") && review.contains("Inputs: text1"), "{review}");
        std::fs::write(dir.join("skills").join("send-report.json"), "{}").unwrap();
        assert!(prepare_import(&f, None, &dir.join("skills")).is_err(), "never replaces a skill");
        assert_eq!(prepare_import(&f, Some("copy-2"), &dir.join("skills")).unwrap().0, "copy-2");
        assert!(prepare_import(&f, Some("files.delete"), &dir.join("skills")).is_err(), "not a built-in name");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
