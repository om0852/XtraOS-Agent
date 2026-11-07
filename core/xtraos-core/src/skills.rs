//! Skills (P2-M3, ADR 0026): one call for a routine task.
//! - Built-in skills: Rust executors (first: files.* through the Windows shell — undoable,
//!   Recycle Bin for deletes, never overwriting).
//! - Macro skills: JSON files in `%LOCALAPPDATA%\XtraOS\skills\*.json` whose steps run through
//!   `batch` — every step still passes the policy engine.
//!
//! Protected paths are refused before anything happens.
//!
//! Docs: IFileOperation https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Map, Value};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, IBindCtx, CLSCTX_ALL};
use windows::Win32::UI::Shell::{
    FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem,
    SHCreateItemFromParsingName, FILEOPERATION_FLAGS, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO,
    FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT,
};

use crate::input::ActError;
use crate::policy::Class;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "list_skills shows each skill's arguments.",
    }
}

fn blocked(m: impl Into<String>) -> ActError {
    crate::safety::blocked(m)
}

fn internal(m: impl Into<String>) -> ActError {
    ActError {
        code: "INTERNAL",
        message: m.into(),
        hint: "",
    }
}

// ---------------------------------------------------------------------------------------------
// Registry

pub struct Builtin {
    pub name: &'static str,
    pub args: &'static str,
    pub about: &'static str,
    pub class: Class,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "files.list",
        args: "path, pattern?",
        about: "folder contents (dirs end with /, sizes)",
        class: Class::Observe,
    },
    Builtin {
        name: "files.open",
        args: "path",
        about: "open a folder in Explorer, or show a file selected in its folder",
        class: Class::Launch,
    },
    Builtin {
        name: "files.mkdir",
        args: "path",
        about: "create a folder (with parents)",
        class: Class::FileChange,
    },
    Builtin {
        name: "files.copy",
        args: "from[], to_dir",
        about: "copy files/folders; never overwrites",
        class: Class::FileChange,
    },
    Builtin {
        name: "files.move",
        args: "from[], to_dir",
        about: "move files/folders; never overwrites",
        class: Class::FileChange,
    },
    Builtin {
        name: "files.rename",
        args: "path, new_name",
        about: "rename in place; never overwrites",
        class: Class::FileChange,
    },
    Builtin {
        name: "files.delete",
        args: "paths[]",
        about: "to the Recycle Bin (asks the user first)",
        class: Class::Delete,
    },
];

fn skills_dir() -> PathBuf {
    crate::pairing::data_dir().join("skills")
}

/// Macro skills from `skills\*.json` (invalid files are skipped with a note).
fn macros() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(skills_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Some(v) = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            {
                if let Some(n) = v["name"].as_str() {
                    out.push((n.to_string(), v));
                }
            }
        }
    }
    out
}

pub fn list(p: &Value) -> Result<Value, ActError> {
    let prefix = p
        .get("app")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let mut lines: Vec<String> = BUILTINS
        .iter()
        .filter(|b| b.name.starts_with(&prefix))
        .map(|b| format!("{}({}) — {}", b.name, b.args, b.about))
        .collect();
    for (n, v) in macros() {
        if n.starts_with(&prefix) {
            let args: Vec<String> = v["args"]
                .as_object()
                .map(|a| a.keys().cloned().collect())
                .unwrap_or_default();
            lines.push(format!(
                "{n}({}) — {} [macro]",
                args.join(", "),
                v["description"].as_str().unwrap_or("")
            ));
        }
    }
    Ok(
        json!({"text": format!("{}\n-- {} skills", lines.join("\n"), lines.len()), "skills": lines.len()}),
    )
}

// ---------------------------------------------------------------------------------------------
// Paths

