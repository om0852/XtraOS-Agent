//! Several clients at once (P8-M1, ADR 0059): per-window leases with a fair queue. An acting
//! call from a session leases the windows it targets; another session waits in a first-come
//! queue (≤ 10 s) and then gets WINDOW_BUSY. Reading is never blocked; the user is never
//! blocked. Leases lapse 20 s after the session's last action there, or when it disconnects.
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::input::{ActError, HWND, POINT};

pub const LEASE_MS: u64 = 20_000;
const WAIT_MS: u64 = 10_000;

const ACTING: &[&str] = &[
    "click", "type", "press_keys", "scroll", "drag", "mouse", "set_value", "touch", "pen", "focus_window",
    "close_window", "window_bounds",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Lease {
    pub session: String,
    pub client: String,
    pub last: u64,
}

/// The pure lease table (times in ms, given by the caller).
#[derive(Default)]
pub struct Table {
    leases: HashMap<usize, Lease>,
    queues: HashMap<usize, VecDeque<String>>,
}

impl Table {
    /// Takes (or renews) window `w` for session `s`, or says who holds it. A free window goes
    /// to the head of its queue first (first come, first served).
    pub fn try_take(&mut self, w: usize, s: &str, client: &str, now: u64) -> Result<(), Lease> {
        let held_by_other = self
            .leases
            .get(&w)
            .filter(|l| l.session != s && now.saturating_sub(l.last) < LEASE_MS)
            .cloned();
        let q = self.queues.entry(w).or_default();
        let my_turn = q.front().is_none_or(|h| h == s);
        match held_by_other {
            None if my_turn => {
                q.retain(|x| x != s);
                self.leases.insert(w, Lease { session: s.into(), client: client.into(), last: now });
                Ok(())
            }
            other => {
                if !q.iter().any(|x| x == s) {
                    q.push_back(s.into());
                }
                Err(other.unwrap_or_else(|| self.leases.get(&w).cloned().unwrap_or(Lease {
                    session: q.front().cloned().unwrap_or_default(),
                    client: "another agent (queued first)".into(),
                    last: now,
                })))
            }
        }
    }

    /// The session's action on `w` ended now: the lease runs from here.
    pub fn touch(&mut self, w: usize, s: &str, now: u64) {
        if let Some(l) = self.leases.get_mut(&w).filter(|l| l.session == s) {
            l.last = now;
        }
    }

    pub fn leave_queue(&mut self, w: usize, s: &str) {
        if let Some(q) = self.queues.get_mut(&w) {
            q.retain(|x| x != s);
        }
    }

    /// The session disconnected: its leases and queue places go.
    pub fn release_session(&mut self, s: &str) {
        self.leases.retain(|_, l| l.session != s);
        for q in self.queues.values_mut() {
            q.retain(|x| x != s);
        }
    }

    /// Live leases (not lapsed) as (window, lease).
    pub fn live(&self, now: u64) -> Vec<(usize, Lease)> {
        let mut v: Vec<_> = self
            .leases
            .iter()
            .filter(|(_, l)| now.saturating_sub(l.last) < LEASE_MS)
            .map(|(w, l)| (*w, l.clone()))
            .collect();
        v.sort_by_key(|(w, _)| *w);
        v
    }
}

// ---------------------------------------------------------------------------------------------
// The process-wide table and the session of the current action thread

static TABLE: Mutex<Option<Table>> = Mutex::new(None);
static FREED: Condvar = Condvar::new();
static T0: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static CURRENT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

fn now() -> u64 {
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// The IPC layer says which session this action thread serves (None: no session, e.g. tests
/// calling a tool directly — then nothing is leased).
pub fn set_current(s: Option<(String, String)>) {
    CURRENT.with(|c| *c.borrow_mut() = s);
}

fn current() -> Option<(String, String)> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Sets this thread's session and returns the previous one (a routine run inside a call).
pub(crate) fn replace_current(s: Option<(String, String)>) -> Option<(String, String)> {
    CURRENT.with(|c| std::mem::replace(&mut *c.borrow_mut(), s))
}

/// The client name of the session this action thread serves (P8-M2).
pub(crate) fn current_client() -> Option<String> {
    current().map(|(_, c)| c)
}

/// The windows an acting call targets.
pub(crate) fn targets(method: &str, p: &Value) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut add = |h: HWND| {
        if !h.is_null() && !crate::safety::is_own_window(h) && !out.contains(&(h as usize)) {
            out.push(h as usize);
        }
    };
    for k in ["window", "to_window"] {
        if let Some(h) = p.get(k).filter(|v| !v.is_null()).and_then(|v| crate::winmgr::resolve_window(v).ok()) {
            add(h);
        }
    }
    for k in ["ref", "from_ref", "to_ref"] {
        if let Some((h, _, _)) = p.get(k).and_then(crate::perceive::ref_facts) {
            add(h);
        }
    }
    if method != "window_bounds" {
        for (kx, ky) in [("x", "y"), ("from_x", "from_y"), ("to_x", "to_y")] {
            if let (Some(x), Some(y)) = (p.get(kx).and_then(Value::as_i64), p.get(ky).and_then(Value::as_i64)) {
                add(crate::winmgr::root_at(POINT { x: x as i32, y: y as i32 }));
            }
        }
    }
    out
}

fn with_table<T>(f: impl FnOnce(&mut Table) -> T) -> T {
    let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Table::default))
}

