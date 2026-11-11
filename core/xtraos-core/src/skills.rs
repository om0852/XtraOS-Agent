//! Skills (P2-M3, ADR 0026): one call for a routine task.
//! - Built-in skills: Rust executors (first: files.* through the Windows shell — undoable,
//!   Recycle Bin for deletes, never overwriting).
//! - Macro skills: JSON files in `%LOCALAPPDATA%\XtraOS\skills\*.json` whose steps run through
//!   `batch` — every step still passes the policy engine.
//!
//! Protected paths are refused before anything happens.
//!
//! Docs: IFileOperation https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation
use std::collections::HashMap;
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
        args: "path, pattern?, sort?",
        about: "folder contents (dirs end with /, sizes, modified); sort: name | newest",
        class: Class::Observe,
    },
    Builtin {
        name: "files.find",
        args: "path, pattern, limit?",
        about: "find files by name in a folder and its subfolders, newest first, with dates",
        class: Class::Observe,
    },
    Builtin {
        name: "files.open",
        args: "path, app? (true: open the file in its default app)",
        about: "show a file selected in its folder in Explorer (a folder: open it); app: true opens a document in its default app (never programs or scripts)",
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
        name: "office.excel_read",
        args: "path, range, sheet?",
        about: "read cells from an Excel file (a hidden Excel, macros off; ≤ 2,000 cells)",
        class: Class::Observe,
    },
    Builtin {
        name: "office.excel_write",
        args: "path, range (top-left cell), values[[...]], sheet?, save_as?",
        about: "write cells into an Excel file and save (asks the user; ≤ 1,000 cells; a new path creates the workbook)",
        class: Class::FileChange,
    },
    Builtin {
        name: "office.word_read",
        args: "path, max_chars?",
        about: "the text of a Word document (a hidden Word, macros off)",
        class: Class::Observe,
    },
    Builtin {
        name: "office.word_append",
        args: "path, text, save_as?",
        about: "add paragraphs at the end of a Word document, or create it (asks the user)",
        class: Class::FileChange,
    },
    Builtin {
        name: "office.ppt_read",
        args: "path, notes?, max_chars?",
        about: "the slides of a PowerPoint file: titles, text, notes (a hidden PowerPoint, macros off)",
        class: Class::Observe,
    },
    Builtin {
        name: "office.ppt_add_slide",
        args: "path, title, bullets?[], at?, save_as?",
        about: "add a title + bullets slide to a PowerPoint file, or create it (asks the user)",
        class: Class::FileChange,
    },
    Builtin {
        name: "window.snap",
        args: "window, to: left|right|top|bottom|top-left|top-right|bottom-left|bottom-right|center|maximize",
        about: "snap a window to part of its monitor (background, no focus change)",
        class: Class::Input,
    },
    Builtin {
        name: "files.recent",
        args: "days? (2), limit? (20), pattern?",
        about: "files changed lately in Desktop, Documents and Downloads, newest first",
        class: Class::Observe,
    },
    Builtin {
        name: "files.read_text",
        args: "path, max_chars? (20000)",
        about: "the text of a text file (txt, csv, json, md, code; ≤ 5 MB)",
        class: Class::Observe,
    },
    Builtin {
        name: "disk.usage",
        args: "",
        about: "free space on each drive (LOW under 10 % or 5 GB)",
        class: Class::Observe,
    },
    Builtin {
        name: "routine.pause",
        args: "name",
        about: "stop a routine from running until routine.resume",
        class: Class::Observe,
    },
    Builtin {
        name: "routine.resume",
        args: "name",
        about: "let a paused routine run again",
        class: Class::FileChange,
    },
    Builtin {
        name: "screen.find",
        args: "query, timeout_ms? (5000), max_windows? (10)",
        about: "find text (element names) in every open window, front first; refs work at once",
        class: Class::Observe,
    },
    Builtin {
        name: "notify.show",
        args: "text, title?",
        about: "a Windows notification from the XtraOS tray icon (with routine once_at: a reminder)",
        class: Class::Observe,
    },
    Builtin {
        name: "layout.save",
        args: "name, filter?",
        about: "remember where the open windows are (position, size, maximized/minimized)",
        class: Class::Observe,
    },
    Builtin {
        name: "layout.restore",
        args: "name",
        about: "put the open windows back where a saved layout had them (reports windows not open)",
        class: Class::Input,
    },
    Builtin {
        name: "layout.list",
        args: "",
        about: "the saved window layouts",
        class: Class::Observe,
    },
    Builtin {
        name: "layout.delete",
        args: "name",
        about: "delete a saved window layout",
        class: Class::Observe,
    },
    Builtin {
        name: "routine.add",
        args: "name, skill, args?, every_minutes? (≥ 5) | daily_at? \"HH:MM\" | once_at? \"HH:MM\"|\"YYYY-MM-DD HH:MM\" | new_file_in? (folder) + pattern?, notify? failure|always|never",
        about: "run a skill on a schedule or for each new file in a folder ({file} in args), while XtraOS runs (the user approves it in a dialog)",
        class: Class::FileChange,
    },
    Builtin {
        name: "routine.list",
        args: "",
        about: "the scheduled routines and their last run",
        class: Class::Observe,
    },
    Builtin {
        name: "routine.run",
        args: "name",
        about: "run a routine now (as itself: its own rules apply)",
        class: Class::FileChange,
    },
    Builtin {
        name: "routine.remove",
        args: "name",
        about: "delete a routine",
        class: Class::FileChange,
    },
    Builtin {
        name: "routine.summary",
        args: "hours? (24)",
        about: "what the routines did: runs, failures, last result",
        class: Class::Observe,
    },
    Builtin {
        name: "office.export_pdf",
        args: "path, out?",
        about: "save a Word, Excel or PowerPoint file as PDF next to it (asks the user; never overwrites)",
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
    Builtin {
        name: "files.duplicates",
        args: "path, min_kb?, limit?",
        about: "identical files (byte for byte) under a folder, biggest waste first; delete with files.delete",
        class: Class::Observe,
    },
    Builtin {
        name: "files.largest",
        args: "path, limit?",
        about: "the biggest files under a folder, with sizes and dates",
        class: Class::Observe,
    },
    Builtin {
        name: "files.organize",
        args: "path, by?: type|month, apply?",
        about: "sort the files in a folder (e.g. Downloads) into subfolders; without apply only the plan; apply asks the user",
        class: Class::FileChange,
    },
    Builtin {
        name: "files.bulk_rename",
        args: "path, find, replace, pattern?, apply?",
        about: "replace text in many file names; without apply only the preview; apply asks the user",
        class: Class::FileChange,
    },
    Builtin {
        name: "skill.export",
        args: "name, out?, include_defaults?",
        about: "save a recorded skill as one file to share (typed text left out unless include_defaults; asks the user)",
        class: Class::FileChange,
    },
    Builtin {
        name: "skill.import",
        args: "path, name?",
        about: "add a shared skill file: the user reviews every step and must press Import",
        class: Class::Observe,
    },
    Builtin {
        name: "task.save",
        args: "id, state (any JSON ≤ 32 KB), note?",
        about: "save a long task's progress so another agent or session can resume (never credentials)",
        class: Class::Observe,
    },
    Builtin {
        name: "task.load",
        args: "id",
        about: "the saved progress of a task",
        class: Class::Observe,
    },
    Builtin {
        name: "task.list",
        args: "",
        about: "saved task checkpoints, newest first (older than 30 days are removed)",
        class: Class::Observe,
    },
    Builtin {
        name: "task.done",
        args: "id",
        about: "remove a finished task's checkpoint",
        class: Class::Observe,
    },
    Builtin {
        name: "dev.check",
        args: "window, expect[{text}|{no_text}|{element, role?, value?, enabled?}|{title}|{changed: true}], timeout_ms?, screenshot_on_fail?",
        about: "UI check of an app: PASS/FAIL per expectation, retried until timeout (3 s); a failure saves a screenshot",
        class: Class::Observe,
    },
    Builtin {
        name: "whatsapp.send",
        args: "phone, text?, file?, keep_open?",
        about: "send a message/file on WhatsApp Web (asks the user first; checks the chat is that number)",
        class: Class::MessageSend,
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
    // ADR 0050: Windows' hidden system folders and files at the root of any drive.
    if s.as_bytes().get(1) == Some(&b':') && s.as_bytes().get(2) == Some(&b'\\') {
        let first = s[3..].split('\\').next().unwrap_or("");
        if matches!(
            first,
            "$recycle.bin"
                | "system volume information"
                | "recovery"
                | "boot"
                | "efi"
                | "pagefile.sys"
                | "hiberfil.sys"
                | "swapfile.sys"
                | "$windows.~bt"
                | "$windows.~ws"
                | "$winreagent"
        ) {
            return Some("Windows system files at the drive root".into());
        }
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
    let mut v = vec![crate::pairing::data_dir(), crate::sandbox::shared_base()];
    if let Ok(exe) = std::env::current_exe() {
        // <prefix>\bin\xtraos-core.exe → protect <prefix>
        if let Some(prefix) = exe.parent().and_then(Path::parent) {
            v.push(prefix.to_path_buf());
        }
    }
    v
}

/// Normalised + checked path (and its real target if it exists: junctions cannot sneak around).
pub(crate) fn checked(p: &str) -> Result<PathBuf, ActError> {
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
            "files.list" | "files.find" | "files.open" | "files.mkdir" | "files.duplicates"
            | "files.largest" => {
                let x = checked(str_arg(&args, "path")?)?;
                format!("{} {}", b.name, x.display())
            }
            // ADR 0055: Office files go through the same protected-places check.
            "office.excel_read" => {
                let x = checked(str_arg(&args, "path")?)?;
                format!("read {} from {}", str_arg(&args, "range")?, x.display())
            }
            "office.excel_write" => {
                let x = checked(str_arg(&args, "path")?)?;
                let vals = crate::office::write_values(&args)?;
                let first: Vec<String> = vals
                    .iter()
                    .take(3)
                    .map(|r| r.iter().take(6).map(|c| crate::perceive::clip(&c.to_string(), 20)).collect::<Vec<_>>().join(", "))
                    .collect();
                format!(
                    "write {} row(s) at {}{} in {}{}:\n{}",
                    vals.len(),
                    str_arg(&args, "range")?,
                    args.get("sheet").and_then(Value::as_str).map(|s| format!(" (sheet {s})")).unwrap_or_default(),
                    x.display(),
                    args.get("save_as").and_then(Value::as_str).map(|s| format!(", saved as {s}")).unwrap_or_default(),
                    first.join("\n")
                )
            }
            "skill.export" => format!(
                "export the recorded skill '{}' to a file{}",
                str_arg(&args, "name")?,
                if args.get("include_defaults").and_then(Value::as_bool).unwrap_or(false) { " (with the text it typed)" } else { "" }
            ),
            "skill.import" => format!("import a skill from {} (you review it first)", checked(str_arg(&args, "path")?)?.display()),
            "office.word_read" => {
                let x = checked(str_arg(&args, "path")?)?;
                format!("read the text of {}", x.display())
            }
            "office.ppt_read" => {
                let x = checked(str_arg(&args, "path")?)?;
                format!("read the slides of {}", x.display())
            }
            // ADR 0063: adding shows its own dialog with the schedule; the skill it schedules
            // is checked (paths, credentials) before that dialog.
            "routine.add" => {
                let r = crate::routines::from_args(&args, 0, "")?;
                plan(&json!({"name": r.skill, "args": crate::routines::sample_args(&r)}))?;
                format!("add routine '{}': {} {}", r.name, r.skill, r.schedule.describe())
            }
            "routine.run" => format!("run routine '{}' now", str_arg(&args, "name")?),
            "routine.remove" => format!("remove routine '{}'", str_arg(&args, "name")?),
            "routine.list" | "routine.summary" | "layout.list" => b.about.to_string(),
            "screen.find" => format!("find \"{}\" in all open windows", crate::perceive::clip(str_arg(&args, "query")?, 60)),
            "window.snap" => format!("snap window {} to the {}", args.get("window").map(|w| w.to_string()).unwrap_or_default(), str_arg(&args, "to")?),
            "files.recent" | "disk.usage" => b.about.to_string(),
            "files.read_text" => format!("read the text of {}", checked(str_arg(&args, "path")?)?.display()),
            "routine.pause" => format!("pause routine '{}'", str_arg(&args, "name")?),
            "routine.resume" => format!("resume routine '{}'", str_arg(&args, "name")?),
            "notify.show" => {
                let (title, text) = notify_args(&args)?;
                format!("show the notification \"{}: {}\"", title, crate::perceive::clip(&text, 80))
            }
            "layout.save" => format!("save the window layout '{}'", str_arg(&args, "name")?),
            "layout.restore" => format!("arrange the windows as in layout '{}'", str_arg(&args, "name")?),
            "layout.delete" => format!("delete the window layout '{}'", str_arg(&args, "name")?),
            "office.ppt_add_slide" => {
                let x = checked(str_arg(&args, "path")?)?;
                let (title, bullets) = crate::office::slide_args(&args)?;
                format!(
                    "add a slide \"{}\" with {} bullet(s) to {}{}",
                    crate::perceive::clip(&title, 80),
                    bullets.len(),
                    x.display(),
                    args.get("save_as").and_then(Value::as_str).map(|s| format!(", saved as {s}")).unwrap_or_default()
                )
            }
            "office.word_append" => {
                let x = checked(str_arg(&args, "path")?)?;
                let t = args.get("text").and_then(Value::as_str).unwrap_or("");
                format!(
                    "add {} character(s) to the end of {}{}:\n\"{}\"",
                    t.chars().count(),
                    x.display(),
                    args.get("save_as").and_then(Value::as_str).map(|s| format!(", saved as {s}")).unwrap_or_default(),
                    crate::perceive::clip(t, 160)
                )
            }
            "office.export_pdf" => {
                let x = checked(str_arg(&args, "path")?)?;
                let out = crate::office::pdf_target(&x, args.get("out").and_then(Value::as_str));
                format!("save {} as PDF {}", x.display(), checked(&out.to_string_lossy())?.display())
            }
            // ADR 0054: the plan itself is the description the user approves.
            "files.organize" => {
                let x = checked(str_arg(&args, "path")?)?;
                let (moves, _) = organize_plan(&x, organize_by(&args)?)?;
                let folders: std::collections::BTreeSet<&str> =
                    moves.iter().map(|(_, f)| f.as_str()).collect();
                format!(
                    "move {} file(s) in {} into {} folder(s): {}",
                    moves.len(),
                    x.display(),
                    folders.len(),
                    folders.into_iter().collect::<Vec<_>>().join(", ")
                )
            }
            "files.bulk_rename" => {
                let x = checked(str_arg(&args, "path")?)?;
                let plan = bulk_rename_plan(&x, &args)?;
                let shown: Vec<String> = plan
                    .iter()
                    .take(8)
                    .map(|(a, b)| format!("{a} → {b}"))
                    .collect();
                format!(
                    "rename {} file(s) in {}:\n{}{}",
                    plan.len(),
                    x.display(),
                    shown.join("\n"),
                    if plan.len() > 8 { "\n…" } else { "" }
                )
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
            "whatsapp.send" => {
                let w = wa_args(&args)?;
                format!(
                    "send a WhatsApp message to +{}{}{}",
                    w.phone,
                    w.text
                        .as_ref()
                        .map(|t| format!(":\n\"{}\"", crate::perceive::clip(t, 120)))
                        .unwrap_or_default(),
                    w.file
                        .as_ref()
                        .map(|f| format!("\nwith the file {}", f.display()))
                        .unwrap_or_default()
                )
            }
            "dev.check" => {
                let n = parse_expect(&args)?.len();
                let w = args.get("window").ok_or_else(|| bad("'window' is required"))?;
                let h = crate::perceive::check_window(w)?;
                format!(
                    "check {n} expectation(s) in \"{}\"",
                    crate::perceive::clip(&crate::winmgr::title(h), 50)
                )
            }
            _ => b.name.to_string(),
        };
        let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
        let class = if b.name == "whatsapp.send" && args.get("file").is_some_and(|f| !f.is_null()) {
            Class::FileSend
        } else if matches!(b.name, "files.organize" | "files.bulk_rename") && !apply {
            Class::Observe // a preview changes nothing
        } else {
            b.class
        };
        return Ok(Plan {
            name,
            class,
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
        "files.find" => files_find(&args),
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
        "whatsapp.send" => whatsapp_send(&args),
        "dev.check" => dev_check(&args),
        n @ ("task.save" | "task.load" | "task.list" | "task.done") => crate::tasks::run(n, &args),
        "skill.export" => crate::sharing::export(&args),
        "skill.import" => crate::sharing::import(&args),
        "files.duplicates" => files_duplicates(&args),
        "files.largest" => files_largest(&args),
        "files.organize" => files_organize(&args),
        "files.bulk_rename" => files_bulk_rename(&args),
        "office.excel_read" => crate::office::excel_read(&args),
        "office.excel_write" => crate::office::excel_write(&args),
        "office.export_pdf" => crate::office::export_pdf(&args),
        "office.word_read" => crate::office::word_read(&args),
        "office.word_append" => crate::office::word_append(&args),
        "office.ppt_read" => crate::office::ppt_read(&args),
        "office.ppt_add_slide" => crate::office::ppt_add_slide(&args),
        n if n.starts_with("routine.") => crate::routines::run(n, &args),
        n if n.starts_with("layout.") => crate::layouts::run(n, &args),
        "screen.find" => screen_find(&args),
        "window.snap" => crate::everyday::window_snap(&args),
        "files.recent" => crate::everyday::files_recent(&args),
        "files.read_text" => crate::everyday::files_read_text(&args),
        "disk.usage" => crate::everyday::disk_usage(),
        "notify.show" => {
            let (title, text) = notify_args(&args)?;
            let shown = crate::safety::notify_waiting(&title, &text);
            Ok(json!({"text": if shown { format!("ok notification shown: {title}") } else { "the XtraOS tray icon is not available: nothing shown".into() }, "shown": shown}))
        }
        n => Err(bad(format!("unknown skill '{n}'"))),
    }
}

// ---------------------------------------------------------------------------------------------
// dev.check (P7-M3, ADR 0052)

#[derive(Clone, Debug, PartialEq)]
pub enum Expect {
    Text(String),
    NoText(String),
    /// An element whose name contains `name` (role exact when given); optionally its value
    /// contains `value` and its enabled state is `enabled`.
    Element {
        name: String,
        role: Option<String>,
        value: Option<String>,
        enabled: Option<bool>,
    },
    Title(String),
    /// The window visibly changed during the check (frame_diff, ≥ `min_pct` % of the window;
    /// default 0.1: above a blinking caret, below a small text update).
    Changed { min_pct: f64 },
}

impl Expect {
    fn label(&self) -> String {
        match self {
            Expect::Text(t) => format!("text \"{}\"", crate::perceive::clip(t, 60)),
            Expect::NoText(t) => format!("no text \"{}\"", crate::perceive::clip(t, 60)),
            Expect::Element { name, role, value, enabled } => format!(
                "element {}\"{}\"{}{}",
                role.as_ref().map(|r| format!("{r} ")).unwrap_or_default(),
                crate::perceive::clip(name, 60),
                value.as_ref().map(|v| format!(" = \"{}\"", crate::perceive::clip(v, 40))).unwrap_or_default(),
                match enabled {
                    Some(true) => " enabled",
                    Some(false) => " disabled",
                    None => "",
                }
            ),
            Expect::Title(t) => format!("title \"{}\"", crate::perceive::clip(t, 60)),
            Expect::Changed { .. } => "window changed".into(),
        }
    }
}

/// `expect` → checks; every entry needs exactly one of text / no_text / element / title /
/// changed. `value` and `enabled` refine an element check.
pub fn parse_expect(args: &Value) -> Result<Vec<Expect>, ActError> {
    let list = args
        .get("expect")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= 20)
        .ok_or_else(|| bad("'expect' is required: 1..20 checks like {\"text\":\"Saved\"}"))?;
    list.iter()
        .map(|e| {
            let s = |k: &str| {
                e.get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.trim().is_empty())
                    .map(String::from)
            };
            let given = ["text", "no_text", "element", "title", "changed"]
                .iter()
                .filter(|k| e.get(**k).is_some())
                .count();
            if given != 1 {
                return Err(bad(
                    "each check needs exactly one of text, no_text, element (+ role, value, enabled), title, changed",
                ));
            }
            if e.get("changed").is_none() && e.get("min_pct").is_some() {
                return Err(bad("'min_pct' goes with 'changed'"));
            }
            if e.get("element").is_none() && (e.get("value").is_some() || e.get("enabled").is_some()) {
                return Err(bad("'value' and 'enabled' go with 'element'"));
            }
            Ok(if let Some(t) = s("text") {
                Expect::Text(t)
            } else if let Some(t) = s("no_text") {
                Expect::NoText(t)
            } else if let Some(n) = s("element") {
                let enabled = match e.get("enabled") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_bool().ok_or_else(|| bad("'enabled' must be true or false"))?),
                };
                let value = match e.get("value") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_str().map(String::from).ok_or_else(|| bad("'value' must be a string"))?),
                };
                Expect::Element { name: n, role: s("role"), value, enabled }
            } else if let Some(t) = s("title") {
                Expect::Title(t)
            } else if e.get("changed") == Some(&Value::Bool(true)) {
                let min_pct = match e.get("min_pct") {
                    None | Some(Value::Null) => 0.1,
                    Some(v) => v
                        .as_f64()
                        .filter(|p| (0.01..=100.0).contains(p))
                        .ok_or_else(|| bad("'min_pct' must be 0.01..=100"))?,
                };
                Expect::Changed { min_pct }
            } else {
                return Err(bad("a check's text must be a non-empty string ('changed' must be true)"));
            })
        })
        .collect()
}

