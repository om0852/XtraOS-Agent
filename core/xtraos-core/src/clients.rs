//! P8-M2 (ADR 0060): per-client profiles — full, background only, read only, blocked.
//!
//! The client name comes from the pairing (`mcp:<clientInfo.name>`, `xtraos-cli`, …). Names are
//! self-declared by local processes that already hold the user's pairing token: profiles keep
//! well-behaved agents within bounds; they are not a boundary against a malicious program.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::input::ActError;
use crate::policy::{is_read_method, ClientRule, NewClients, Policy, Profile};

/// Choices the user made in the "new agent" dialog, by lower-case client name.
static STORED: Mutex<Option<HashMap<String, Profile>>> = Mutex::new(None);

fn stored_path() -> PathBuf {
    crate::pairing::data_dir().join("clients.json")
}

fn load(path: &Path) -> HashMap<String, Profile> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| Profile::parse(&v).map(|p| (k.to_lowercase(), p)))
                .collect()
        })
        .unwrap_or_default()
}

fn with_stored<T>(f: impl FnOnce(&mut HashMap<String, Profile>) -> T) -> T {
    let mut g = STORED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| load(&stored_path())))
}

fn remember(client: &str, profile: Profile) {
    with_stored(|m| {
        m.insert(client.to_lowercase(), profile);
        let out: HashMap<&String, &str> = m.iter().map(|(k, v)| (k, v.name())).collect();
        if let Ok(s) = serde_json::to_string_pretty(&out) {
            let _ = std::fs::write(stored_path(), s);
        }
    });
}

/// The rule for a client (pure). Order: its own policy entry, the user's dialog choice, the `*`
/// entry, then `new_clients`. None: not decided yet (`new_clients: "ask"`).
pub fn resolve(policy: &Policy, stored: &HashMap<String, Profile>, client: &str) -> Option<ClientRule> {
    let key = client.to_lowercase();
    if let Some(r) = policy.clients.get(&key) {
        return Some(r.clone());
    }
    if let Some(&profile) = stored.get(&key) {
        return Some(ClientRule { profile, apps: None });
    }
    if let Some(r) = policy.clients.get("*") {
        return Some(r.clone());
    }
    match policy.new_clients {
        NewClients::Full => Some(ClientRule { profile: Profile::Full, apps: None }),
        NewClients::ReadOnly => Some(ClientRule { profile: Profile::ReadOnly, apps: None }),
        NewClients::Ask => None,
    }
}

/// Whether a process name is in an `apps` list ("chrome" and "chrome.exe" both match).
pub fn app_allowed(apps: &[String], process: &str) -> bool {
    let p = process.to_lowercase();
    let stem = p.strip_suffix(".exe").unwrap_or(&p);
    apps.iter().any(|a| {
        let a = a.to_lowercase();
        a == p || a.strip_suffix(".exe").unwrap_or(&a) == stem
    })
}

/// What a profile refuses for a call (pure; None = allowed by the profile itself).
pub fn refusal(profile: Profile, method: &str, read: bool) -> Option<String> {
    match profile {
        Profile::Full => None,
        Profile::Blocked if method != "status" => {
            Some("this agent is blocked on this PC (the user set its profile to \"blocked\")".into())
        }
        Profile::Blocked => None,
        Profile::ReadOnly if !read => Some(format!(
            "this agent may only read the screen (profile \"read_only\"); '{method}' would change something"
        )),
        Profile::BackgroundOnly if method == "focus_window" => Some(
            "this agent may act in the background only (profile \"background_only\"); it may not bring windows to the front".into(),
        ),
        _ => None,
    }
}

/// Calls that are real input by their arguments alone (pure): refused for a background-only
/// client before anything waits. Others are caught where input would be sent.
pub fn needs_real_input(method: &str, p: &Value) -> bool {
    let has = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    match method {
        "mouse" | "touch" | "pen" => true,
        "click" | "scroll" => !has("ref") && (has("x") || has("y")),
        "drag" => !(has("from_ref") && has("to_ref")),
        _ => false,
    }
}

/// The caller's profile name for `status` (never shows a dialog).
pub fn profile_name(client: &str) -> String {
    let stored = with_stored(|m| m.clone());
    crate::safety::peek_policy(|pol| match resolve(pol, &stored, client) {
        Some(ClientRule { profile, apps: None }) => profile.name().to_string(),
        Some(ClientRule { profile, apps: Some(a) }) => format!("{} (only in {})", profile.name(), a.join(", ")),
        None => "not decided (read only until the user chooses)".into(),
    })
}

