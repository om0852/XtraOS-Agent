//! P8-M3 (ADR 0061): which agents are connected, what each did last, and pause per agent.
//!
//! Only method names are kept — never arguments or typed text. Pause/resume is driven by the
//! tray menu (the user); agents only see the effect (`STOPPED`, `status.state: "paused"`).

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::input::ActError;

/// The viewer window talks to the core as this client; it is not an agent.
pub const VIEWER_CLIENT: &str = "xtraos-viewer";
const KEEP_ENDED: Duration = Duration::from_secs(600);
const MAX_ENTRIES: usize = 20;

#[derive(Clone, Debug)]
pub struct Agent {
    pub session: String,
    pub client: String,
    pub connected: bool,
    pub since: Instant,
    pub ended: Option<Instant>,
    pub last_method: Option<String>,
    pub last_at: Option<Instant>,
    pub calls: u64,
    pub refused: u64,
}

/// The agent list (pure, unit-tested).
#[derive(Default)]
pub struct Table {
    pub list: Vec<Agent>,
}

impl Table {
    pub fn connected(&mut self, session: &str, client: &str, now: Instant) {
        if self.list.iter().any(|a| a.session == session) {
            return;
        }
        self.prune(now);
        self.list.push(Agent {
            session: session.into(),
            client: client.into(),
            connected: true,
            since: now,
            ended: None,
            last_method: None,
            last_at: None,
            calls: 0,
            refused: 0,
        });
        // Cap: drop the oldest ended entries first, then the oldest.
        while self.list.len() > MAX_ENTRIES {
            let i = self.list.iter().position(|a| !a.connected).unwrap_or(0);
            self.list.remove(i);
        }
    }

    pub fn acted(&mut self, session: &str, method: &str, refused: bool, now: Instant) {
        if let Some(a) = self.list.iter_mut().find(|a| a.session == session) {
            a.last_method = Some(method.into());
            a.last_at = Some(now);
            a.calls += 1;
            if refused {
                a.refused += 1;
            }
        }
    }

    pub fn disconnected(&mut self, session: &str, now: Instant) {
        if let Some(a) = self.list.iter_mut().find(|a| a.session == session) {
            a.connected = false;
            a.ended = Some(now);
        }
    }

    fn prune(&mut self, now: Instant) {
        self.list
            .retain(|a| a.connected || a.ended.is_none_or(|e| now.duration_since(e) < KEEP_ENDED));
    }
}

/// Listed on the dashboard: everyone except the viewer window — and a "viewer" that does
/// anything besides `status` (which is not an action) is listed too, so no agent can hide.
pub fn visible(a: &Agent) -> bool {
    a.client != VIEWER_CLIENT || a.calls > 0
}

/// Tray item text for one agent (pure).
pub fn label(a: &Agent, paused: bool, now: Instant) -> String {
    let last = match (&a.last_method, a.last_at) {
        (Some(m), Some(t)) => format!("last: {m}, {} ago", ago(now.duration_since(t))),
        _ => "no actions yet".into(),
    };
    let calls = if a.refused > 0 {
        format!("{} calls, {} refused", a.calls, a.refused)
    } else {
        format!("{} calls", a.calls)
    };
    if paused {
        format!("Resume {} (paused) — {last}, {calls}", a.client)
    } else {
        format!("Pause {} — {last}, {calls}", a.client)
    }
}

fn ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

// ---------------------------------------------------------------------------------------------
// Process-wide state

static TABLE: Mutex<Option<Table>> = Mutex::new(None);
static PAUSED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn with_table<T>(f: impl FnOnce(&mut Table) -> T) -> T {
    let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Table::default))
}

fn with_paused<T>(f: impl FnOnce(&mut HashSet<String>) -> T) -> T {
    let mut g = PAUSED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashSet::new))
}

pub fn connected(session: &str, client: &str) {
    with_table(|t| t.connected(session, client, Instant::now()));
}

pub fn disconnected(session: &str) {
    with_table(|t| t.disconnected(session, Instant::now()));
}

pub fn acted(session: &str, method: &str, error_code: Option<&str>) {
    let refused = matches!(error_code, Some("BLOCKED_BY_POLICY") | Some("STOPPED"));
    with_table(|t| t.acted(session, method, refused, Instant::now()));
}

pub fn is_paused(client: &str) -> bool {
    with_paused(|p| p.contains(&client.to_lowercase()))
}