/// One check against what the window shows (pure): (pass, detail). `changed`: the window
/// visibly changed since the check started.
pub fn verdict(
    e: &Expect,
    text: &str,
    els: &[crate::perceive::ElemView],
    title: &str,
    changed: bool,
) -> (bool, String) {
    let low = text.to_lowercase();
    let clip = crate::perceive::clip;
    match e {
        Expect::Text(t) => {
            let ok = low.contains(&t.to_lowercase());
            (ok, if ok { "found".into() } else { "not in the window's text".into() })
        }
        Expect::NoText(t) => {
            let ok = !low.contains(&t.to_lowercase());
            (ok, if ok { "absent".into() } else { "present in the window's text".into() })
        }
        Expect::Element { name, role, value, enabled } => {
            let n = name.to_lowercase();
            let role_ok = |r: &str| role.as_ref().is_none_or(|x| x.eq_ignore_ascii_case(r));
            let named: Vec<&crate::perceive::ElemView> = els
                .iter()
                .filter(|v| v.name.to_lowercase().contains(&n) && role_ok(v.role))
                .collect();
            if named.is_empty() {
                let same: Vec<String> = els
                    .iter()
                    .filter(|v| !v.name.is_empty() && role_ok(v.role))
                    .take(5)
                    .map(|v| format!("{} \"{}\"", v.role, clip(&v.name, 30)))
                    .collect();
                return (false, if same.is_empty() { "no such element".into() } else { format!("not found; e.g. {}", same.join(", ")) });
            }
            let state_ok = |v: &crate::perceive::ElemView| {
                value.as_ref().is_none_or(|want| {
                    v.value.as_deref().is_some_and(|got| got.to_lowercase().contains(&want.to_lowercase()))
                }) && enabled.is_none_or(|want| v.enabled == want)
            };
            match named.iter().find(|v| state_ok(v)) {
                Some(v) => (true, format!("{} \"{}\"{}", v.role, clip(&v.name, 40), v.value.as_ref().map(|x| format!(" = \"{}\"", clip(x, 40))).unwrap_or_default())),
                None => {
                    let v = named[0];
                    (false, format!(
                        "{} \"{}\" is{}{}",
                        v.role,
                        clip(&v.name, 40),
                        match &v.value { Some(x) => format!(" = \"{}\"", clip(x, 40)), None => " (no value)".into() },
                        if v.enabled { ", enabled" } else { ", disabled" }
                    ))
                }
            }
        }
        Expect::Title(t) => {
            let ok = title.to_lowercase().contains(&t.to_lowercase());
            (ok, if ok { "matches".into() } else { format!("title is \"{}\"", clip(title, 60)) })
        }
        Expect::Changed { .. } => (changed, if changed { "changed".into() } else { "no visible change".into() }),
    }
}

