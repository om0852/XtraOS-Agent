//! Answering XtraOS dialogs from a paired device (ADR 0076). A device holds its own P-256 key;
//! XtraOS keeps only public keys (`approvers.json`) and answers a dialog only for an answer the
//! device signed. Adding a device needs the human's click on an XtraOS dialog on the PC.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::input::ActError;

pub const METHODS: &[&str] = &["approvals.list", "approvals.answer", "approvers.add", "approvers.list", "approvers.remove"];

/// How far `issued_ms` may be from now.
const FRESH_MS: u64 = 120_000;
static ADDING: AtomicBool = AtomicBool::new(false);

fn err(code: &'static str, m: impl Into<String>) -> ActError {
    ActError { code, message: m.into(), hint: "See docs/adr/0076-approvals-on-a-paired-device.md." }
}

fn bad(m: impl Into<String>) -> ActError {
    err("INVALID_ARGS", m)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The exact text a device signs for one answer (pure).
pub fn signed_text(device: &str, uid: &str, button: usize, issued_ms: u64) -> String {
    format!("xtraos-approval-v1\n{device}\n{uid}\n{button}\n{issued_ms}")
}

/// A raw uncompressed P-256 key (0x04 ‖ X ‖ Y, as WebCrypto exports it) as a CNG public blob (pure).
pub fn raw_to_blob(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() != 65 || raw[0] != 0x04 {
        return None;
    }
    let mut blob = Vec::with_capacity(72);
    blob.extend_from_slice(&0x3153_4345u32.to_le_bytes()); // BCRYPT_ECDSA_PUBLIC_P256_MAGIC ("ECS1")
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(&raw[1..]);
    Some(blob)
}

/// The reverse, for tests (a CNG public blob as the raw key a browser would send).
pub fn blob_to_raw(blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() != 72 {
        return None;
    }
    let mut raw = vec![0x04];
    raw.extend_from_slice(&blob[8..]);
    Some(raw)
}

/// The code both screens show for a key: the first 8 hex characters of its SHA-256 (pure).
pub fn key_code(raw: &[u8]) -> String {
    let h = crate::actionlog::sha256_hex(raw);
    format!("{}-{}", h[..4].to_uppercase(), h[4..8].to_uppercase())
}

// ---- The approvers file ------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Approver {
    pub id: String,
    pub name: String,
    /// Raw key, base64.
    pub key: String,
    pub added: String,
}

fn file() -> PathBuf {
    crate::pairing::data_dir().join("approvers.json")
}

pub fn load(path: &Path) -> Vec<Approver> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return vec![] };
    v.get("approvers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Approver {
                        id: x.get("id")?.as_str()?.to_string(),
                        name: x.get("name")?.as_str()?.to_string(),
                        key: x.get("key")?.as_str()?.to_string(),
                        added: x.get("added").and_then(Value::as_str).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn save(path: &Path, list: &[Approver]) -> std::io::Result<()> {
    let v = json!({
        "version": 1,
        "approvers": list.iter().map(|a| json!({"id": a.id, "name": a.name, "key": a.key, "added": a.added})).collect::<Vec<_>>(),
    });
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&v).unwrap_or_default())
}

/// A checked answer (pure, apart from the signature check). Ok(index of the button).
pub fn check_answer(list: &[Approver], open: &[crate::safety::OpenPrompt], p: &Value, now: u64) -> Result<(usize, String), ActError> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).ok_or_else(|| bad(format!("'{k}' is required")));
    let device = s("device")?;
    let uid = s("uid")?;
    let sig = crate::team::unb64(s("signature")?).ok_or_else(|| bad("'signature' is not base64"))?;
    let button = p.get("button").and_then(Value::as_u64).ok_or_else(|| bad("'button' is required"))? as usize;
    let issued = p.get("issued_ms").and_then(Value::as_u64).ok_or_else(|| bad("'issued_ms' is required"))?;
    let who = list.iter().find(|a| a.id == device).ok_or_else(|| err("BLOCKED_BY_POLICY", "this device is not an approver"))?;
    if issued.abs_diff(now) > FRESH_MS {
        return Err(err("BLOCKED_BY_POLICY", "the answer is too old (or the device clock is wrong)"));
    }
    let dialog = open.iter().find(|d| d.uid == uid).ok_or_else(|| err("NOT_FOUND", "that dialog is no longer open"))?;
    if button >= dialog.buttons.len() {
        return Err(bad("no such button"));
    }
    let raw = crate::team::unb64(&who.key).ok_or_else(|| err("INTERNAL", "stored key is damaged"))?;
    let blob = raw_to_blob(&raw).ok_or_else(|| err("INTERNAL", "stored key is damaged"))?;
    if !crate::team::verify(&blob, signed_text(device, uid, button, issued).as_bytes(), &sig) {
        return Err(err("BLOCKED_BY_POLICY", "the signature does not match this device"));
    }
    Ok((button, who.name.clone()))
}

