//! Team policies (ADR 0073): a policy from the organisation, signed with ECDSA P-256 (Windows
//! CNG), kept in %ProgramData%\XtraOS, enforced as a floor: it can only make rules stricter.
//! A team policy file that is present but does not verify blocks every acting call.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::policy::{Class, Decision, Profile};

// ---------------------------------------------------------------------------------------------
// ECDSA P-256 through Windows CNG (bcrypt.dll), the same style as winsec.rs

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptOpenAlgorithmProvider(alg: *mut *mut c_void, id: *const u16, imp: *const u16, flags: u32) -> i32;
    fn BCryptCloseAlgorithmProvider(alg: *mut c_void, flags: u32) -> i32;
    fn BCryptGenerateKeyPair(alg: *mut c_void, key: *mut *mut c_void, bits: u32, flags: u32) -> i32;
    fn BCryptFinalizeKeyPair(key: *mut c_void, flags: u32) -> i32;
    fn BCryptExportKey(key: *mut c_void, exp: *mut c_void, blob: *const u16, out: *mut u8, cb: u32, res: *mut u32, flags: u32) -> i32;
    fn BCryptImportKeyPair(alg: *mut c_void, imp: *mut c_void, blob: *const u16, key: *mut *mut c_void, input: *const u8, cb: u32, flags: u32) -> i32;
    fn BCryptSignHash(key: *mut c_void, pad: *mut c_void, hash: *const u8, cbh: u32, out: *mut u8, cb: u32, res: *mut u32, flags: u32) -> i32;
    fn BCryptVerifySignature(key: *mut c_void, pad: *mut c_void, hash: *const u8, cbh: u32, sig: *const u8, cbs: u32, flags: u32) -> i32;
    fn BCryptDestroyKey(key: *mut c_void) -> i32;
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Alg(*mut c_void);
impl Alg {
    fn ecdsa() -> Result<Alg, String> {
        let mut a = std::ptr::null_mut();
        let id = w("ECDSA_P256");
        let st = unsafe { BCryptOpenAlgorithmProvider(&mut a, id.as_ptr(), std::ptr::null(), 0) };
        if st != 0 {
            return Err(format!("ECDSA provider: 0x{st:08x}"));
        }
        Ok(Alg(a))
    }
}
impl Drop for Alg {
    fn drop(&mut self) {
        unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
    }
}
struct Key(*mut c_void);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe { BCryptDestroyKey(self.0) };
    }
}

fn export(k: &Key, blob: &str) -> Result<Vec<u8>, String> {
    let bt = w(blob);
    let mut n = 0u32;
    let st = unsafe { BCryptExportKey(k.0, std::ptr::null_mut(), bt.as_ptr(), std::ptr::null_mut(), 0, &mut n, 0) };
    if st != 0 {
        return Err(format!("export size: 0x{st:08x}"));
    }
    let mut out = vec![0u8; n as usize];
    let st = unsafe { BCryptExportKey(k.0, std::ptr::null_mut(), bt.as_ptr(), out.as_mut_ptr(), n, &mut n, 0) };
    if st != 0 {
        return Err(format!("export: 0x{st:08x}"));
    }
    out.truncate(n as usize);
    Ok(out)
}