fn dev_check(args: &Value) -> Result<Value, ActError> {
    let checks = parse_expect(args)?;
    let h = crate::perceive::check_window(args.get("window").ok_or_else(|| bad("'window' is required"))?)?;
    let timeout = std::time::Duration::from_millis(
        args.get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(3_000)
            .min(30_000),
    );
    let shot = args
        .get("screenshot_on_fail")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let t0 = std::time::Instant::now();
    // `changed`: a fresh frame baseline now; any later frame that differs latches it.
    let wref = crate::winmgr::window_ref(h);
    let change_pct = checks.iter().find_map(|c| match c {
        Expect::Changed { min_pct } => Some(*min_pct),
        _ => None,
    });
    let want_change = change_pct.is_some();
    // A window that cannot be captured right now (minimized) fails only the `changed` check,
    // with the reason; the baseline is retried while the check runs.
    let mut baseline: Result<(), String> = Err("not taken".into());
    let mut changed = false;
    let results = loop {
        crate::safety::check()?;
        if !crate::winmgr::exists(h) {
            return Err(ActError {
                code: "WINDOW_NOT_FOUND",
                message: "the window closed during the check".into(),
                hint: "list_windows, then check again.",
            });
        }
        if want_change && !changed {
            if baseline.is_err() {
                baseline = crate::framediff::frame_diff(&json!({"window": wref, "reset": true, "min_pct": change_pct}))
                    .map(|_| ())
                    .map_err(|e| e.message);
            } else {
                changed = crate::framediff::frame_diff(&json!({"window": wref, "min_pct": change_pct}))
                    .map(|r| r["changed"] == true)
                    .unwrap_or(false);
            }
        }
        let (text, els) = crate::perceive::check_view(h)?;
        let title = crate::winmgr::title(h);
        let r: Vec<(String, bool, String)> = checks
            .iter()
            .map(|c| {
                let (ok, mut d) = verdict(c, &text, &els, &title, changed);
                if let (Expect::Changed { .. }, Err(why)) = (c, &baseline) {
                    d = format!("could not watch the window: {why}");
                }
                (c.label(), ok, d)
            })
            .collect();
        if r.iter().all(|x| x.1) || t0.elapsed() >= timeout {
            break r;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    };
    let pass = results.iter().all(|x| x.1);
    let mut shot_path = None;
    if !pass && shot {
        if let Ok((img, _, _)) = crate::capture::grab(&json!({"window": crate::winmgr::window_ref(h)})) {
            let (w, hh, rgb) = crate::capture::downscale_rgb(&img, 1.0);
            if let Ok(png) = crate::capture::png_rgb(w, hh, &rgb) {
                let dir = std::env::temp_dir().join("xtraos-checks");
                let file = dir.join(format!(
                    "check-{}.png",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                ));
                if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&file, png).is_ok() {
                    shot_path = Some(file.display().to_string());
                }
            }
        }
    }
    let mut lines: Vec<String> = results
        .iter()
        .map(|(l, ok, d)| format!("{} {l} — {d}", if *ok { "PASS" } else { "FAIL" }))
        .collect();
    let passed = results.iter().filter(|x| x.1).count();
    lines.push(format!(
        "-- {} {passed}/{} in {} ms{}",
        if pass { "PASS" } else { "FAIL" },
        results.len(),
        t0.elapsed().as_millis(),
        shot_path.as_ref().map(|p| format!("; screenshot {p}")).unwrap_or_default()
    ));
    Ok(json!({
        "pass": pass,
        "text": lines.join("\n"),
        "checks": results.iter().map(|(l, ok, d)| json!({"check": l, "pass": ok, "detail": d})).collect::<Vec<_>>(),
        "screenshot": shot_path,
        "ms": t0.elapsed().as_millis() as u64,
    }))
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

/// Last-modified time as unix ms (0 if unknown).
fn modified_ms(md: &std::fs::Metadata) -> u64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// "YYYY-MM-DD HH:MM", local time.
fn modified_text(ms: u64) -> String {
    let t = crate::viewer::local_time(ms);
    t.get(..16).unwrap_or(&t).to_string()
}

fn files_list(a: &Value) -> Result<Value, ActError> {
    let d = checked(str_arg(a, "path")?)?;
    let pat = a.get("pattern").and_then(Value::as_str).unwrap_or("*");
    let newest = match a.get("sort").and_then(Value::as_str).unwrap_or("name") {
        "name" => false,
        "newest" => true,
        s => return Err(bad(format!("sort must be name or newest, not '{s}'"))),
    };
    let rd = std::fs::read_dir(&d).map_err(|e| bad(format!("{}: {e}", d.display())))?;
    let mut rows: Vec<(bool, String, u64, u64)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let md = e.metadata().ok()?;
            wildcard(pat, &n).then_some((md.is_dir(), n, md.len(), modified_ms(&md)))
        })
        .collect();
    if newest {
        rows.sort_by_key(|r| std::cmp::Reverse(r.3));
    } else {
        rows.sort_by(|x, y| {
            y.0.cmp(&x.0)
                .then(x.1.to_lowercase().cmp(&y.1.to_lowercase()))
        });
    }
    let total = rows.len();
    let lines: Vec<String> = rows
        .iter()
        .take(200)
        .map(|(dir, n, s, m)| {
            if *dir {
                format!("{n}/  {}", modified_text(*m))
            } else {
                format!("{n}  {}  {}", size(*s), modified_text(*m))
            }
        })
        .collect();
    Ok(
        json!({"text": format!("{}\n{}\n-- {total} items{}", d.display(), lines.join("\n"),
        if total > 200 { " (first 200)" } else { "" }), "items": total}),
    )
}