// ---- Methods -----------------------------------------------------------------------------

pub fn run(method: &str, p: &Value) -> Result<Value, ActError> {
    match method {
        "approvals.list" => {
            let open = crate::safety::open_prompts();
            Ok(json!({
                "dialogs": open.iter().map(|d| json!({
                    "uid": d.uid, "title": d.title, "message": d.message, "buttons": d.buttons,
                    "cancel": d.cancel, "age_ms": d.age_ms,
                })).collect::<Vec<_>>(),
            }))
        }
        "approvals.answer" => {
            let (button, name) = check_answer(&load(&file()), &crate::safety::open_prompts(), p, now_ms())?;
            let uid = p.get("uid").and_then(Value::as_str).unwrap_or("");
            if !crate::safety::answer_uid(uid, button) {
                return Err(err("NOT_FOUND", "that dialog closed just now"));
            }
            crate::safety::activity(&format!("dialog answered from the device \"{name}\""));
            Ok(json!({"answered": true, "button": button, "device": name}))
        }
        "approvers.list" => Ok(json!({
            "approvers": load(&file()).iter().map(|a| {
                let code = crate::team::unb64(&a.key).map(|r| key_code(&r)).unwrap_or_default();
                json!({"id": a.id, "name": a.name, "code": code, "added": a.added})
            }).collect::<Vec<_>>(),
        })),
        "approvers.remove" => {
            let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad("'id' is required"))?;
            let mut list = load(&file());
            let before = list.len();
            list.retain(|a| a.id != id);
            save(&file(), &list).map_err(|e| err("INTERNAL", e.to_string()))?;
            Ok(json!({"removed": before != list.len()}))
        }
        "approvers.add" => add(p),
        m => Err(bad(format!("'{m}' is not an approvals method"))),
    }
}