fn import(alg: &Alg, blob: &str, bytes: &[u8]) -> Result<Key, String> {
    let bt = w(blob);
    let mut k = std::ptr::null_mut();
    let st = unsafe { BCryptImportKeyPair(alg.0, std::ptr::null_mut(), bt.as_ptr(), &mut k, bytes.as_ptr(), bytes.len() as u32, 0) };
    if st != 0 {
        return Err(format!("not a valid key (0x{st:08x})"));
    }
    Ok(Key(k))
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let hex = crate::actionlog::sha256_hex(data);
    (0..32).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

/// A new key pair: (public blob, private blob).
pub fn generate() -> Result<(Vec<u8>, Vec<u8>), String> {
    let alg = Alg::ecdsa()?;
    let mut k = std::ptr::null_mut();
    let st = unsafe { BCryptGenerateKeyPair(alg.0, &mut k, 256, 0) };
    if st != 0 {
        return Err(format!("generate: 0x{st:08x}"));
    }
    let key = Key(k);
    let st = unsafe { BCryptFinalizeKeyPair(key.0, 0) };
    if st != 0 {
        return Err(format!("finalize: 0x{st:08x}"));
    }
    Ok((export(&key, "ECCPUBLICBLOB")?, export(&key, "ECCPRIVATEBLOB")?))
}

pub fn sign(private_blob: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let alg = Alg::ecdsa()?;
    let key = import(&alg, "ECCPRIVATEBLOB", private_blob)?;
    let h = sha256(data);
    let mut n = 0u32;
    let mut sig = vec![0u8; 64];
    let st = unsafe { BCryptSignHash(key.0, std::ptr::null_mut(), h.as_ptr(), h.len() as u32, sig.as_mut_ptr(), 64, &mut n, 0) };
    if st != 0 {
        return Err(format!("sign: 0x{st:08x}"));
    }
    sig.truncate(n as usize);
    Ok(sig)
}

pub fn verify(public_blob: &[u8], data: &[u8], sig: &[u8]) -> bool {
    let Ok(alg) = Alg::ecdsa() else { return false };
    let Ok(key) = import(&alg, "ECCPUBLICBLOB", public_blob) else { return false };
    let h = sha256(data);
    unsafe { BCryptVerifySignature(key.0, std::ptr::null_mut(), h.as_ptr(), h.len() as u32, sig.as_ptr(), sig.len() as u32, 0) == 0 }
}

pub fn key_id(public_blob: &[u8]) -> String {
    crate::actionlog::sha256_hex(public_blob)[..16].to_string()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64(data: &[u8]) -> String {
    crate::capture::base64(data)
}

pub fn unb64(s: &str) -> Option<Vec<u8>> {
    let s: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if s.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| B64.iter().position(|&x| x == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for ch in s.chunks(4) {
        let pad = ch.iter().rev().take_while(|&&c| c == b'=').count();
        let mut n = 0u32;
        for (i, &c) in ch.iter().enumerate() {
            let v = if c == b'=' && i >= 4 - pad { 0 } else { val(c)? };
            n = (n << 6) | v;
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

/// The signed text of a policy: compact JSON with keys in sorted order (serde_json's default).
pub fn canonical(policy: &Value) -> String {
    policy.to_string()
}

// ---------------------------------------------------------------------------------------------
// The team policy (pure parts unit-tested)

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Team {
    pub classes: HashMap<Class, Decision>,
    pub apps: HashMap<String, Decision>,
    pub clients: HashMap<String, Profile>,
    pub rate_per_minute: Option<u32>,
    pub max_actions_per_task: Option<u32>,
    pub key_id: String,
    pub hash: String,
}

/// Only what the team file names (no defaults: unnamed entries stay the user's).
pub fn parse_policy(v: &Value) -> Result<Team, String> {
    let mut t = Team::default();
    if let Some(cs) = v.get("classes").and_then(Value::as_object) {
        for (k, d) in cs {
            let c = Class::parse(k).ok_or_else(|| format!("classes.{k}: unknown class"))?;
            let d = d.as_str().and_then(Decision::parse).ok_or_else(|| format!("classes.{k}: allow, confirm or deny"))?;
            t.classes.insert(c, d);
        }
    }
    if let Some(apps) = v.get("apps").and_then(Value::as_object) {
        for (k, d) in apps {
            let d = d.as_str().and_then(Decision::parse).ok_or_else(|| format!("apps.{k}: allow, confirm or deny"))?;
            t.apps.insert(k.to_lowercase(), d);
        }
    }
    if let Some(cs) = v.get("clients").and_then(Value::as_object) {
        for (k, p) in cs {
            let p = p.as_str().and_then(Profile::parse).ok_or_else(|| format!("clients.{k}: full, background_only, read_only or blocked"))?;
            t.clients.insert(k.to_lowercase(), p);
        }
    }
    t.rate_per_minute = v.get("rate_per_minute").and_then(Value::as_u64).map(|n| n.clamp(1, 10_000) as u32);
    t.max_actions_per_task = v.get("max_actions_per_task").and_then(Value::as_u64).map(|n| n.clamp(1, 1_000_000) as u32);
    Ok(t)
}

/// How strict a client profile is (higher = stricter).
pub fn rank(p: Profile) -> u8 {
    match p {
        Profile::Full => 0,
        Profile::BackgroundOnly => 1,
        Profile::ReadOnly => 2,
        Profile::Blocked => 3,
    }
}

impl Team {
    /// The team's own decision for an action: only rules it names; Allow otherwise (so the
    /// stricter-of with the user's decision leaves the user's decision alone).
    pub fn decide(&self, f: &crate::policy::Facts) -> (Decision, String) {
        let mut best = (Decision::Allow, String::new());
        for (p, _) in f.windows.iter().chain(f.drop_process.iter()) {
            if let Some(&d) = self.apps.get(p) {
                if d > best.0 {
                    best = (d, format!("organisation rule for {p}"));
                }
            }
        }
        for (c, why) in crate::policy::classify(f) {
            if let Some(&d) = self.classes.get(&c) {
                if d > best.0 {
                    best = (d, format!("organisation rule: {} ({why})", c.name()));
                }
            }
        }
        best
    }

    pub fn client(&self, name: &str) -> Option<Profile> {
        self.clients.get(&name.to_lowercase()).or_else(|| self.clients.get("*")).copied()
    }
}

/// The stricter of two decisions, keeping the reason of the one that wins (pure).
pub fn stricter(user: (Decision, String), team: (Decision, String)) -> (Decision, String) {
    if team.0 > user.0 {
        team
    } else {
        user
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    None,
    Active(Team),
    Broken(String),
}

/// Reads and verifies `<dir>/team-policy.json` against `<dir>/team-keys/<key_id>.pub`.
pub fn load(dir: &Path) -> State {
    let file = dir.join("team-policy.json");
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return State::None,
        Err(e) => return State::Broken(format!("cannot read {}: {e}", file.display())),
    };
    let env: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return State::Broken(format!("team-policy.json is not JSON: {e}")),
    };
    let (Some(policy), Some(kid), Some(sig)) = (env.get("policy"), env["key_id"].as_str(), env["signature"].as_str()) else {
        return State::Broken("team-policy.json needs policy, key_id and signature".into());
    };
    if kid.len() != 16 || !kid.bytes().all(|c| c.is_ascii_hexdigit()) {
        return State::Broken("key_id must be 16 hex characters".into());
    }
    let Some(pubkey) = std::fs::read_to_string(dir.join("team-keys").join(format!("{kid}.pub"))).ok().and_then(|s| unb64(s.trim())) else {
        return State::Broken(format!("no trusted key {kid} in team-keys"));
    };
    if key_id(&pubkey) != kid {
        return State::Broken(format!("key file {kid}.pub does not match its name"));
    }
    let Some(sig) = unb64(sig) else { return State::Broken("signature is not base64".into()) };
    let signed = canonical(policy);
    if !verify(&pubkey, signed.as_bytes(), &sig) {
        return State::Broken("the signature does not match the policy (changed or not signed by the organisation's key)".into());
    }
    match parse_policy(policy) {
        Ok(mut t) => {
            t.key_id = kid.to_string();
            t.hash = crate::actionlog::sha256_hex(signed.as_bytes())[..16].to_string();
            State::Active(t)
        }
        Err(e) => State::Broken(format!("team policy: {e}")),
    }
}

pub fn team_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XTRAOS_TEAM_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")).join("XtraOS")
}

type Stamp = (Option<SystemTime>, Option<SystemTime>, PathBuf);
static CACHE: Mutex<Option<(Stamp, State)>> = Mutex::new(None);

fn stamp(dir: &Path) -> Stamp {
    let m = |p: PathBuf| std::fs::metadata(p).ok().and_then(|m| m.modified().ok());
    (m(dir.join("team-policy.json")), m(dir.join("team-keys")), dir.to_path_buf())
}

/// The team policy now (re-read when its file or the key folder changes).
pub fn current() -> State {
    let dir = team_dir();
    let s = stamp(&dir);
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((old, st)) = g.as_ref() {
        if *old == s {
            return st.clone();
        }
    }
    let st = load(&dir);
    if let State::Broken(why) = &st {
        eprintln!("[xtraos-core] team policy broken: {why}");
    }
    *g = Some((s, st.clone()));
    st
}

pub fn status_json() -> Value {
    match current() {
        State::None => Value::Null,
        State::Active(t) => json!({"active": true, "key_id": t.key_id, "hash": t.hash}),
        State::Broken(why) => json!({"active": false, "broken": why}),
    }
}

// ---------------------------------------------------------------------------------------------
// Admin tools: `xtraos-core --team-keygen <dir>`, `--team-sign <policy.json> --key <k> --out <f>`

pub fn keygen(dir: &Path) -> Result<String, String> {
    let (public, private) = generate()?;
    let kid = key_id(&public);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (pubf, keyf) = (dir.join(format!("{kid}.pub")), dir.join(format!("{kid}.key")));
    if pubf.exists() || keyf.exists() {
        return Err("key files exist already".into());
    }
    std::fs::write(&pubf, b64(&public)).map_err(|e| e.to_string())?;
    std::fs::write(&keyf, b64(&private)).map_err(|e| e.to_string())?;
    Ok(format!(
        "key {kid}\n  public:  {} (copy to %ProgramData%\\XtraOS\\team-keys\\ on each PC)\n  private: {} (keep it with the admin; never on users' PCs)",
        pubf.display(),
        keyf.display()
    ))
}

pub fn sign_file(policy_file: &Path, key_file: &Path, out: &Path) -> Result<String, String> {
    let policy: Value = serde_json::from_str(&std::fs::read_to_string(policy_file).map_err(|e| e.to_string())?).map_err(|e| format!("policy is not JSON: {e}"))?;
    parse_policy(&policy)?;
    let private = unb64(std::fs::read_to_string(key_file).map_err(|e| e.to_string())?.trim()).ok_or("key file is not base64")?;
    let alg = Alg::ecdsa()?;
    let key = import(&alg, "ECCPRIVATEBLOB", &private)?;
    let public = export(&key, "ECCPUBLICBLOB")?;
    let sig = sign(&private, canonical(&policy).as_bytes())?;
    let env = json!({"policy": policy, "key_id": key_id(&public), "signature": b64(&sig)});
    std::fs::write(out, serde_json::to_string_pretty(&env).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(format!("signed with key {} → {}", key_id(&public), out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let (public, private) = generate().unwrap();
        assert_eq!(key_id(&public).len(), 16);
        let sig = sign(&private, b"{\"classes\":{\"file_send\":\"deny\"}}").unwrap();
        assert!(verify(&public, b"{\"classes\":{\"file_send\":\"deny\"}}", &sig));
        assert!(!verify(&public, b"{\"classes\":{\"file_send\":\"allow\"}}", &sig), "changed text");
        let (other, _) = generate().unwrap();
        assert!(!verify(&other, b"{\"classes\":{\"file_send\":\"deny\"}}", &sig), "another key");
        assert_eq!(unb64(&b64(&public)).unwrap(), public);
    }

    #[test]
    fn canonical_text_ignores_key_order_and_spacing() {
        let a: Value = serde_json::from_str("{\"b\": 1, \"a\": {\"y\": 2, \"x\": 3}}").unwrap();
        let b: Value = serde_json::from_str("{\"a\":{\"x\":3,\"y\":2},\"b\":1}").unwrap();
        assert_eq!(canonical(&a), canonical(&b));
    }

    #[test]
    fn team_rules_are_a_floor() {
        let t = parse_policy(&json!({"classes": {"file_send": "deny"}, "apps": {"WhatsApp.exe": "confirm"}, "clients": {"mcp:x": "read_only"}, "rate_per_minute": 30})).unwrap();
        assert_eq!(t.classes.get(&Class::FileSend), Some(&Decision::Deny));
        assert_eq!(t.apps.get("whatsapp.exe"), Some(&Decision::Confirm));
        assert_eq!(t.client("MCP:X"), Some(Profile::ReadOnly));
        assert_eq!(t.client("mcp:other"), None, "unnamed clients are the user's business");
        assert_eq!(t.rate_per_minute, Some(30));
        // stricter wins either way
        assert_eq!(stricter((Decision::Allow, "u".into()), (Decision::Deny, "t".into())).0, Decision::Deny);
        assert_eq!(stricter((Decision::Deny, "u".into()), (Decision::Allow, "t".into())).0, Decision::Deny);
        assert_eq!(stricter((Decision::Confirm, "u".into()), (Decision::Allow, String::new())).1, "u");
        assert!(rank(Profile::Blocked) > rank(Profile::ReadOnly) && rank(Profile::ReadOnly) > rank(Profile::BackgroundOnly));
        assert!(parse_policy(&json!({"classes": {"nope": "deny"}})).is_err());
        assert!(parse_policy(&json!({"clients": {"x": "superuser"}})).is_err());
    }

    #[test]
    fn load_verifies_and_fails_closed() {
        let d = std::env::temp_dir().join(format!("xtraos-team-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(load(&d), State::None);
        let admin = d.join("admin");
        let msg = keygen(&admin).unwrap();
        let kid = msg.lines().next().unwrap().trim_start_matches("key ").to_string();
        std::fs::create_dir_all(d.join("team-keys")).unwrap();
        std::fs::copy(admin.join(format!("{kid}.pub")), d.join("team-keys").join(format!("{kid}.pub"))).unwrap();
        std::fs::write(admin.join("p.json"), "{\"classes\": {\"file_send\": \"deny\"}}").unwrap();
        sign_file(&admin.join("p.json"), &admin.join(format!("{kid}.key")), &d.join("team-policy.json")).unwrap();
        match load(&d) {
            State::Active(t) => assert_eq!((t.classes.get(&Class::FileSend), t.key_id.as_str()), (Some(&Decision::Deny), kid.as_str())),
            other => panic!("{other:?}"),
        }
        // A program loosening the file breaks the signature: fail closed.
        let text = std::fs::read_to_string(d.join("team-policy.json")).unwrap().replace("\"deny\"", "\"allow\"");
        std::fs::write(d.join("team-policy.json"), text).unwrap();
        assert!(matches!(load(&d), State::Broken(w) if w.contains("signature")));
        // An untrusted key: broken too.
        std::fs::remove_file(d.join("team-keys").join(format!("{kid}.pub"))).unwrap();
        assert!(matches!(load(&d), State::Broken(w) if w.contains("no trusted key")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