/// Files whose name matches `pattern` in `path` and its subfolders, newest first. Stays out of
/// protected and hidden folders (AppData) and links, and stops after FIND_BUDGET.
fn files_find(a: &Value) -> Result<Value, ActError> {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const REPARSE: u32 = 0x400;
    const FIND_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
    let root = checked(str_arg(a, "path")?)?;
    let pat = str_arg(a, "pattern")?;
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    if !root.is_dir() {
        return Err(bad(format!("{} is not a folder", root.display())));
    }
    let own = own_dirs();
    let t0 = std::time::Instant::now();
    // Breadth-first: Desktop, Downloads and Documents are scanned before deep project trees,
    // so a budget cut loses the least likely places (live: depth-first spent 5 s in one tree).
    let mut queue = std::collections::VecDeque::from([(root.clone(), 0)]);
    let (mut hits, mut scanned, mut cut) = (Vec::new(), 0u64, false);
    while let Some((dir, depth)) = queue.pop_front() {
        if t0.elapsed() > FIND_BUDGET {
            cut = true;
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            scanned += 1;
            let Ok(md) = e.metadata() else { continue };
            let attrs = md.file_attributes();
            let path = e.path();
            if md.is_dir() {
                // Tool and app-data folders hold no user documents: .git, .cargo, node_modules,
                // AppData (live: its Recent folder filled the results with .lnk shortcuts; it
                // is not always marked hidden).
                let name = e.file_name().to_string_lossy().to_lowercase();
                if depth < 12
                    && attrs & (HIDDEN | REPARSE) == 0
                    && !name.starts_with('.')
                    && name != "node_modules"
                    && name != "appdata"
                    && protected_reason(&path, &own).is_none()
                {
                    queue.push_back((path, depth + 1));
                }
            } else if wildcard(pat, &e.file_name().to_string_lossy()) {
                hits.push((modified_ms(&md), md.len(), path));
            }
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    let total = hits.len();
    let lines: Vec<String> = hits
        .iter()
        .take(limit)
        .map(|(m, s, p)| format!("{}  {}  {}", modified_text(*m), size(*s), p.display()))
        .collect();
    let mut foot = format!("-- {total} match(es), newest first; {scanned} items scanned");
    if cut {
        foot.push_str("; time budget reached (search a smaller folder for the rest)");
    }
    Ok(
        json!({"text": format!("{} \"{pat}\"\n{}\n{foot}", root.display(), lines.join("\n")),
              "matches": total, "complete": !cut}),
    )
}

/// Files that run code when "opened": never opened with `app: true` (that would be launching a
/// program by the back door; launch_app is the explicit way). Pure.
pub fn runs_code(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    [
        "exe", "com", "bat", "cmd", "ps1", "psm1", "psd1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi", "msp", "msix",
        "appx", "scr", "lnk", "url", "hta", "cpl", "jar", "reg", "py", "pyw", "appref-ms", "msc", "pif", "inf", "sys", "dll",
        "application", "gadget", "settingcontent-ms", "library-ms", "search-ms",
    ]
    .contains(&ext.as_str())
}

/// `files.open {path, app: true}` (D1 demo fix): open a document in its default app; returns
/// the app's window when one with the file's name appears within 15 s.
fn open_with_app(x: &Path) -> Result<Value, ActError> {
    if x.is_dir() {
        return Err(bad("app: true opens files; a folder opens in Explorer without it"));
    }
    if runs_code(x) {
        return Err(blocked(format!(
            "{} is a program or script; files.open does not run code (use launch_app, which asks the user where the policy says so)",
            x.display()
        )));
    }
    let before = crate::winmgr::listable_windows();
    let file = windows::core::HSTRING::from(x.as_os_str());
    let r = unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            &file,
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if (r.0 as isize) <= 32 {
        return Err(ActError {
            code: "NEEDS_HUMAN",
            message: format!("Windows could not open {} (no app for this file type?)", x.display()),
            hint: "Ask the user which app should open it, or use launch_app with that app.",
        });
    }
    let stem = x.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let t0 = std::time::Instant::now();
    while t0.elapsed() < std::time::Duration::from_secs(15) {
        std::thread::sleep(std::time::Duration::from_millis(300));
        crate::safety::check()?;
        // A new window with the file's name first; else one that already existed (the file
        // was open already and the app just came forward).
        let wins = crate::winmgr::listable_windows();
        let named = |h: &crate::input::HWND| crate::winmgr::title(*h).to_lowercase().contains(&stem);
        if let Some(h) = wins.iter().find(|h| !before.contains(h) && named(h)).or_else(|| wins.iter().find(|h| named(h))).copied() {
            return Ok(json!({
                "text": format!("ok opened {} in {} ({})", x.display(), crate::winmgr::process_name(crate::winmgr::pid_of(h)), crate::winmgr::window_ref(h)),
                "ref": crate::winmgr::window_ref(h),
                "title": crate::winmgr::title(h),
            }));
        }
    }
    Ok(json!({"text": format!("ok asked Windows to open {}; no window with its name seen within 15 s (list_windows to find it)", x.display()), "ref": null}))
}

fn files_open(a: &Value) -> Result<Value, ActError> {
    let x = checked(str_arg(a, "path")?)?;
    if !x.exists() {
        return Err(bad(format!("{} does not exist", x.display())));
    }
    if a.get("app").and_then(Value::as_bool).unwrap_or(false) {
        return open_with_app(&x);
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

// ---------------------------------------------------------------------------------------------
// Housekeeping (P7-M2, ADR 0054)

/// Every file under `root` (breadth-first, like files.find: no hidden folders, AppData,
/// node_modules, links or protected places): (path, size, modified ms), and whether the time
/// budget cut the walk short.
pub(crate) fn walk_files(root: &Path, budget: std::time::Duration) -> (Vec<(PathBuf, u64, u64)>, bool) {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const REPARSE: u32 = 0x400;
    let own = own_dirs();
    let t0 = std::time::Instant::now();
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0)]);
    let (mut out, mut cut) = (Vec::new(), false);
    while let Some((dir, depth)) = queue.pop_front() {
        if t0.elapsed() > budget {
            cut = true;
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            let attrs = md.file_attributes();
            let path = e.path();
            if md.is_dir() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if depth < 12
                    && attrs & (HIDDEN | REPARSE) == 0
                    && !name.starts_with('.')
                    && name != "node_modules"
                    && name != "appdata"
                    && protected_reason(&path, &own).is_none()
                {
                    queue.push_back((path, depth + 1));
                }
            } else if attrs & REPARSE == 0 {
                out.push((path, md.len(), modified_ms(&md)));
            }
        }
    }
    (out, cut)
}

fn files_largest(a: &Value) -> Result<Value, ActError> {
    let root = checked(str_arg(a, "path")?)?;
    if !root.is_dir() {
        return Err(bad(format!("{} is not a folder", root.display())));
    }
    let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100) as usize;
    let (mut files, cut) = walk_files(&root, std::time::Duration::from_secs(15));
    let total: u64 = files.iter().map(|f| f.1).sum();
    let count = files.len();
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    let lines: Vec<String> = files
        .iter()
        .take(limit)
        .map(|(p, s, m)| format!("{:>9}  {}  {}", size(*s), modified_text(*m), p.display()))
        .collect();
    let mut foot = format!("-- {count} files, {} in total", size(total));
    if cut {
        foot.push_str("; time budget reached (only part of the folder was scanned)");
    }
    Ok(json!({
        "text": format!("largest files in {}\n{}\n{foot}", root.display(), lines.join("\n")),
        "files": count, "bytes": total, "complete": !cut,
    }))
}

/// Byte-for-byte equality of two files, streamed.
fn same_content(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(mut x), Ok(mut y)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let (mut bx, mut by) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let (Ok(n), Ok(m)) = (read_full(&mut x, &mut bx), read_full(&mut y, &mut by)) else {
            return false;
        };
        if n != m || bx[..n] != by[..m] {
            return false;
        }
        if n == 0 {
            return true;
        }
    }
    fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut got = 0;
        while got < buf.len() {
            match f.read(&mut buf[got..])? {
                0 => break,
                n => got += n,
            }
        }
        Ok(got)
    }
}

