//! Window layouts (ADR 0065): save how the windows are arranged and restore it in one call.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::input::ActError;

const MAX_LAYOUTS: usize = 30;
const MAX_WINDOWS: usize = 40;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "See list_skills for layout.* arguments." }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub process: String,
    pub title: String,
    /// Visible frame; None for a minimized window (Windows reports no real position).
    pub rect: Option<[i32; 4]>,
    pub state: String,
}

impl Saved {
    fn to_json(&self) -> Value {
        json!({"process": self.process, "title": self.title, "rect": self.rect, "state": self.state})
    }

    fn from_json(v: &Value) -> Option<Saved> {
        let rect = v["rect"].as_array().and_then(|a| {
            let n: Vec<i32> = a.iter().filter_map(|x| x.as_i64().map(|n| n as i32)).collect();
            (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
        });
        Some(Saved {
            process: v["process"].as_str()?.to_string(),
            title: v["title"].as_str().unwrap_or("").to_string(),
            rect,
            state: v["state"].as_str().unwrap_or("normal").to_string(),
        })
    }
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 40 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
}

fn dir() -> PathBuf {
    crate::pairing::data_dir().join("layouts")
}

fn file_of(name: &str) -> PathBuf {
    dir().join(format!("{}.json", name.replace(' ', "_")))
}

fn name_arg(a: &Value) -> Result<String, ActError> {
    let n = a.get("name").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    if !valid_name(&n) {
        return Err(bad("'name' must be 1..40 characters: letters, digits, space - _ ."));
    }
    Ok(n)
}

/// " - " separated parts of a title, lower case (pure).
fn parts(t: &str) -> Vec<String> {
    t.to_lowercase()
        .split(" - ")
        .flat_map(|p| p.split(" — "))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Which open window each saved window goes to (pure). `open`: (process, title).
/// Same process; the same title first, else the most title parts in common, else the first
/// unused one. No open window is used twice.
pub fn match_windows(saved: &[Saved], open: &[(String, String)]) -> Vec<Option<usize>> {
    let mut used = vec![false; open.len()];
    let mut out = vec![None; saved.len()];
    // Exact titles first, so a fuzzy match cannot take another entry's exact window.
    for (i, s) in saved.iter().enumerate() {
        if let Some(j) = (0..open.len()).find(|&j| {
            !used[j] && open[j].0.eq_ignore_ascii_case(&s.process) && open[j].1 == s.title
        }) {
            used[j] = true;
            out[i] = Some(j);
        }
    }
    for (i, s) in saved.iter().enumerate() {
        if out[i].is_some() {
            continue;
        }
        let want = parts(&s.title);
        let best = (0..open.len())
            .filter(|&j| !used[j] && open[j].0.eq_ignore_ascii_case(&s.process))
            .max_by_key(|&j| {
                let have = parts(&open[j].1);
                // more parts in common wins; on a tie the earlier (front-most) window
                (want.iter().filter(|p| have.contains(p)).count(), std::cmp::Reverse(j))
            });
        if let Some(j) = best {
            used[j] = true;
            out[i] = Some(j);
        }
    }
    out
}

fn current() -> Vec<(crate::input::HWND, String, String)> {
    crate::winmgr::listable_windows()
        .into_iter()
        .filter(|&h| !crate::safety::is_own_window(h) && !crate::winmgr::is_elevated(h))
        .map(|h| (h, crate::winmgr::process_name(crate::winmgr::pid_of(h)), crate::winmgr::title(h)))
        .collect()
}

fn load(name: &str) -> Result<Vec<Saved>, ActError> {
    let text = std::fs::read_to_string(file_of(name)).map_err(|_| bad(format!("no layout named '{name}' (see layout.list)")))?;
    let v: Value = serde_json::from_str(&text).map_err(|_| bad(format!("layout '{name}' is damaged")))?;
    Ok(v["windows"].as_array().into_iter().flatten().filter_map(Saved::from_json).collect())
}

pub fn run(op: &str, a: &Value) -> Result<Value, ActError> {
    match op {
        "layout.save" => {
            let name = name_arg(a)?;
            let filter = a.get("filter").and_then(Value::as_str).map(str::to_lowercase);
            let existing = std::fs::read_dir(dir()).map(|d| d.count()).unwrap_or(0);
            if !file_of(&name).exists() && existing >= MAX_LAYOUTS {
                return Err(bad(format!("at most {MAX_LAYOUTS} layouts; delete one first")));
            }
            let mut wins: Vec<Saved> = current()
                .into_iter()
                .filter(|(_, p, t)| filter.as_ref().is_none_or(|f| p.to_lowercase().contains(f) || t.to_lowercase().contains(f)))
                .map(|(h, process, title)| {
                    let state = crate::winmgr::window_state(h).to_string();
                    let r = crate::winmgr::frame(h);
                    let rect = (state != "minimized").then_some([r.left, r.top, r.right - r.left, r.bottom - r.top]);
                    Saved { process, title, rect, state }
                })
                .collect();
            // Back to front, so restoring in this order keeps the same stacking.
            wins.reverse();
            wins.truncate(MAX_WINDOWS);
            if wins.is_empty() {
                return Err(bad("no windows to save (check 'filter')"));
            }
            std::fs::create_dir_all(dir()).map_err(|e| bad(format!("layouts: {e}")))?;
            let doc = json!({"name": name, "saved": crate::routines::show_time(crate::routines::local_now_ms()), "windows": wins.iter().map(Saved::to_json).collect::<Vec<_>>()});
            std::fs::write(file_of(&name), serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| bad(format!("layouts: {e}")))?;
            let shown: Vec<String> = wins.iter().map(|w| format!("{} \"{}\" {}", w.process, crate::perceive::clip(&w.title, 40), w.state)).collect();
            Ok(json!({"text": format!("ok layout '{name}' saved: {} window(s)\n{}", wins.len(), shown.join("\n")), "windows": wins.len()}))
        }
        "layout.restore" => {
            let name = name_arg(a)?;
            let saved = load(&name)?;
            let open = current();
            let pairs: Vec<(String, String)> = open.iter().map(|(_, p, t)| (p.clone(), t.clone())).collect();
            let m = match_windows(&saved, &pairs);
            let mut done = Vec::new();
            let mut missing = Vec::new();
            let mut failed = Vec::new();
            for (s, j) in saved.iter().zip(m) {
                crate::safety::check()?;
                let Some(j) = j else {
                    missing.push(format!("{} \"{}\"", s.process, crate::perceive::clip(&s.title, 40)));
                    continue;
                };
                let r = crate::winmgr::window_ref(open[j].0);
                let res = (|| -> Result<(), ActError> {
                    if let Some([x, y, w, h]) = s.rect {
                        // A maximized window: put it on its monitor first, then maximize.
                        let (x, y, w, h) = if s.state == "maximized" { (x + 40, y + 40, (w - 80).max(200), (h - 80).max(200)) } else { (x, y, w, h) };
                        crate::winmgr::run("window_bounds", &json!({"window": r, "x": x, "y": y, "w": w.max(50), "h": h.max(50)}))?;
                    }
                    match s.state.as_str() {
                        "maximized" => crate::winmgr::run("window_bounds", &json!({"window": r, "state": "maximize"})).map(|_| ()),
                        "minimized" => crate::winmgr::run("window_bounds", &json!({"window": r, "state": "minimize"})).map(|_| ()),
                        _ => Ok(()),
                    }
                })();
                match res {
                    Ok(()) => done.push(format!("{r} {}", s.process)),
                    Err(e) if e.code == "STOPPED" => return Err(e),
                    Err(e) => failed.push(format!("{r} {}: {}", s.process, e.message)),
                }
            }
            let mut text = format!("ok layout '{name}': {} window(s) placed", done.len());
            if !missing.is_empty() {
                text.push_str(&format!("\nnot open (open them with launch_app, then restore again): {}", missing.join("; ")));
            }
            if !failed.is_empty() {
                text.push_str(&format!("\ncould not place: {}", failed.join("; ")));
            }
            Ok(json!({"text": text, "placed": done.len(), "missing": missing, "failed": failed}))
        }
        "layout.list" => {
            let mut names: Vec<(String, usize, String)> = std::fs::read_dir(dir())
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
                    Some((v["name"].as_str()?.to_string(), v["windows"].as_array().map_or(0, Vec::len), v["saved"].as_str().unwrap_or("").to_string()))
                })
                .collect();
            names.sort();
            let text = if names.is_empty() {
                "no layouts".to_string()
            } else {
                names.iter().map(|(n, c, t)| format!("{n}: {c} window(s), saved {t}")).collect::<Vec<_>>().join("\n")
            };
            Ok(json!({"text": text, "layouts": names.iter().map(|(n, c, _)| json!({"name": n, "windows": c})).collect::<Vec<_>>()}))
        }
        "layout.delete" => {
            let name = name_arg(a)?;
            std::fs::remove_file(file_of(&name)).map_err(|_| bad(format!("no layout named '{name}'")))?;
            Ok(json!({"text": format!("ok layout '{name}' deleted")}))
        }
        o => Err(bad(format!("unknown layout op '{o}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(p: &str, t: &str) -> Saved {
        Saved { process: p.into(), title: t.into(), rect: Some([0, 0, 100, 100]), state: "normal".into() }
    }

    fn o(p: &str, t: &str) -> (String, String) {
        (p.into(), t.into())
    }

    #[test]
    fn names() {
        assert!(valid_name("coding"));
        assert!(valid_name("morning setup"));
        assert!(!valid_name(""));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(&"x".repeat(41)));
    }

    #[test]
    fn windows_match_by_process_then_title() {
        let saved = vec![
            s("Code.exe", "main.rs - xtraos - Visual Studio Code"),
            s("chrome.exe", "GitHub - Google Chrome"),
            s("chrome.exe", "Docs - Google Chrome"),
            s("WindowsTerminal.exe", "PowerShell"),
        ];
        let open = vec![
            o("chrome.exe", "Docs - Google Chrome"),
            o("code.exe", "lib.rs - xtraos - Visual Studio Code"),
            o("chrome.exe", "Inbox - Google Chrome"),
            o("notepad.exe", "x"),
        ];
        let m = match_windows(&saved, &open);
        assert_eq!(m[0], Some(1), "same process (any case), most title parts in common");
        assert_eq!(m[2], Some(0), "the exact title wins");
        assert_eq!(m[1], Some(2), "the other chrome window, not the exact one taken by entry 2");
        assert_eq!(m[3], None, "not open");
    }

    #[test]
    fn no_window_is_used_twice() {
        let saved = vec![s("notepad.exe", "a"), s("notepad.exe", "b"), s("notepad.exe", "c")];
        let open = vec![o("notepad.exe", "z"), o("notepad.exe", "y")];
        let m = match_windows(&saved, &open);
        assert_eq!(m, vec![Some(0), Some(1), None]);
    }

    #[test]
    fn saved_round_trip() {
        let a = Saved { process: "x.exe".into(), title: "T".into(), rect: None, state: "minimized".into() };
        assert_eq!(Saved::from_json(&a.to_json()), Some(a));
        let b = s("y.exe", "U");
        assert_eq!(Saved::from_json(&b.to_json()), Some(b));
    }
}