/// Absolute, `..`-free form of a path (does not need to exist). None for relative paths.
pub fn normalize(p: &str) -> Option<PathBuf> {
    let path = Path::new(p.trim());
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

fn lower(p: &Path) -> String {
    p.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Folders an agent's file skills may never touch (ADR 0026).
pub fn protected_reason(p: &Path, own: &[PathBuf]) -> Option<String> {
    let s = lower(p);
    if s.starts_with(r"\\") || s.starts_with("//") {
        return Some("network paths".into());
    }
    if s.len() <= 3 {
        return Some("drive roots".into());
    }
    let env = |k: &str| std::env::var(k).ok().map(|v| lower(Path::new(&v)));
    let under = |root: &str| s == root || s.starts_with(&format!("{root}\\"));
    for (k, why) in [
        ("SystemRoot", "the Windows folder"),
        ("ProgramFiles", "Program Files"),
        ("ProgramFiles(x86)", "Program Files"),
        ("ProgramData", "ProgramData"),
    ] {
        if let Some(r) = env(k) {
            if under(&r) {
                return Some(why.into());
            }
        }
    }
    if let (Some(users), Some(me)) = (
        env("SystemDrive").map(|d| format!("{d}\\users")),
        env("USERPROFILE"),
    ) {
        if under(&users) && !under(&me) {
            return Some("other users' folders".into());
        }
    }
    for o in own {
        if under(&lower(o)) {
            return Some("XtraOS's own files (token, policy, logs, app map, install)".into());
        }
    }
    None
}

fn own_dirs() -> Vec<PathBuf> {
    let mut v = vec![crate::pairing::data_dir()];
    if let Ok(exe) = std::env::current_exe() {
        // <prefix>\bin\xtraos-core.exe → protect <prefix>
        if let Some(prefix) = exe.parent().and_then(Path::parent) {
            v.push(prefix.to_path_buf());
        }
    }
    v
}

/// Normalised + checked path (and its real target if it exists: junctions cannot sneak around).
fn checked(p: &str) -> Result<PathBuf, ActError> {
    let n = normalize(p).ok_or_else(|| bad(format!("'{p}' must be an absolute path")))?;
    let own = own_dirs();
    if let Some(why) = protected_reason(&n, &own) {
        return Err(blocked(format!("{} is protected ({why})", n.display())));
    }
    if let Ok(real) = std::fs::canonicalize(&n) {
        if let Some(why) = protected_reason(&real, &own) {
            return Err(blocked(format!(
                "{} leads to a protected place ({why})",
                n.display()
            )));
        }
    }
    Ok(n)
}

fn str_list(v: &Value, k: &str) -> Result<Vec<String>, ActError> {
    let a = v
        .get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| bad(format!("'{k}' (array of paths) is required")))?;
    if a.is_empty() || a.len() > 200 {
        return Err(bad(format!("'{k}' needs 1..200 paths")));
    }
    a.iter()
        .map(|x| {
            x.as_str()
                .map(String::from)
                .ok_or_else(|| bad(format!("'{k}' items are strings")))
        })
        .collect()
}

fn str_arg<'a>(v: &'a Value, k: &str) -> Result<&'a str, ActError> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad(format!("'{k}' is required")))
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Plan (for the policy gate) and run

/// What the policy needs before the skill runs: class + a description for the dialog/log.
#[derive(Debug)]
pub struct Plan {
    pub name: String,
    pub class: Class,
    pub what: String,
    pub is_macro: bool,
}

pub fn plan(p: &Value) -> Result<Plan, ActError> {
    let name = str_arg(p, "name")?.to_string();
    let args = p.get("args").cloned().unwrap_or(json!({}));
    if let Some(b) = BUILTINS.iter().find(|b| b.name == name) {
        // Validate every path now: protected ones are refused before anything happens.
        let what = match b.name {
            "files.list" | "files.open" | "files.mkdir" => {
                let x = checked(str_arg(&args, "path")?)?;
                format!("{} {}", b.name, x.display())
            }
            "files.rename" => {
                let x = checked(str_arg(&args, "path")?)?;
                format!(
                    "rename {} to \"{}\"",
                    x.display(),
                    str_arg(&args, "new_name")?
                )
            }
            "files.copy" | "files.move" => {
                let from = str_list(&args, "from")?;
                for f in &from {
                    checked(f)?;
                }
                let to = checked(str_arg(&args, "to_dir")?)?;
                format!(
                    "{} {} item(s) to {}",
                    &b.name[6..],
                    from.len(),
                    to.display()
                )
            }
            "files.delete" => {
                let ps = str_list(&args, "paths")?;
                let mut names = Vec::new();
                for f in &ps {
                    names.push(checked(f)?.display().to_string());
                }
                let shown: Vec<&str> = names.iter().take(8).map(String::as_str).collect();
                format!(
                    "move {} item(s) to the Recycle Bin:\n{}{}",
                    ps.len(),
                    shown.join("\n"),
                    if ps.len() > 8 { "\n…" } else { "" }
                )
            }
            _ => b.name.to_string(),
        };
        return Ok(Plan {
            name,
            class: b.class,
            what,
            is_macro: false,
        });
    }
    if macros().iter().any(|(n, _)| *n == name) {
        return Ok(Plan {
            what: format!("macro skill {name}"),
            name,
            class: Class::Observe,
            is_macro: true,
        });
    }
    Err(bad(format!("unknown skill '{name}' (see list_skills)")))
}