fn head_hash(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0;
    while got < buf.len() {
        match f.read(&mut buf[got..]).ok()? {
            0 => break,
            n => got += n,
        }
    }
    Some(crate::actionlog::sha256_hex(&buf[..got]))
}

/// Groups of identical files among (path, size, modified): same size → same first 64 KB →
/// byte-for-byte equal. Each group newest first; groups with the most wasted bytes first.
pub fn duplicate_groups(files: &[(PathBuf, u64, u64)], deadline: std::time::Instant) -> (Vec<Vec<(PathBuf, u64, u64)>>, bool) {
    let mut by_size: HashMap<u64, Vec<&(PathBuf, u64, u64)>> = HashMap::new();
    for f in files {
        by_size.entry(f.1).or_default().push(f);
    }
    let mut groups = Vec::new();
    let mut cut = false;
    let mut sizes: Vec<u64> = by_size.iter().filter(|(_, v)| v.len() > 1).map(|(s, _)| *s).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a)); // biggest first: the most space to win
    'sizes: for s in sizes {
        let mut by_head: HashMap<String, Vec<&(PathBuf, u64, u64)>> = HashMap::new();
        for f in &by_size[&s] {
            if let Some(h) = head_hash(&f.0) {
                by_head.entry(h).or_default().push(*f);
            }
        }
        for (_, mut rest) in by_head.into_iter().filter(|(_, v)| v.len() > 1) {
            while rest.len() > 1 {
                if std::time::Instant::now() > deadline {
                    cut = true;
                    break 'sizes;
                }
                let first = rest.remove(0);
                let (same, other): (Vec<_>, Vec<_>) =
                    rest.into_iter().partition(|f| same_content(&first.0, &f.0));
                if !same.is_empty() {
                    let mut g: Vec<(PathBuf, u64, u64)> = std::iter::once(first)
                        .chain(same)
                        .map(|f| (f.0.clone(), f.1, f.2))
                        .collect();
                    g.sort_by_key(|f| std::cmp::Reverse(f.2));
                    groups.push(g);
                }
                rest = other;
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g[0].1 * (g.len() as u64 - 1)));
    (groups, cut)
}

fn files_duplicates(a: &Value) -> Result<Value, ActError> {
    let root = checked(str_arg(a, "path")?)?;
    if !root.is_dir() {
        return Err(bad(format!("{} is not a folder", root.display())));
    }
    let min = a.get("min_kb").and_then(Value::as_u64).unwrap_or(100) * 1024;
    let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100) as usize;
    let t0 = std::time::Instant::now();
    let (files, walk_cut) = walk_files(&root, std::time::Duration::from_secs(10));
    let files: Vec<_> = files.into_iter().filter(|f| f.1 >= min.max(1)).collect();
    let (groups, cmp_cut) = duplicate_groups(&files, t0 + std::time::Duration::from_secs(20));
    let wasted: u64 = groups.iter().map(|g| g[0].1 * (g.len() as u64 - 1)).sum();
    let mut text = format!("identical files in {} (≥ {} each)\n", root.display(), size(min.max(1)));
    for g in groups.iter().take(limit) {
        text.push_str(&format!(
            "{} copies × {} ({} wasted):\n",
            g.len(),
            size(g[0].1),
            size(g[0].1 * (g.len() as u64 - 1))
        ));
        for (i, (p, _, m)) in g.iter().enumerate() {
            text.push_str(&format!(
                "  {} {}  {}\n",
                if i == 0 { "newest" } else { "      " },
                modified_text(*m),
                p.display()
            ));
        }
    }
    text.push_str(&format!(
        "-- {} group(s), {} could be freed; {} files compared",
        groups.len(),
        size(wasted),
        files.len()
    ));
    if walk_cut || cmp_cut {
        text.push_str("; time budget reached (scan a smaller folder for the rest)");
    }
    text.push_str("\nTo remove copies: files.delete (asks the user; Recycle Bin).");
    Ok(json!({
        "text": text,
        "groups": groups.iter().take(limit).map(|g| g.iter().map(|f| f.0.display().to_string()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "wasted_bytes": wasted,
        "complete": !(walk_cut || cmp_cut),
    }))
}

/// The folder a file goes to when organizing by type (pure).
pub fn type_folder(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "svg" | "tif" | "tiff" | "ico" | "avif" => "Images",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv" | "odt" | "ods" | "rtf" | "epub" => "Documents",
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" => "Videos",
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "wma" | "opus" => "Audio",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "iso" => "Archives",
        "exe" | "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" => "Installers",
        "js" | "ts" | "py" | "rs" | "json" | "html" | "css" | "java" | "c" | "cpp" | "h" | "go" | "sql" | "sh" | "ps1" | "ipynb" | "xml" | "yaml" | "yml" => "Code",
        _ => "Other",
    }
}

/// Downloads still in progress, never moved (pure).
// ---------------------------------------------------------------------------------------------
// screen.find (ADR 0067)

const SCREEN_FIND_LINES: usize = 5;

/// The combined result text (pure). `hits`: (window header, matching element lines, total).
pub fn screen_find_text(query: &str, hits: &[(String, Vec<String>, u64)], searched: usize, skipped: usize, out_of_time: bool) -> String {
    let mut s = String::new();
    for (head, lines, total) in hits {
        s.push_str(&format!("{head}: {total} match(es)\n"));
        for l in lines.iter().take(SCREEN_FIND_LINES) {
            s.push_str(&format!("  {l}\n"));
        }
        if *total as usize > SCREEN_FIND_LINES {
            s.push_str("  … more: use find in that window\n");
        }
    }
    let n: u64 = hits.iter().map(|h| h.2).sum();
    s.push_str(&format!(
        "-- \"{query}\": {n} match(es) in {} window(s); searched {searched}{}{}",
        hits.len(),
        if skipped > 0 { format!(", {skipped} skipped (not responding or closing)") } else { String::new() },
        if out_of_time { "; time budget reached: raise timeout_ms to search the rest" } else { "" }
    ));
    if n == 0 {
        s.push_str("\nNothing found by name. Text drawn as pixels (games, canvas, images) needs ocr on a window.");
    }
    s
}

fn screen_find(a: &Value) -> Result<Value, ActError> {
    let query = str_arg(a, "query")?.trim().to_string();
    let budget = std::time::Duration::from_millis(a.get("timeout_ms").and_then(Value::as_u64).unwrap_or(5_000).clamp(500, 15_000));
    let max_windows = a.get("max_windows").and_then(Value::as_u64).unwrap_or(10).clamp(1, 40) as usize;
    let t0 = std::time::Instant::now();
    let mut hits = Vec::new();
    let (mut searched, mut skipped, mut out_of_time) = (0, 0, false);
    for h in crate::winmgr::listable_windows() {
        if crate::safety::is_own_window(h) || crate::winmgr::is_elevated(h) {
            continue;
        }
        if t0.elapsed() >= budget {
            out_of_time = true;
            break;
        }
        if hits.len() >= max_windows {
            break;
        }
        crate::safety::check()?;
        let r = crate::winmgr::window_ref(h);
        searched += 1;
        match crate::perceive::run("find", &json!({"window": r, "query": query})) {
            Ok(v) => {
                let total = v["matches"].as_u64().unwrap_or(0);
                if total > 0 {
                    let lines: Vec<String> = v["text"].as_str().unwrap_or("").lines().filter(|l| !l.starts_with("--")).map(String::from).collect();
                    let head = format!(
                        "{r} {} \"{}\"",
                        crate::winmgr::process_name(crate::winmgr::pid_of(h)),
                        crate::perceive::clip(&crate::winmgr::title(h), 50)
                    );
                    hits.push((head, lines, total));
                }
            }
            Err(e) if e.code == "STOPPED" => return Err(e),
            Err(_) => skipped += 1,
        }
    }
    let windows: Vec<Value> = hits.iter().map(|(h, _, n)| json!({"window": h.split(' ').next().unwrap_or(""), "matches": n})).collect();
    Ok(json!({
        "text": screen_find_text(&query, &hits, searched, skipped, out_of_time),
        "windows": windows,
        "searched": searched,
        "skipped": skipped,
        "complete": !out_of_time,
    }))
}

/// `notify.show` arguments (pure): title (default "XtraOS reminder", ≤ 60), text (≤ 250).
pub fn notify_args(a: &Value) -> Result<(String, String), ActError> {
    let text = a.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad("'text' is required"))?;
    if text.chars().count() > 250 {
        return Err(bad("'text' is limited to 250 characters"));
    }
    let title = a.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).unwrap_or("XtraOS reminder");
    if title.chars().count() > 60 {
        return Err(bad("'title' is limited to 60 characters"));
    }
    Ok((title.to_string(), text.to_string()))
}

pub fn unfinished(name: &str) -> bool {
    let n = name.to_lowercase();
    [".crdownload", ".part", ".partial", ".download", ".tmp"]
        .iter()
        .any(|e| n.ends_with(e))
}

fn organize_by(a: &Value) -> Result<&'static str, ActError> {
    match a.get("by").and_then(Value::as_str).unwrap_or("type") {
        "type" => Ok("type"),
        "month" => Ok("month"),
        o => Err(bad(format!("'by' must be type or month, not '{o}'"))),
    }
}