fn add(p: &Value) -> Result<Value, ActError> {
    let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(bad("'name' is 1-40 characters"));
    }
    let raw = p
        .get("public_key")
        .and_then(Value::as_str)
        .and_then(crate::team::unb64)
        .ok_or_else(|| bad("'public_key' is the raw P-256 key, base64"))?;
    let blob = raw_to_blob(&raw).ok_or_else(|| bad("'public_key' is not a raw uncompressed P-256 key"))?;
    let id = crate::team::key_id(&blob);
    if load(&file()).iter().any(|a| a.id == id) {
        return Ok(json!({"added": true, "id": id, "already": true}));
    }
    if ADDING.swap(true, Ordering::SeqCst) {
        return Err(err("USER_ACTIVE", "another device is waiting to be added; answer that first"));
    }
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            ADDING.store(false, Ordering::SeqCst);
        }
    }
    let _done = Done;
    let code = key_code(&raw);
    let answer = crate::safety::prompt(
        "XtraOS Agent — add a device",
        &format!(
            "Let the device \"{name}\" answer XtraOS dialogs (approvals and questions) from anywhere?\n\nIts code: {code}\nCheck that the device shows the same code.\n\nOnly allow a device you own. You can remove it later."
        ),
        vec!["Allow".into(), "Deny".into()],
        1,
        Duration::from_secs(120),
    )?;
    if answer != Some(0) {
        return Err(err("BLOCKED_BY_POLICY", "the device was not allowed on the PC"));
    }
    let mut list = load(&file());
    list.push(Approver {
        id: id.clone(),
        name: name.to_string(),
        key: crate::team::b64(&raw),
        added: format!("{}", now_ms()),
    });
    save(&file(), &list).map_err(|e| err("INTERNAL", e.to_string()))?;
    Ok(json!({"added": true, "id": id, "code": code}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::OpenPrompt;

    fn device() -> (Approver, Vec<u8>) {
        let (public, private) = crate::team::generate().unwrap();
        let raw = blob_to_raw(&public).unwrap();
        assert_eq!(raw_to_blob(&raw).unwrap(), public, "raw and blob convert both ways");
        let a = Approver { id: crate::team::key_id(&public), name: "Phone".into(), key: crate::team::b64(&raw), added: String::new() };
        (a, private)
    }

    fn dialog(uid: &str) -> OpenPrompt {
        OpenPrompt { uid: uid.into(), title: "t".into(), message: "m".into(), buttons: vec!["Allow once".into(), "Deny".into()], cancel: 1, age_ms: 10 }
    }

    fn answer(a: &Approver, private: &[u8], uid: &str, button: usize, issued: u64) -> Value {
        let sig = crate::team::sign(private, signed_text(&a.id, uid, button, issued).as_bytes()).unwrap();
        json!({"device": a.id, "uid": uid, "button": button, "issued_ms": issued, "signature": crate::team::b64(&sig)})
    }

    #[test]
    fn a_signed_answer_from_an_approver_is_accepted() {
        let (a, private) = device();
        let open = [dialog("u1")];
        let now = 1_800_000_000_000;
        assert_eq!(check_answer(&[a.clone()], &open, &answer(&a, &private, "u1", 0, now - 5_000), now).unwrap(), (0, "Phone".into()));
        assert_eq!(check_answer(&[a.clone()], &open, &answer(&a, &private, "u1", 1, now), now).unwrap().0, 1);
    }

    #[test]
    fn everything_else_is_refused() {
        let (a, private) = device();
        let (other, other_private) = device();
        let open = [dialog("u1"), dialog("u2")];
        let now = 1_800_000_000_000;
        let code = |v: Value| check_answer(&[a.clone()], &open, &v, now).unwrap_err().code;
        assert_eq!(code(answer(&other, &other_private, "u1", 0, now)), "BLOCKED_BY_POLICY", "not an approver");
        assert_eq!(code(answer(&a, &private, "u1", 0, now - 121_000)), "BLOCKED_BY_POLICY", "too old");
        assert_eq!(code(answer(&a, &private, "u1", 0, now + 121_000)), "BLOCKED_BY_POLICY", "from the future");
        assert_eq!(code(answer(&a, &private, "gone", 0, now)), "NOT_FOUND", "dialog not open");
        assert_eq!(code(answer(&a, &private, "u1", 2, now)), "INVALID_ARGS", "no such button");
        // A signature for one dialog, button or time does not work for another.
        let mut v = answer(&a, &private, "u1", 1, now);
        v["uid"] = json!("u2");
        assert_eq!(code(v), "BLOCKED_BY_POLICY", "replayed on another dialog");
        let mut v = answer(&a, &private, "u1", 1, now);
        v["button"] = json!(0);
        assert_eq!(code(v), "BLOCKED_BY_POLICY", "Deny turned into Allow");
        let mut v = answer(&a, &private, "u1", 0, now);
        v["issued_ms"] = json!(now - 1);
        assert_eq!(code(v), "BLOCKED_BY_POLICY", "time changed");
        // Signed by another key but claiming the approver's id.
        let mut v = answer(&other, &other_private, "u1", 0, now);
        v["device"] = json!(a.id.clone());
        assert_eq!(code(v), "BLOCKED_BY_POLICY", "someone else's signature");
        assert_eq!(code(json!({"device": a.id, "uid": "u1", "button": 0, "issued_ms": now})), "INVALID_ARGS", "no signature");
    }

    #[test]
    fn keys_codes_and_the_file() {
        assert!(raw_to_blob(&[4u8; 64]).is_none(), "too short");
        assert!(raw_to_blob(&[[2u8].as_slice(), &[0u8; 64]].concat()).is_none(), "compressed form");
        assert_eq!(signed_text("d", "u", 1, 5), "xtraos-approval-v1\nd\nu\n1\n5");
        let code = key_code(&[4u8; 65]);
        assert!(code.len() == 9 && code.as_bytes()[4] == b'-');
        let dir = std::env::temp_dir().join(format!("xtraos-approvers-{}", std::process::id()));
        let f = dir.join("approvers.json");
        let (a, _) = device();
        assert!(load(&f).is_empty());
        save(&f, &[a.clone()]).unwrap();
        assert_eq!(load(&f), vec![a]);
        std::fs::write(&f, "not json").unwrap();
        assert!(load(&f).is_empty(), "a damaged file means no approvers, not a crash");
        let _ = std::fs::remove_dir_all(dir);
    }
}