/// Before an acting call: lease its windows for this session, waiting in the queue ≤ 10 s.
/// Returns the leased windows (to touch when the action ends).
pub fn acquire(method: &str, p: &Value) -> Result<Vec<usize>, ActError> {
    if !ACTING.contains(&method) {
        return Ok(Vec::new());
    }
    let Some((s, client)) = current() else {
        return Ok(Vec::new());
    };
    let wins = targets(method, p);
    let t0 = Instant::now();
    for &w in &wins {
        let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            crate::safety::check()?;
            match g.get_or_insert_with(Table::default).try_take(w, &s, &client, now()) {
                Ok(()) => break,
                Err(holder) => {
                    if t0.elapsed() >= Duration::from_millis(WAIT_MS) {
                        g.get_or_insert_with(Table::default).leave_queue(w, &s);
                        drop(g);
                        FREED.notify_all();
                        let idle = now().saturating_sub(holder.last) / 1000;
                        return Err(ActError {
                            code: "WINDOW_BUSY",
                            message: format!(
                                "{} \"{}\" is in use by {} (session {}), last action {idle} s ago",
                                crate::winmgr::window_ref(w as HWND),
                                crate::perceive::clip(&crate::winmgr::title(w as HWND), 50),
                                holder.client,
                                holder.session
                            ),
                            hint: "Another agent is working in that window. Retry later, work in another window, or ask_human.",
                        });
                    }
                    g = FREED.wait_timeout(g, Duration::from_millis(100)).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
                }
            }
        }
    }
    Ok(wins)
}

/// After an acting call: the lease runs from now.
pub fn done(wins: &[usize]) {
    let Some((s, _)) = current() else { return };
    let t = now();
    with_table(|tb| {
        for &w in wins {
            tb.touch(w, &s, t);
        }
    });
}

/// A connection closed.
pub fn release_session(s: &str) {
    with_table(|tb| tb.release_session(s));
    FREED.notify_all();
}

/// `status.locks`.
pub fn status_json() -> Value {
    let t = now();
    let live = with_table(|tb| tb.live(t));
    Value::Array(
        live.into_iter()
            .map(|(w, l)| {
                json!({
                    "window": crate::winmgr::window_ref(w as HWND),
                    "title": crate::perceive::clip(&crate::winmgr::title(w as HWND), 60),
                    "client": l.client,
                    "session": l.session,
                    "idle_s": t.saturating_sub(l.last) / 1000,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_renew_lapse_and_hand_over_fairly() {
        let mut t = Table::default();
        assert!(t.try_take(1, "s1", "claude", 0).is_ok());
        assert!(t.try_take(1, "s1", "claude", 5_000).is_ok(), "the holder renews");
        let busy = t.try_take(1, "s2", "codex", 6_000).unwrap_err();
        assert_eq!(busy.session, "s1");
        assert!(t.try_take(1, "s3", "hermes", 6_500).is_err(), "s3 queues behind s2");
        assert!(t.try_take(2, "s2", "codex", 6_000).is_ok(), "other windows are free");
        // s1 stops acting: 20 s after its last action the window frees, s2 (first in line) gets it.
        assert!(t.try_take(1, "s3", "hermes", 26_000).is_err(), "not s3's turn yet");
        assert!(t.try_take(1, "s2", "codex", 26_000).is_ok());
        assert_eq!(t.live(26_000)[0].1.session, "s2");
        // s2 disconnects: s3 is next.
        t.release_session("s2");
        assert!(t.try_take(1, "s3", "hermes", 27_000).is_ok());
        // touch moves the lease start.
        t.touch(1, "s3", 40_000);
        assert!(t.try_take(1, "s4", "x", 55_000).is_err(), "15 s after the touch: still held");
        // giving up leaves the queue so others are not stuck behind a ghost
        t.leave_queue(1, "s4");
        assert!(t.try_take(1, "s5", "y", 61_000).is_ok(), "lapsed and s4 left the queue");
        assert!(t.live(100_000).is_empty());
    }
}
