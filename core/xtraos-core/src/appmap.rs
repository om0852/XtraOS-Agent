//! App map (P2-M3, ADR 0026): where the user's apps and services are, so the agent never
//! guesses. `scan` lists installed apps (classic + Store) from the Windows Apps folder with
//! their launch ids; `set`/`get`/`list`/`remove` keep the agent's notes ("whatsapp = the Store
//! app", "gmail = Chrome profile 2 at mail.google.com") in a per-user file encrypted with DPAPI.
//!
//! Docs: FOLDERID_AppsFolder https://learn.microsoft.com/windows/win32/shell/knownfolderid
//!       CryptProtectData  https://learn.microsoft.com/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Map, Value};
use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
    KF_FLAG_DEFAULT, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "Check the tool schema in docs/04-mcp-tool-contract.md.",
    }
}

fn internal(m: impl Into<String>) -> ActError {
    ActError {
        code: "INTERNAL",
        message: m.into(),
        hint: "",
    }
}

/// Runs `f` on a fresh single-threaded-apartment thread (the shell expects STA).
pub fn on_sta<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(move || {
        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let r = f();
        if init.is_ok() {
            unsafe { CoUninitialize() };
        }
        r
    })
    .join()
    .expect("STA worker panicked")
}

// ---------------------------------------------------------------------------------------------
// Storage (DPAPI, current user)

static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Tests use their own file.
pub fn set_path(p: PathBuf) {
    *PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
}

fn path() -> PathBuf {
    PATH.lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| crate::pairing::data_dir().join("appmap.bin"))
}

fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: b.len() as u32,
        pbData: b.as_ptr() as *mut u8,
    }
}

fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
    v
}

pub fn protect(plain: &[u8]) -> Result<Vec<u8>, ActError> {
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &blob(plain),
            w!("XtraOS app map"),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    }
    .map_err(|e| internal(format!("DPAPI protect: {e}")))?;
    Ok(take(out))
}

pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, ActError> {
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &blob(sealed),
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    }
    .map_err(|e| internal(format!("DPAPI unprotect: {e}")))?;
    Ok(take(out))
}

fn load() -> Map<String, Value> {
    std::fs::read(path())
        .ok()
        .and_then(|b| unprotect(&b).ok())
        .and_then(|p| serde_json::from_slice::<Value>(&p).ok())
        .and_then(|v| v.get("services").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

fn save(services: &Map<String, Value>) -> Result<(), ActError> {
    let plain = serde_json::to_vec(&json!({"version": 1, "services": services}))
        .map_err(|e| internal(e.to_string()))?;
    let sealed = protect(&plain)?;
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| internal(e.to_string()))?;
    }
    std::fs::write(&p, sealed).map_err(|e| internal(format!("writing the app map: {e}")))
}