fn is_read(method: &str, p: &Value) -> bool {
    let skill_observe = method == "run_skill"
        && crate::skills::plan(p).is_ok_and(|pl| pl.is_macro || pl.class == crate::policy::Class::Observe);
    is_read_method(method, p, skill_observe)
}

/// Before every call (safety::authorize): refuse what this client's profile does not allow, and
/// arm the no-real-input switch for a background-only client.
pub fn enforce(method: &str, p: &Value) -> Result<(), ActError> {
    crate::input::set_background_only(false);
    let Some(client) = crate::locks::current_client() else {
        return Ok(()); // no session (in-process tests): nothing to scope
    };
    let read = is_read(method, p);
    let stored = with_stored(|m| m.clone());
    let rule = crate::safety::with_policy(|s| resolve(&s.policy, &stored, &client)).0;
    let rule = match rule {
        Some(r) => r,
        // Reading is fine until the user decides; the first change asks.
        None if read => ClientRule { profile: Profile::ReadOnly, apps: None },
        None => ask(&client, method)?,
    };
    if let Some(why) = refusal(rule.profile, method, read) {
        return Err(crate::safety::blocked(why));
    }
    if rule.profile == Profile::BackgroundOnly && needs_real_input(method, p) {
        crate::input::set_background_only(true);
        return crate::input::real_input_allowed();
    }
    if let (Some(apps), false) = (&rule.apps, read) {
        let mut names: Vec<String> = crate::locks::targets(method, p)
            .into_iter()
            .map(|h| crate::winmgr::process_name(crate::winmgr::pid_of(h as _)))
            .collect();
        if names.is_empty() {
            if method == "launch_app" {
                names.push(p.get("app").and_then(Value::as_str).unwrap_or("").to_string());
            } else {
                let fg = crate::winmgr::foreground_root();
                names.push(crate::winmgr::process_name(crate::winmgr::pid_of(fg)));
            }
        }
        if let Some(bad) = names.iter().find(|n| !app_allowed(apps, n)) {
            return Err(crate::safety::blocked(format!(
                "this agent may only act in {} (its profile's apps list); the target is '{}'",
                apps.join(", "),
                if bad.is_empty() { "unknown" } else { bad }
            )));
        }
    }
    crate::input::set_background_only(rule.profile == Profile::BackgroundOnly);
    Ok(())
}