/// Only the tray menu (the user) calls these.
pub fn set_paused(client: &str, on: bool) {
    with_paused(|p| {
        if on {
            p.insert(client.to_lowercase());
        } else {
            p.remove(&client.to_lowercase());
        }
    });
}

/// Called by every safety check: a paused agent's calls stop, others go on.
pub fn check_current() -> Result<(), ActError> {
    let Some(client) = crate::locks::current_client() else {
        return Ok(());
    };
    if is_paused(&client) {
        return Err(ActError {
            code: "STOPPED",
            message: format!("the user paused this agent ({client}); other agents keep working"),
            hint: "Stop and tell the user. Only the user can resume this agent, from the XtraOS tray icon → Agents.",
        });
    }
    Ok(())
}

/// Connected agents with their tray labels (for the tray submenu).
pub fn menu_items() -> Vec<(String, String)> {
    let now = Instant::now();
    let list = with_table(|t| t.list.clone());
    let mut seen = HashSet::new();
    list.into_iter()
        .rev() // newest session of a client first
        .filter(|a| a.connected && visible(a) && seen.insert(a.client.to_lowercase()))
        .map(|a| {
            let l = label(&a, is_paused(&a.client), now);
            (a.client, l)
        })
        .collect()
}

/// `status.agents`.
pub fn status_json() -> Value {
    let now = Instant::now();
    let list = with_table(|t| {
        t.prune(now);
        t.list.clone()
    });
    Value::Array(
        list.iter()
            .filter(|a| visible(a))
            .map(|a| {
                json!({
                    "session": a.session,
                    "client": a.client,
                    "connected": a.connected,
                    "paused": is_paused(&a.client),
                    "last_action": a.last_method,
                    "idle_s": a.last_at.map(|t| now.duration_since(t).as_secs()),
                    "calls": a.calls,
                    "refused": a.refused,
                    "connected_s": now.duration_since(a.since).as_secs(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_sessions_and_actions() {
        let t0 = Instant::now();
        let mut t = Table::default();
        t.connected("s1", "mcp:claude-code", t0);
        t.connected("s2", "mcp:codex", t0);
        t.connected("s1", "mcp:claude-code", t0); // twice: once
        assert_eq!(t.list.len(), 2);
        // the viewer is not listed — until it acts like an agent
        t.connected("s9", VIEWER_CLIENT, t0);
        assert!(!visible(&t.list[2]));
        t.acted("s9", "click", false, t0);
        assert!(visible(&t.list[2]));
        t.list.pop();
        t.acted("s1", "click", false, t0 + Duration::from_secs(1));
        t.acted("s1", "type", true, t0 + Duration::from_secs(2));
        let a = &t.list[0];
        assert_eq!((a.calls, a.refused), (2, 1));
        assert_eq!(a.last_method.as_deref(), Some("type"));
        let l = label(a, false, t0 + Duration::from_secs(6));
        assert_eq!(l, "Pause mcp:claude-code — last: type, 4 s ago, 2 calls, 1 refused");
        assert!(label(&t.list[1], true, t0).starts_with("Resume mcp:codex (paused) — no actions yet"));
        // ended sessions stay 10 minutes
        t.disconnected("s2", t0);
        t.prune(t0 + Duration::from_secs(599));
        assert_eq!(t.list.len(), 2);
        t.prune(t0 + Duration::from_secs(601));
        assert_eq!(t.list.len(), 1);
    }

    #[test]
    fn list_is_capped_dropping_ended_first() {
        let t0 = Instant::now();
        let mut t = Table::default();
        for i in 0..MAX_ENTRIES {
            t.connected(&format!("s{i}"), "c", t0);
        }
        t.disconnected("s5", t0);
        t.connected("new", "c", t0);
        assert_eq!(t.list.len(), MAX_ENTRIES);
        assert!(!t.list.iter().any(|a| a.session == "s5"));
        t.connected("new2", "c", t0);
        assert!(!t.list.iter().any(|a| a.session == "s0"), "then the oldest");
    }

    #[test]
    fn pause_is_by_client_name() {
        set_paused("mcp:Test-Pause", true);
        assert!(is_paused("MCP:test-pause"));
        assert!(!is_paused("mcp:other"));
        set_paused("mcp:test-pause", false);
        assert!(!is_paused("mcp:test-pause"));
    }

    #[test]
    fn ago_units() {
        assert_eq!(ago(Duration::from_secs(5)), "5 s");
        assert_eq!(ago(Duration::from_secs(125)), "2 min");
        assert_eq!(ago(Duration::from_secs(7300)), "2 h");
    }
}