pub fn run(p: &Value, log: &mut dyn FnMut(crate::exec::StepLog)) -> Result<Value, ActError> {
    let pl = plan(p)?;
    let args = p.get("args").cloned().unwrap_or(json!({}));
    if pl.is_macro {
        let (_, def) = macros()
            .into_iter()
            .find(|(n, _)| *n == pl.name)
            .expect("planned");
        let steps = expand(&def, &args)?;
        return crate::exec::batch(&json!({"steps": steps}), log);
    }
    match pl.name.as_str() {
        "files.list" => files_list(&args),
        "files.open" => files_open(&args),
        "files.mkdir" => {
            let d = checked(str_arg(&args, "path")?)?;
            let existed = d.is_dir();
            std::fs::create_dir_all(&d).map_err(|e| internal(format!("mkdir: {e}")))?;
            Ok(
                json!({"text": format!("ok {} {}", if existed { "exists" } else { "created" }, d.display())}),
            )
        }
        "files.copy" | "files.move" => copy_move(&args, pl.name == "files.move"),
        "files.rename" => rename(&args),
        "files.delete" => delete(&args),
        n => Err(bad(format!("unknown skill '{n}'"))),
    }
}

/// Macro args: declared names, required ones present, `${x}` replaced (a whole-string
/// placeholder keeps the JSON type, e.g. an array of paths).
pub fn expand(def: &Value, given: &Value) -> Result<Vec<Value>, ActError> {
    let declared = def["args"].as_object().cloned().unwrap_or_default();
    let mut vals = Map::new();
    for (k, spec) in &declared {
        match given.get(k) {
            Some(v) => {
                vals.insert(k.clone(), v.clone());
            }
            None => match spec.get("default") {
                Some(d) => {
                    vals.insert(k.clone(), d.clone());
                }
                None if spec["required"].as_bool().unwrap_or(true) => {
                    return Err(bad(format!("argument '{k}' is required")))
                }
                None => {}
            },
        }
    }
    if let Some(g) = given.as_object() {
        if let Some(extra) = g.keys().find(|k| !declared.contains_key(*k)) {
            return Err(bad(format!("unknown argument '{extra}'")));
        }
    }
    fn sub(v: &Value, vals: &Map<String, Value>) -> Value {
        match v {
            Value::String(s) => {
                if let Some(k) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
                    if let Some(x) = vals.get(k) {
                        return x.clone();
                    }
                }
                let mut out = s.clone();
                for (k, x) in vals {
                    let txt = x
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| x.to_string());
                    out = out.replace(&format!("${{{k}}}"), &txt);
                }
                Value::String(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| sub(x, vals)).collect()),
            Value::Object(o) => {
                Value::Object(o.iter().map(|(k, x)| (k.clone(), sub(x, vals))).collect())
            }
            other => other.clone(),
        }
    }
    let steps = def["steps"]
        .as_array()
        .ok_or_else(|| bad("the macro skill has no 'steps'"))?;
    Ok(steps.iter().map(|s| sub(s, &vals)).collect())
}

// ---------------------------------------------------------------------------------------------
// File skills