/// (file → folder name) moves and (file, why) skips for the files directly in `dir`.
fn organize_plan(dir: &Path, by: &str) -> Result<(Vec<(PathBuf, String)>, Vec<(String, String)>), ActError> {
    if !dir.is_dir() {
        return Err(bad(format!("{} is not a folder", dir.display())));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let rd = std::fs::read_dir(dir).map_err(|e| bad(format!("{}: {e}", dir.display())))?;
    let (mut moves, mut skips) = (Vec::new(), Vec::new());
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let m = modified_ms(&md);
        if unfinished(&name) {
            skips.push((name, "download not finished".into()));
            continue;
        }
        if now.saturating_sub(m) < 120_000 {
            skips.push((name, "changed in the last 2 minutes".into()));
            continue;
        }
        let folder = if by == "month" {
            modified_text(m).get(..7).unwrap_or("Unknown").to_string()
        } else {
            type_folder(&name).to_string()
        };
        if dir.join(&folder).join(&name).exists() {
            skips.push((name, format!("{folder}\\ already has a file with that name")));
            continue;
        }
        moves.push((e.path(), folder));
    }
    moves.sort();
    Ok((moves, skips))
}

fn files_organize(a: &Value) -> Result<Value, ActError> {
    let dir = checked(str_arg(a, "path")?)?;
    let by = organize_by(a)?;
    let apply = a.get("apply").and_then(Value::as_bool).unwrap_or(false);
    let (moves, skips) = organize_plan(&dir, by)?;
    let mut per: std::collections::BTreeMap<&str, usize> = Default::default();
    for (_, f) in &moves {
        *per.entry(f.as_str()).or_default() += 1;
    }
    let mut text = format!(
        "{} {} file(s) in {} by {by}:\n",
        if apply { "moved" } else { "would move" },
        moves.len(),
        dir.display()
    );
    for (f, n) in &per {
        text.push_str(&format!("  {f}\\  {n}\n"));
    }
    for (n, why) in skips.iter().take(20) {
        text.push_str(&format!("  skipped {n}: {why}\n"));
    }
    if apply && !moves.is_empty() {
        for f in per.keys() {
            let d = dir.join(f);
            checked(&d.to_string_lossy())?;
            std::fs::create_dir_all(&d).map_err(|e| internal(format!("mkdir {}: {e}", d.display())))?;
        }
        let (base, list) = (dir.clone(), moves.clone());
        shell_op(SAFE_FLAGS, move |op| unsafe {
            for (f, folder) in &list {
                op.MoveItem(&item(f)?, &item(&base.join(folder))?, PCWSTR::null(), None::<&IFileOperationProgressSink>)?;
            }
            Ok(())
        })?;
        text.push_str("-- done; Ctrl+Z in Explorer undoes it");
    } else if !apply {
        text.push_str("-- preview only: run again with apply: true to move them (the user is asked)");
    }
    Ok(json!({"text": text, "moved": if apply { moves.len() } else { 0 }, "planned": moves.len(), "skipped": skips.len()}))
}

/// Renames for names containing `find` (case-insensitive) → `replace` (pure). Errors on an
/// invalid or empty new name, two files ending with one name, or a name already taken.
pub fn rename_plan(names: &[String], find: &str, replace: &str) -> Result<Vec<(String, String)>, String> {
    if find.is_empty() {
        return Err("'find' must not be empty".into());
    }
    if replace.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("'replace' may not contain \\ / : * ? \" < > |".into());
    }
    let f = find.to_lowercase();
    let mut out = Vec::new();
    for n in names {
        let low = n.to_lowercase();
        if !low.contains(&f) {
            continue;
        }
        // replace every occurrence, case-insensitively, keeping the rest of the name as is
        let (mut new, mut rest, mut rest_low) = (String::new(), n.as_str(), low.as_str());
        while let Some(i) = rest_low.find(&f) {
            new.push_str(&rest[..i]);
            new.push_str(replace);
            rest = &rest[i + f.len()..];
            rest_low = &rest_low[i + f.len()..];
        }
        new.push_str(rest);
        let trimmed = new.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
            return Err(format!("\"{n}\" would get an empty name"));
        }
        if new != *n {
            out.push((n.clone(), new));
        }
    }
    let lower = |s: &str| s.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    for (_, b) in &out {
        if !seen.insert(lower(b)) {
            return Err(format!("two files would both be named \"{b}\""));
        }
    }
    let renamed: std::collections::HashSet<String> = out.iter().map(|(a, _)| lower(a)).collect();
    for (_, b) in &out {
        if names.iter().any(|n| lower(n) == lower(b)) && !renamed.contains(&lower(b)) {
            return Err(format!("\"{b}\" already exists (never overwrites)"));
        }
    }
    Ok(out)
}

fn bulk_rename_plan(dir: &Path, a: &Value) -> Result<Vec<(String, String)>, ActError> {
    if !dir.is_dir() {
        return Err(bad(format!("{} is not a folder", dir.display())));
    }
    let find = a.get("find").and_then(Value::as_str).unwrap_or("");
    let replace = a.get("replace").and_then(Value::as_str).ok_or_else(|| bad("'replace' is required (may be \"\")"))?;
    let pat = a.get("pattern").and_then(Value::as_str).unwrap_or("*");
    let names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| bad(format!("{}: {e}", dir.display())))?
        .flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| wildcard(pat, n))
        .collect();
    // Names of everything in the folder (subfolders too) count for "already exists".
    let mut all: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| bad(e.to_string()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    all.sort();
    let plan = rename_plan(&names, find, replace).map_err(bad)?;
    for (_, b) in &plan {
        let taken = all.iter().any(|n| n.eq_ignore_ascii_case(b));
        let freed = plan.iter().any(|(a, _)| a.eq_ignore_ascii_case(b));
        if taken && !freed {
            return Err(bad(format!("\"{b}\" already exists (never overwrites)")));
        }
    }
    Ok(plan)
}

fn files_bulk_rename(a: &Value) -> Result<Value, ActError> {
    let dir = checked(str_arg(a, "path")?)?;
    let apply = a.get("apply").and_then(Value::as_bool).unwrap_or(false);
    let plan = bulk_rename_plan(&dir, a)?;
    let lines: Vec<String> = plan.iter().take(50).map(|(x, y)| format!("  {x} → {y}")).collect();
    let mut text = format!(
        "{} {} file(s) in {}:\n{}\n",
        if apply { "renamed" } else { "would rename" },
        plan.len(),
        dir.display(),
        lines.join("\n")
    );
    if apply && !plan.is_empty() {
        let (base, list) = (dir.clone(), plan.clone());
        shell_op(SAFE_FLAGS, move |op| unsafe {
            for (x, y) in &list {
                op.RenameItem(&item(&base.join(x))?, &HSTRING::from(y.as_str()), None::<&IFileOperationProgressSink>)?;
            }
            Ok(())
        })?;
        text.push_str("-- done; Ctrl+Z in Explorer undoes it");
    } else if !apply {
        text.push_str("-- preview only: run again with apply: true to rename (the user is asked)");
    }
    Ok(json!({"text": text, "renamed": if apply { plan.len() } else { 0 }, "planned": plan.len()}))
}

// ---------------------------------------------------------------------------------------------
// whatsapp.send (P2-M4, ADR 0028)

pub struct WaArgs {
    pub phone: String,
    pub text: Option<String>,
    pub file: Option<PathBuf>,
    pub keep_open: bool,
}

/// Digits only, with the country code (8–15 digits).
pub fn wa_phone(raw: &str) -> Option<String> {
    let d: String = raw
        .chars()
        .filter(|c| !matches!(c, '+' | ' ' | '-' | '(' | ')'))
        .collect();
    (d.chars().all(|c| c.is_ascii_digit()) && (8..=15).contains(&d.len())).then_some(d)
}