/// First change by an unknown client with `new_clients: "ask"`: the user picks its profile.
fn ask(client: &str, method: &str) -> Result<ClientRule, ActError> {
    crate::safety::activity("waiting for you: new agent");
    let answer = crate::safety::prompt(
        "XtraOS Agent — a new AI agent",
        &format!(
            "An AI agent called \"{}\" wants to use this PC for the first time.\n\nNext: {method}\n\nWhat may it do?\n\nFull access — like your other agents\nBackground only — no real mouse or keyboard\nRead only — it may look, not change anything\nBlock — nothing",
            crate::perceive::clip(client, 60)
        ),
        vec!["Full access".into(), "Background only".into(), "Read only".into(), "Block".into()],
        2,
        Duration::from_secs(120),
    )?;
    let profile = match answer {
        Some(0) => Profile::Full,
        Some(1) => Profile::BackgroundOnly,
        Some(3) => Profile::Blocked,
        Some(_) => Profile::ReadOnly,
        // No answer: read only for now, ask again next time.
        None => return Ok(ClientRule { profile: Profile::ReadOnly, apps: None }),
    };
    remember(client, profile);
    Ok(ClientRule { profile, apps: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pol(v: Value) -> Policy {
        Policy::from_json(&v.to_string())
    }

    #[test]
    fn profiles_parse_and_resolve() {
        let p = pol(json!({
            "clients": {
                "mcp:Codex": "background_only",
                "mcp:hermes": {"profile": "full", "apps": ["Chrome.exe"]},
                "mcp:bad": "superuser",
                "*": "read_only"
            },
            "new_clients": "ask"
        }));
        assert!(p.warnings.iter().any(|w| w.contains("mcp:bad")), "{:?}", p.warnings);
        let none = HashMap::new();
        assert_eq!(resolve(&p, &none, "MCP:CODEX").unwrap().profile, Profile::BackgroundOnly);
        let h = resolve(&p, &none, "mcp:hermes").unwrap();
        assert_eq!(h.profile, Profile::Full);
        assert_eq!(h.apps.as_deref(), Some(&["chrome.exe".to_string()][..]));
        // `*` covers everyone else, before new_clients
        assert_eq!(resolve(&p, &none, "mcp:other").unwrap().profile, Profile::ReadOnly);
        // a stored choice beats `*`, but not the client's own entry
        let mut st = HashMap::new();
        st.insert("mcp:other".to_string(), Profile::Full);
        st.insert("mcp:codex".to_string(), Profile::Full);
        assert_eq!(resolve(&p, &st, "mcp:other").unwrap().profile, Profile::Full);
        assert_eq!(resolve(&p, &st, "mcp:codex").unwrap().profile, Profile::BackgroundOnly);
    }

    #[test]
    fn new_clients_default_is_full() {
        let none = HashMap::new();
        assert_eq!(resolve(&pol(json!({})), &none, "x").unwrap().profile, Profile::Full);
        let ro = pol(json!({"new_clients": "read_only"}));
        assert_eq!(resolve(&ro, &none, "x").unwrap().profile, Profile::ReadOnly);
        let ask = pol(json!({"new_clients": "ask"}));
        assert!(resolve(&ask, &none, "x").is_none());
        assert!(pol(json!({"new_clients": "maybe"})).warnings.iter().any(|w| w.contains("new_clients")));
    }

    #[test]
    fn read_methods_and_refusals() {
        assert!(is_read_method("observe", &json!({}), false));
        assert!(is_read_method("system", &json!({"op": "health"}), false));
        assert!(is_read_method("system", &json!({"op": "brightness"}), false));
        assert!(!is_read_method("system", &json!({"op": "brightness", "value": 50}), false));
        assert!(is_read_method("volume", &json!({"op": "get"}), false));
        assert!(!is_read_method("volume", &json!({"op": "set", "level": 10}), false));
        assert!(!is_read_method("click", &json!({"ref": "e1"}), false));
        assert!(!is_read_method("type", &json!({}), false));
        assert!(is_read_method("run_skill", &json!({}), true));
        assert!(!is_read_method("run_skill", &json!({}), false));
        assert!(is_read_method("app_map", &json!({"op": "get"}), false));
        assert!(!is_read_method("app_map", &json!({"op": "learn"}), false));

        assert!(refusal(Profile::Full, "click", false).is_none());
        assert!(refusal(Profile::ReadOnly, "observe", true).is_none());
        assert!(refusal(Profile::ReadOnly, "click", false).is_some());
        assert!(refusal(Profile::Blocked, "observe", true).is_some());
        assert!(refusal(Profile::Blocked, "status", true).is_none());
        assert!(refusal(Profile::BackgroundOnly, "click", false).is_none());
        assert!(refusal(Profile::BackgroundOnly, "focus_window", false).is_some());
    }

    #[test]
    fn real_input_by_arguments() {
        assert!(needs_real_input("mouse", &json!({"action": "move"})));
        assert!(needs_real_input("touch", &json!({})));
        assert!(needs_real_input("click", &json!({"x": 1, "y": 2})));
        assert!(!needs_real_input("click", &json!({"ref": "e1"})));
        assert!(needs_real_input("scroll", &json!({"x": 1, "y": 2, "dy": 3})));
        assert!(!needs_real_input("scroll", &json!({"ref": "e1", "dy": 3})));
        assert!(needs_real_input("drag", &json!({"from_ref": "e1", "to_x": 3, "to_y": 4})));
        assert!(!needs_real_input("drag", &json!({"from_ref": "e1", "to_ref": "e2"})));
        assert!(!needs_real_input("set_value", &json!({"ref": "e1", "value": "x"})));
    }

    #[test]
    fn apps_allowlist_matches_with_or_without_exe() {
        let apps = vec!["chrome.exe".to_string(), "notepad".to_string()];
        assert!(app_allowed(&apps, "chrome.exe"));
        assert!(app_allowed(&apps, "Chrome"));
        assert!(app_allowed(&apps, "notepad.exe"));
        assert!(!app_allowed(&apps, "explorer.exe"));
        assert!(!app_allowed(&apps, ""));
    }

    #[test]
    fn client_changes_need_approval() {
        let a = pol(json!({}));
        let b = pol(json!({"clients": {"mcp:x": "full"}, "new_clients": "ask"}));
        let d = crate::policy::summary(&a, &b);
        assert!(d.iter().any(|l| l.contains("client mcp:x")), "{d:?}");
        assert!(d.iter().any(|l| l.contains("new_clients")), "{d:?}");
    }
}