/// `*` and `?` wildcard match (case-insensitive).
pub fn wildcard(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (
        pat.to_lowercase().chars().collect(),
        name.to_lowercase().chars().collect(),
    );
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

fn size(b: u64) -> String {
    match b {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

fn files_list(a: &Value) -> Result<Value, ActError> {
    let d = checked(str_arg(a, "path")?)?;
    let pat = a.get("pattern").and_then(Value::as_str).unwrap_or("*");
    let rd = std::fs::read_dir(&d).map_err(|e| bad(format!("{}: {e}", d.display())))?;
    let mut rows: Vec<(bool, String, u64)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let md = e.metadata().ok()?;
            wildcard(pat, &n).then_some((md.is_dir(), n, md.len()))
        })
        .collect();
    rows.sort_by(|x, y| {
        y.0.cmp(&x.0)
            .then(x.1.to_lowercase().cmp(&y.1.to_lowercase()))
    });
    let total = rows.len();
    let lines: Vec<String> = rows
        .iter()
        .take(200)
        .map(|(dir, n, s)| {
            if *dir {
                format!("{n}/")
            } else {
                format!("{n}  {}", size(*s))
            }
        })
        .collect();
    Ok(
        json!({"text": format!("{}\n{}\n-- {total} items{}", d.display(), lines.join("\n"),
        if total > 200 { " (first 200)" } else { "" }), "items": total}),
    )
}

fn files_open(a: &Value) -> Result<Value, ActError> {
    let x = checked(str_arg(a, "path")?)?;
    if !x.exists() {
        return Err(bad(format!("{} does not exist", x.display())));
    }
    let args = if x.is_dir() {
        json!([x.to_string_lossy()])
    } else {
        json!([format!("/select,{}", x.to_string_lossy())])
    };
    crate::exec::dispatch(
        "launch_app",
        &json!({"app": "explorer", "args": args, "timeout_ms": 15000}),
    )
}

/// Runs shell file operations on an STA thread; Err when the shell reports failure/abort.
fn shell_op(
    flags: FILEOPERATION_FLAGS,
    queue: impl FnOnce(&IFileOperation) -> windows::core::Result<()> + Send + 'static,
) -> Result<(), ActError> {
    crate::appmap::on_sta(move || unsafe {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
            .map_err(|e| internal(format!("file operation: {e}")))?;
        op.SetOperationFlags(flags)
            .map_err(|e| internal(e.to_string()))?;
        queue(&op).map_err(|e| internal(format!("file operation: {e}")))?;
        op.PerformOperations()
            .map_err(|e| internal(format!("file operation failed: {e}")))?;
        if op
            .GetAnyOperationsAborted()
            .map(|b| b.as_bool())
            .unwrap_or(false)
        {
            return Err(ActError {
                code: "NEEDS_HUMAN",
                message: "the file operation was cancelled or partly failed".into(),
                hint: "Check the folder (files.list) and ask the user.",
            });
        }
        Ok(())
    })
}

unsafe fn item(p: &Path) -> windows::core::Result<IShellItem> {
    SHCreateItemFromParsingName(&HSTRING::from(p.as_os_str()), None::<&IBindCtx>)
}

const SAFE_FLAGS: FILEOPERATION_FLAGS =
    FILEOPERATION_FLAGS(FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0 | FOF_NOERRORUI.0 | FOF_SILENT.0);

fn copy_move(a: &Value, is_move: bool) -> Result<Value, ActError> {
    let from: Vec<PathBuf> = str_list(a, "from")?
        .iter()
        .map(|f| checked(f))
        .collect::<Result<_, _>>()?;
    let to = checked(str_arg(a, "to_dir")?)?;
    if !to.is_dir() {
        return Err(bad(format!(
            "{} is not an existing folder (files.mkdir first)",
            to.display()
        )));
    }
    for f in &from {
        if !f.exists() {
            return Err(bad(format!("{} does not exist", f.display())));
        }
        let target = to.join(name_of(f));
        if target.exists() {
            return Err(bad(format!(
                "{} already exists; nothing was {} (never overwrites)",
                target.display(),
                if is_move { "moved" } else { "copied" }
            )));
        }
    }
    let (src, dst) = (from.clone(), to.clone());
    shell_op(SAFE_FLAGS, move |op| unsafe {
        let d = item(&dst)?;
        for f in &src {
            let s = item(f)?;
            if is_move {
                op.MoveItem(&s, &d, PCWSTR::null(), None::<&IFileOperationProgressSink>)?;
            } else {
                op.CopyItem(&s, &d, PCWSTR::null(), None::<&IFileOperationProgressSink>)?;
            }
        }
        Ok(())
    })?;
    let done: Vec<String> = from
        .iter()
        .map(|f| to.join(name_of(f)).display().to_string())
        .collect();
    Ok(
        json!({"text": format!("ok {} {} item(s):\n{}", if is_move { "moved" } else { "copied" }, done.len(), done.join("\n")), "items": done}),
    )
}

fn rename(a: &Value) -> Result<Value, ActError> {
    let x = checked(str_arg(a, "path")?)?;
    let new_name = str_arg(a, "new_name")?;
    if new_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) || new_name.trim() == ".."
    {
        return Err(bad("'new_name' is a plain name, not a path"));
    }
    if !x.exists() {
        return Err(bad(format!("{} does not exist", x.display())));
    }
    let target = x.with_file_name(new_name);
    checked(&target.to_string_lossy())?;
    if target.exists() {
        return Err(bad(format!(
            "{} already exists (never overwrites)",
            target.display()
        )));
    }
    let (src, nn) = (x.clone(), new_name.to_string());
    shell_op(SAFE_FLAGS, move |op| unsafe {
        op.RenameItem(
            &item(&src)?,
            &HSTRING::from(nn.as_str()),
            None::<&IFileOperationProgressSink>,
        )
    })?;
    Ok(json!({"text": format!("ok renamed to {}", target.display())}))
}