/// Percent-encoding for a URL query value (UTF-8).
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn wa_args(a: &Value) -> Result<WaArgs, ActError> {
    let phone = wa_phone(str_arg(a, "phone")?)
        .ok_or_else(|| bad("'phone' needs the country code and 8-15 digits, e.g. 919876543210"))?;
    let text = a
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .map(String::from);
    if text.as_ref().is_some_and(|t| t.chars().count() > 4000) {
        return Err(bad("'text' is limited to 4,000 characters"));
    }
    let file = match a.get("file").and_then(Value::as_str) {
        Some(f) => {
            let p = checked(f)?;
            if !p.is_file() {
                return Err(bad(format!("{} is not an existing file", p.display())));
            }
            Some(p)
        }
        None => None,
    };
    if text.is_none() && file.is_none() {
        return Err(bad("give 'text' and/or 'file'"));
    }
    Ok(WaArgs {
        phone,
        text,
        file,
        keep_open: a.get("keep_open").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn call(tool: &str, args: Value) -> Result<Value, ActError> {
    crate::safety::check()?; // the kill switch stops the skill between steps
    crate::exec::dispatch(tool, &args)
}

/// First line of a find result matching `pred`, as (ref, line).
fn find_line(win: &str, query: &str, pred: impl Fn(&str) -> bool) -> Option<(String, String)> {
    let r = call("find", json!({"window": win, "query": query})).ok()?;
    r["text"]
        .as_str()?
        .lines()
        .find(|l| l.starts_with('e') && pred(l))
        .map(|l| (l.split(' ').next().unwrap_or("").to_string(), l.to_string()))
}

fn whatsapp_send(a: &Value) -> Result<Value, ActError> {
    use std::time::{Duration, Instant};
    let w = wa_args(a)?;
    let (browser, map_args) = crate::appmap::resolve_launch("whatsapp")
        .filter(|(_, args)| args.iter().any(|x| x.contains("web.whatsapp.com")))
        .ok_or_else(|| ActError {
            code: "ELEMENT_NOT_FOUND",
            message: "WhatsApp is not in the app map".into(),
            hint: "app_map set: service whatsapp, surface browser, location {browser: \"firefox\", url: \"https://web.whatsapp.com\"}.",
        })?;
    let url = format!(
        "https://web.whatsapp.com/send?phone={}{}",
        w.phone,
        match (&w.text, &w.file) {
            (Some(t), None) => format!("&text={}", url_encode(t)),
            _ => String::new(),
        }
    );
    let user_fg = crate::winmgr::foreground_root();
    // Reuse an open WhatsApp window (only one WhatsApp tab can be active); otherwise open one.
    let existing = crate::winmgr::listable_windows().into_iter().find(|&h| {
        crate::winmgr::title(h).to_lowercase().contains("whatsapp")
            && matches!(
                crate::winmgr::process_name(crate::winmgr::pid_of(h))
                    .to_lowercase()
                    .as_str(),
                "firefox.exe" | "chrome.exe" | "msedge.exe" | "brave.exe"
            )
    });
    let (win, opened) = match existing {
        Some(h) => {
            let r = crate::winmgr::window_ref(h);
            call("focus_window", json!({"window": r}))?;
            call("press_keys", json!({"window": r, "keys": ["Ctrl+L"]}))?;
            call("type", json!({"window": r, "text": url, "submit": true}))?;
            (r, false)
        }
        None => {
            let mut args = map_args;
            args.pop(); // the map's start URL → the chat URL
            args.push(url.clone());
            let l = call(
                "launch_app",
                json!({"app": browser, "args": args, "timeout_ms": 20000, "reuse": false}),
            )?;
            (l["ref"].as_str().unwrap_or("").to_string(), true)
        }
    };
    // Close what this skill opened (never the user's WhatsApp window); give the foreground back.
    // Returns whether the window it opened is really closed.
    let finish = |opened: bool, win: &str| -> bool {
        let mut closed = false;
        if opened && !w.keep_open {
            closed = crate::winmgr::run("cleanup_one", &json!({"window": win}))
                .is_ok_and(|r| r["closed"] == 1);
        }
        if !user_fg.is_null()
            && crate::winmgr::exists(user_fg)
            && crate::winmgr::foreground_root() != user_fg
        {
            crate::winmgr::focus(user_fg);
        }
        closed
    };
    // Wait for the chat's compose box (Firefox exposes the page only while it is in front).
    let deadline = Instant::now() + Duration::from_secs(45);
    let compose = loop {
        let _ = call("focus_window", json!({"window": win}));
        if let Some((r, _)) = find_line(&win, "use here", |l| l.contains("Button \"Use here\"")) {
            let _ = call("click", json!({"ref": r}));
        }
        if find_line(&win, "shared via url is invalid", |_| true).is_some() {
            finish(opened, &win);
            return Err(bad(format!(
                "WhatsApp says +{} is not a valid WhatsApp number",
                w.phone
            )));
        }
        if let Some(found) = find_line(&win, "type a message to", |l| l.contains("Edit")) {
            break found;
        }
        if Instant::now() > deadline {
            finish(opened, &win);
            return Err(ActError {
                code: "TIMEOUT",
                message: "WhatsApp did not open the chat within 45 s".into(),
                hint: "Check that WhatsApp Web is signed in (QR code is a human step).",
            });
        }
        std::thread::sleep(Duration::from_millis(1000));
    };
    // Never send to the wrong chat: the compose label must name this number.
    let label_digits: String = compose
        .1
        .split(" = ")
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    let tail = &w.phone[w.phone.len().saturating_sub(10)..];
    if !label_digits.contains(tail) {
        finish(opened, &win);
        return Err(ActError {
            code: "NEEDS_HUMAN",
            message: "WhatsApp opened a chat that does not show this number; nothing was sent"
                .into(),
            hint: "The contact may be saved under a name only: ask the user.",
        });
    }
    let mut what = String::new();
    if let Some(f) = &w.file {
        let previous = crate::input::clipboard_set_files(std::slice::from_ref(f))?;
        let pasted = call("press_keys", json!({"window": win, "keys": ["Ctrl+V"]}));
        crate::input::clipboard_restore(previous);
        pasted?;
        let name = name_of(f);
        let t0 = Instant::now();
        while find_line(&win, "selected", |l| l.contains("Button \"Send")).is_none() {
            if t0.elapsed() > Duration::from_secs(15) {
                finish(opened, &win);
                return Err(internal("WhatsApp did not show the file preview"));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if let Some(t) = &w.text {
            call("type", json!({"window": win, "text": t}))?; // caption
        }
        call("press_keys", json!({"window": win, "keys": ["Enter"]}))?;
        what = format!("file {name}");
        let t0 = Instant::now();
        while find_line(&win, "selected", |l| l.contains("Button \"Send")).is_some() {
            if t0.elapsed() > Duration::from_secs(20) {
                finish(opened, &win);
                return Err(internal(
                    "the file preview did not close: not confirmed as sent",
                ));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    } else if let Some(t) = &w.text {
        // The URL put the text in the compose box; check, then send.
        if !compose
            .1
            .contains(&crate::perceive::clip(t, 60).replace('…', ""))
        {
            call(
                "press_keys",
                json!({"window": win, "keys": ["Ctrl+A", "Delete"]}),
            )?;
            call("type", json!({"window": win, "text": t}))?;
        }
        call("press_keys", json!({"window": win, "keys": ["Enter"]}))?;
        what = format!("text ({} chars)", t.chars().count());
        let t0 = Instant::now();
        loop {
            let c = find_line(&win, "type a message to", |l| l.contains("Edit"));
            if c.as_ref()
                .is_some_and(|(_, l)| !l.contains(" = \"") || l.contains(" = \"\""))
            {
                break;
            }
            if t0.elapsed() > Duration::from_secs(15) {
                finish(opened, &win);
                return Err(internal(
                    "the message is still in the compose box: not confirmed as sent",
                ));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    std::thread::sleep(Duration::from_millis(800));
    let closed = finish(opened, &win);
    let note = match (opened, w.keep_open, closed) {
        (false, _, _) => " (your WhatsApp window was reused and left open)".to_string(),
        (true, true, _) => format!(" (window {win} kept open)"),
        (true, false, true) => " (the window it opened is closed)".to_string(),
        (true, false, false) => format!(" (window {win} could not be closed: run cleanup)"),
    };
    Ok(json!({
        "text": format!("ok sent {what} to +{} on WhatsApp{note}", w.phone),
        "window_reused": !opened,
        "window_closed": closed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_is_recursive_newest_first_and_skips_hidden_folders() {
        use std::os::windows::process::CommandExt;
        let root = std::env::temp_dir().join(format!("xtraos-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let deep = root.join("a").join("b");
        let hidden = root.join("hid");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(&hidden).unwrap();
        let t = |d: u64| std::time::SystemTime::now() - std::time::Duration::from_secs(d);
        for (p, age) in [
            (root.join("old_resume.pdf"), 3000),
            (deep.join("new_resume.docx"), 10),
            (root.join("notes.txt"), 5),
            (hidden.join("secret_resume.pdf"), 1),
        ] {
            let f = std::fs::File::create(&p).unwrap();
            f.set_modified(t(age)).unwrap();
        }
        // attrib +h (no extra crate): the hidden folder must not be searched.
        std::process::Command::new("attrib")
            .arg("+h")
            .arg(&hidden)
            .creation_flags(0x0800_0000)
            .status()
            .unwrap();
        let r =
            files_find(&json!({"path": root.to_string_lossy(), "pattern": "*resume*"})).unwrap();
        let text = r["text"].as_str().unwrap();
        assert_eq!(r["matches"], 2, "{text}");
        let new = text.find("new_resume.docx").expect("deep file found");
        let old = text.find("old_resume.pdf").expect("top file found");
        assert!(new < old, "newest first: {text}");
        assert!(
            !text.contains("secret_resume"),
            "hidden folder skipped: {text}"
        );
        let _ = std::process::Command::new("attrib")
            .arg("-h")
            .arg(&hidden)
            .status();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("*.txt", "Report.TXT"));
        assert!(wildcard("rep?rt*", "report-final.pdf"));
        assert!(!wildcard("*.txt", "report.pdf"));
        assert!(wildcard("*", ""));
    }

    // P7-M2 (ADR 0054): housekeeping rules
    #[test]
    fn housekeeping_rules() {
        assert_eq!(type_folder("IMG_001.JPG"), "Images");
        assert_eq!(type_folder("report.pdf"), "Documents");
        assert_eq!(type_folder("setup.msi"), "Installers");
        assert_eq!(type_folder("archive.tar.gz"), "Archives");
        assert_eq!(type_folder("README"), "Other");
        assert!(unfinished("movie.mp4.crdownload") && unfinished("x.PART") && !unfinished("a.pdf"));
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let p = rename_plan(&n(&["IMG_1.jpg", "img_2.JPG", "notes.txt"]), "img_", "Trip ").unwrap();
        assert_eq!(p, vec![("IMG_1.jpg".into(), "Trip 1.jpg".into()), ("img_2.JPG".into(), "Trip 2.JPG".into())]);
        assert!(rename_plan(&n(&["a1.txt", "b1.txt"]), "a", "b").is_err(), "a1 → b1 collides with b1");
        assert!(rename_plan(&n(&["x.txt"]), "x.txt", "").is_err(), "empty name");
        assert!(rename_plan(&n(&["x.txt"]), "x", "a/b").is_err(), "path characters");
        assert!(rename_plan(&n(&["x.txt"]), "", "y").is_err());
        assert!(rename_plan(&n(&["ab.txt", "cd.txt"]), "zz", "y").unwrap().is_empty());
    }

    // P7-M2 (ADR 0054): the skills on a real temp folder (moves and renames through the shell).
    #[test]
    fn housekeeping_on_a_temp_folder() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("xtraos-house-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let mk = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(bytes).unwrap();
            f.set_modified(old).unwrap();
        };
        let a: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut c = a.clone();
        c[299_999] ^= 1; // same size, one byte different
        mk("photo.jpg", &a);
        mk("sub/photo copy.jpg", &a);
        mk("other.jpg", &c);
        mk("notes.txt", b"hello");
        mk("setup.exe", &vec![1u8; 500_000]);
        mk("movie.mp4.crdownload", b"partial");
        let p = |extra: Value| {
            let mut v = json!({"path": dir.display().to_string()});
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };

        let d = files_duplicates(&p(json!({}))).unwrap();
        let groups = d["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "{d}");
        assert_eq!(groups[0].as_array().unwrap().len(), 2, "photo.jpg + its copy, not other.jpg: {d}");
        assert_eq!(d["wasted_bytes"], 300_000);

        let l = files_largest(&p(json!({"limit": 2}))).unwrap();
        assert!(l["text"].as_str().unwrap().lines().nth(1).unwrap().contains("setup.exe"), "{l}");

        let pre = files_organize(&p(json!({}))).unwrap();
        assert_eq!(pre["planned"], 4, "photo, other, notes, setup (not the download): {pre}");
        assert_eq!(pre["moved"], 0);
        assert!(dir.join("photo.jpg").exists(), "a preview changes nothing");
        // The policy class: a preview is read-only; apply is a file change (the user is asked),
        // and the approval text is the plan.
        let pl = |apply: bool| plan(&json!({"name": "files.organize", "args": {"path": dir.display().to_string(), "apply": apply}})).unwrap();
        assert_eq!(pl(false).class, Class::Observe);
        let ask = pl(true);
        assert_eq!(ask.class, Class::FileChange);
        assert!(ask.what.contains("move 4 file(s)") && ask.what.contains("Images"), "{}", ask.what);
        let done = files_organize(&p(json!({"apply": true}))).unwrap();
        assert_eq!(done["moved"], 4, "{done}");
        assert!(dir.join("Images").join("photo.jpg").exists());
        assert!(dir.join("Images").join("other.jpg").exists());
        assert!(dir.join("Documents").join("notes.txt").exists());
        assert!(dir.join("Installers").join("setup.exe").exists());
        assert!(dir.join("movie.mp4.crdownload").exists(), "unfinished download stays");

        let img = dir.join("Images");
        let rp = |extra: Value| {
            let mut v = json!({"path": img.display().to_string()});
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };
        let pre = files_bulk_rename(&rp(json!({"find": "photo", "replace": "trip"}))).unwrap();
        assert_eq!(pre["planned"], 1, "{pre}");
        assert!(img.join("photo.jpg").exists());
        let done = files_bulk_rename(&rp(json!({"find": "photo", "replace": "trip", "apply": true}))).unwrap();
        assert_eq!(done["renamed"], 1, "{done}");
        assert!(img.join("trip.jpg").exists() && !img.join("photo.jpg").exists());
        // A rename onto an existing name is refused before anything changes.
        assert!(files_bulk_rename(&rp(json!({"find": "other", "replace": "trip"}))).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // P7-M3 (ADR 0052): dev.check
    #[test]
    fn dev_check_parses_and_judges() {
        let e = parse_expect(&json!({"expect": [
            {"text": "Saved"}, {"no_text": "Error"}, {"element": "Save", "role": "Button"}, {"title": "report"}
        ]}))
        .unwrap();
        assert_eq!(e.len(), 4);
        assert!(parse_expect(&json!({"expect": []})).is_err());
        assert!(parse_expect(&json!({"expect": [{"text": "a", "title": "b"}]})).is_err(), "one kind each");
        assert!(parse_expect(&json!({"expect": [{"text": ""}]})).is_err());
        use crate::perceive::ElemView;
        let el = |role: &'static str, name: &str, value: Option<&str>, enabled: bool| ElemView {
            role,
            name: name.into(),
            value: value.map(String::from),
            enabled,
        };
        let els = vec![
            el("Button", "Save", None, true),
            el("Edit", "Name", Some("Om"), true),
            el("Text", "Total", Some("42.00"), true),
            el("Button", "Delete", None, false),
        ];
        let text = "File SAVED at 10:00";
        assert!(verdict(&e[0], text, &els, "report.txt", false).0, "case-insensitive text");
        assert!(verdict(&e[1], text, &els, "", false).0);
        assert!(!verdict(&Expect::NoText("saved".into()), text, &els, "", false).0);
        assert!(verdict(&e[2], text, &els, "", false).0);
        let missing = Expect::Element { name: "Cancel".into(), role: Some("Button".into()), value: None, enabled: None };
        let (ok, d) = verdict(&missing, text, &els, "", false);
        assert!(!ok && d.contains("Save"), "failure names what is there: {d}");
        assert!(verdict(&e[3], text, &els, "Q3 Report.pdf", false).0);
        let (ok, d) = verdict(&e[3], text, &els, "Untitled", false);
        assert!(!ok && d.contains("Untitled"));

        // Value, enabled state and visible change.
        let x = parse_expect(&json!({"expect": [
            {"element": "Total", "value": "42"}, {"element": "Delete", "enabled": false},
            {"element": "Total", "value": "41"}, {"changed": true}
        ]}))
        .unwrap();
        assert!(verdict(&x[0], "", &els, "", false).0, "value contains");
        assert!(verdict(&x[1], "", &els, "", false).0, "disabled as expected");
        let (ok, d) = verdict(&x[2], "", &els, "", false);
        assert!(!ok && d.contains("42.00"), "shows the actual value: {d}");
        assert!(!verdict(&x[3], "", &els, "", false).0);
        assert!(verdict(&x[3], "", &els, "", true).0);
        assert!(parse_expect(&json!({"expect": [{"text": "a", "value": "b"}]})).is_err(), "value needs element");
        assert!(parse_expect(&json!({"expect": [{"element": "a", "enabled": "yes"}]})).is_err());
        assert!(parse_expect(&json!({"expect": [{"changed": false}]})).is_err());
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
        // ADR 0050: hidden system folders/files at any drive root
        for p in [
            r"C:\$Recycle.Bin\S-1-5\x",
            r"D:\System Volume Information\y",
            r"C:\pagefile.sys",
            r"C:\Recovery\WindowsRE",
            r"E:\EFI\Microsoft\Boot\bcd",
        ] {
            assert!(protected_reason(Path::new(p), &own).is_some(), "{p}");
        }
        assert!(protected_reason(Path::new(r"D:\Projects\boot-camp\notes.txt"), &own).is_none());
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
    fn whatsapp_args() {
        assert_eq!(wa_phone("+91 98765-43210").as_deref(), Some("919876543210"));
        assert!(wa_phone("12345").is_none());
        assert!(wa_phone("91abc").is_none());
        assert_eq!(
            url_encode("Hi, नमस्ते!"),
            "Hi%2C%20%E0%A4%A8%E0%A4%AE%E0%A4%B8%E0%A5%8D%E0%A4%A4%E0%A5%87%21"
        );
        assert_eq!(
            wa_args(&json!({"phone": "919876543210"}))
                .err()
                .unwrap()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            wa_args(&json!({"phone": "1", "text": "x"}))
                .err()
                .unwrap()
                .code,
            "INVALID_ARGS"
        );
        let p = plan(&json!({"name": "whatsapp.send", "args": {"phone": "+91 98765 43210", "text": "Hello"}})).unwrap();
        assert_eq!(p.class, Class::MessageSend);
        assert!(
            p.what.contains("+919876543210") && p.what.contains("Hello"),
            "{}",
            p.what
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

#[cfg(test)]
mod screen_find_tests {
    use super::screen_find_text;

    #[test]
    fn combined_text_caps_lines_and_notes_budget() {
        let lines: Vec<String> = (1..=7).map(|i| format!("e{i} Button \"Save {i}\"")).collect();
        let hits = vec![
            ("w1 notepad.exe \"a.txt\"".to_string(), vec!["e9 MenuItem \"Save\"".to_string()], 1u64),
            ("w2 chrome.exe \"Docs\"".to_string(), lines, 7u64),
        ];
        let t = screen_find_text("save", &hits, 4, 1, true);
        assert!(t.starts_with("w1 notepad.exe \"a.txt\": 1 match(es)\n  e9 MenuItem \"Save\"\n"), "{t}");
        assert!(t.contains("  e5 Button \"Save 5\"\n  … more: use find in that window"), "{t}");
        assert!(!t.contains("Save 6"), "{t}");
        assert!(t.contains("\"save\": 8 match(es) in 2 window(s); searched 4, 1 skipped"), "{t}");
        assert!(t.contains("time budget reached"), "{t}");
        let none = screen_find_text("zzz", &[], 6, 0, false);
        assert!(none.contains("0 match(es) in 0 window(s); searched 6") && none.contains("ocr"), "{none}");
    }
}

#[cfg(test)]
mod open_app_tests {
    use super::runs_code;
    use std::path::Path;

    #[test]
    fn programs_and_scripts_are_not_opened_as_documents() {
        for p in ["a.exe", "B.BAT", "c.ps1", "d.lnk", "e.msi", "f.vbs", "g.js", "h.url", "i.hta", "j.reg", "k.cmd"] {
            assert!(runs_code(Path::new(p)), "{p}");
        }
        for p in ["report.xlsx", "notes.txt", "photo.png", "deck.pptx", "doc.pdf", "page.html", "README"] {
            assert!(!runs_code(Path::new(p)), "{p}");
        }
    }
}