/// What `launch_app` should start for a mapped service: (app, args).
pub fn resolve_launch(service: &str) -> Option<(String, Vec<String>)> {
    let e = load().get(&service.to_lowercase())?.clone();
    let loc = &e["location"];
    match e["surface"].as_str()? {
        "desktop" => Some((loc.as_str()?.to_string(), Vec::new())),
        "browser" => {
            let browser = loc["browser"].as_str().unwrap_or("chrome").to_string();
            let mut args = vec!["--new-window".to_string()];
            if let Some(p) = loc["profile"].as_str() {
                if browser.contains("firefox") {
                    args = vec!["-P".into(), p.into(), "-new-window".into()];
                } else {
                    args.push(format!("--profile-directory={p}"));
                }
            } else if browser.contains("firefox") {
                args = vec!["-new-window".into()];
            }
            args.push(loc["url"].as_str()?.to_string());
            Some((browser, args))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Installed apps (Windows Apps folder)

unsafe fn display(item: &IShellItem, how: SIGDN) -> String {
    match item.GetDisplayName(how) {
        Ok(p) => {
            let s = PWSTR(p.0).to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0.cast()));
            s
        }
        Err(_) => String::new(),
    }
}

/// (name, launch id) of every app in the Apps folder.
pub fn installed_apps() -> Result<Vec<(String, String)>, ActError> {
    on_sta(|| unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
            .map_err(|e| internal(format!("Apps folder: {e}")))?;
        let items: IEnumShellItems = folder
            .BindToHandler(None::<&IBindCtx>, &BHID_EnumItems)
            .map_err(|e| internal(format!("Apps folder items: {e}")))?;
        let mut out = Vec::new();
        loop {
            let mut one: [Option<IShellItem>; 1] = [None];
            let mut n = 0u32;
            if items.Next(&mut one, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            if let Some(it) = one[0].take() {
                let name = display(&it, SIGDN_NORMALDISPLAY);
                let id = display(&it, SIGDN_PARENTRELATIVEPARSING);
                if !name.is_empty() && !id.is_empty() {
                    out.push((name, format!("shell:AppsFolder\\{id}")));
                }
            }
        }
        out.sort_by_key(|(n, _)| n.to_lowercase());
        Ok(out)
    })
}

// ---------------------------------------------------------------------------------------------
// Tool

pub fn run(p: &Value) -> Result<Value, ActError> {
    let op = p
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("'op' is required: scan, list, get, set, remove"))?;
    let service = || {
        p.get("service")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty() && s.len() <= 60)
            .ok_or_else(|| bad("'service' (1-60 chars) is required"))
    };
    match op {
        "scan" => {
            let q = p
                .get("query")
                .and_then(Value::as_str)
                .map(|s| s.to_lowercase());
            let apps = installed_apps()?;
            let hits: Vec<String> = apps
                .iter()
                .filter(|(n, id)| {
                    q.as_ref().is_none_or(|q| {
                        n.to_lowercase().contains(q) || id.to_lowercase().contains(q)
                    })
                })
                .take(50)
                .map(|(n, id)| format!("{n} → {id}"))
                .collect();
            let total = apps.len();
            Ok(json!({
                "text": format!("{}\n-- {} shown of {total} installed apps", hits.join("\n"), hits.len()),
                "apps": hits.len(),
            }))
        }
        "list" => {
            let m = load();
            let lines: Vec<String> = m
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{k}: {} {}",
                        v["surface"].as_str().unwrap_or("?"),
                        v["location"]
                    )
                })
                .collect();
            Ok(
                json!({"text": if lines.is_empty() { "(app map is empty)".to_string() } else { lines.join("\n") }, "entries": m.len()}),
            )
        }
        "get" => {
            let s = service()?;
            match load().get(&s) {
                Some(e) => {
                    Ok(json!({"service": s, "surface": e["surface"], "location": e["location"]}))
                }
                None => Err(ActError {
                    code: "ELEMENT_NOT_FOUND",
                    message: format!("'{s}' is not in the app map"),
                    hint: "Use app_map scan to find the app, then app_map set.",
                }),
            }
        }
        "set" => {
            let s = service()?;
            let surface = p.get("surface").and_then(Value::as_str).unwrap_or("");
            let loc = p.get("location").cloned().unwrap_or(Value::Null);
            match surface {
                "desktop" => {
                    loc.as_str()
                        .filter(|l| !l.trim().is_empty())
                        .ok_or_else(|| {
                            bad("desktop 'location' is a launch id or path (from app_map scan)")
                        })?;
                }
                "browser" => {
                    let url = loc["url"].as_str().unwrap_or("");
                    if !(url.starts_with("https://") || url.starts_with("http://")) {
                        return Err(bad(
                            "browser 'location' is {browser, url (http/https), profile?}",
                        ));
                    }
                }
                _ => return Err(bad("'surface' must be desktop or browser")),
            }
            let mut m = load();
            m.insert(s.clone(), json!({"surface": surface, "location": loc}));
            save(&m)?;
            Ok(json!({"service": s, "saved": true, "entries": m.len()}))
        }
        "remove" => {
            let s = service()?;
            let mut m = load();
            let had = m.remove(&s).is_some();
            save(&m)?;
            Ok(json!({"service": s, "removed": had}))
        }
        o => Err(bad(format!(
            "unknown op '{o}' (scan, list, get, set, remove)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip_hides_the_text() {
        let sealed = protect(b"whatsapp desktop").unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"whatsapp"));
        assert_eq!(unprotect(&sealed).unwrap(), b"whatsapp desktop");
        assert!(unprotect(b"not a dpapi blob").is_err());
    }

    #[test]
    fn set_get_remove_with_a_private_file() {
        let dir = std::env::temp_dir().join(format!("xtraos-am-{}", std::process::id()));
        set_path(dir.join("appmap.bin"));
        let r = run(&json!({"op": "set", "service": "Mail", "surface": "browser",
            "location": {"browser": "chrome", "url": "https://mail.example.com", "profile": "Profile 2"}}))
        .unwrap();
        assert_eq!(r["saved"], true);
        let g = run(&json!({"op": "get", "service": "mail"})).unwrap();
        assert_eq!(g["location"]["url"], "https://mail.example.com");
        let (app, args) = resolve_launch("mail").unwrap();
        assert_eq!(app, "chrome");
        assert_eq!(
            args,
            vec![
                "--new-window",
                "--profile-directory=Profile 2",
                "https://mail.example.com"
            ]
        );
        assert_eq!(run(&json!({"op": "set", "service": "x", "surface": "browser", "location": {"url": "file:///c"}})).unwrap_err().code, "INVALID_ARGS");
        assert_eq!(
            run(&json!({"op": "remove", "service": "mail"})).unwrap()["removed"],
            true
        );
        assert_eq!(
            run(&json!({"op": "get", "service": "mail"}))
                .unwrap_err()
                .code,
            "ELEMENT_NOT_FOUND"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