fn delete(a: &Value) -> Result<Value, ActError> {
    let ps: Vec<PathBuf> = str_list(a, "paths")?
        .iter()
        .map(|f| checked(f))
        .collect::<Result<_, _>>()?;
    for f in &ps {
        if !f.exists() {
            return Err(bad(format!("{} does not exist", f.display())));
        }
    }
    let items = ps.clone();
    // Recycle Bin (undoable). No FOF_NOCONFIRMATION: if Windows cannot recycle an item it asks
    // the user before deleting it for good — the agent never deletes permanently.
    shell_op(
        FILEOPERATION_FLAGS(SAFE_FLAGS.0 | FOFX_RECYCLEONDELETE.0),
        move |op| unsafe {
            for f in &items {
                op.DeleteItem(&item(f)?, None::<&IFileOperationProgressSink>)?;
            }
            Ok(())
        },
    )?;
    Ok(json!({"text": format!("ok {} item(s) moved to the Recycle Bin", ps.len())}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard("*.txt", "Report.TXT"));
        assert!(wildcard("rep?rt*", "report-final.pdf"));
        assert!(!wildcard("*.txt", "report.pdf"));
        assert!(wildcard("*", ""));
    }

    #[test]
    fn paths_normalise_and_protect() {
        assert_eq!(normalize(r"C:\a\b\..\c").unwrap(), PathBuf::from(r"C:\a\c"));
        assert!(normalize(r"relative\x").is_none());
        let own = vec![PathBuf::from(r"C:\Users\me\AppData\Local\XtraOS")];
        assert!(protected_reason(Path::new(r"\\server\share\x"), &own).is_some());
        assert!(protected_reason(Path::new(r"C:\"), &own).is_some());
        if let Ok(w) = std::env::var("SystemRoot") {
            assert!(protected_reason(&Path::new(&w).join("system32"), &own).is_some());
        }
        assert!(protected_reason(
            Path::new(r"C:\Users\me\AppData\Local\XtraOS\policy.json"),
            &own
        )
        .is_some());
        if let Ok(home) = std::env::var("USERPROFILE") {
            assert!(
                protected_reason(&Path::new(&home).join("Documents").join("a.txt"), &own).is_none()
            );
        }
        // ..\ cannot climb into a protected folder
        let sneaky = normalize(&format!(
            r"{}\..\..\Windows\x.dll",
            std::env::var("USERPROFILE").unwrap_or(r"C:\Users\me".into())
        ));
        assert!(protected_reason(&sneaky.unwrap(), &own).is_some());
    }

    #[test]
    fn macro_expansion() {
        let def = json!({"args": {"x": {"required": true}, "who": {"default": "Rahul"}},
            "steps": [{"tool": "type", "args": {"text": "hi ${who}: ${x}"}}, {"tool": "find", "args": {"query": "${x}"}}]});
        let s = expand(&def, &json!({"x": 7})).unwrap();
        assert_eq!(s[0]["args"]["text"], "hi Rahul: 7");
        assert_eq!(
            s[1]["args"]["query"], 7,
            "whole-string placeholder keeps the type"
        );
        assert_eq!(expand(&def, &json!({})).unwrap_err().code, "INVALID_ARGS");
        assert_eq!(
            expand(&def, &json!({"x": 1, "y": 2})).unwrap_err().code,
            "INVALID_ARGS"
        );
    }

    #[test]
    fn unknown_skill_and_bad_args() {
        assert_eq!(
            plan(&json!({"name": "nope"})).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            plan(&json!({"name": "files.copy", "args": {"from": [], "to_dir": "C:\\x"}}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            plan(&json!({"name": "files.list", "args": {"path": "C:\\"}}))
                .unwrap_err()
                .code,
            "BLOCKED_BY_POLICY"
        );
    }
}
