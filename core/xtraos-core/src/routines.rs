//! P7-M4 routines (ADR 0063): run a skill on a schedule, inside the core (no Task Scheduler).
//!
//! Adding one always asks the user. Each run is a normal agent call (client `routine:<name>`)
//! through `safety::authorize`, so the kill switch, per-agent pause, profiles and the policy all
//! apply, and it is written to the action log.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::actionlog::{ActionLog, Entry};
use crate::input::ActError;

const MAX_ROUTINES: usize = 20;
const MIN_EVERY: u32 = 5;
const MAX_RUN_LINES: usize = 2_000;
const TICK: Duration = Duration::from_secs(20);
const DAY_MS: i64 = 86_400_000;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "See list_skills for routine.* arguments." }
}

// ---------------------------------------------------------------------------------------------
// Local time (pure helpers; the PC's local clock)

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// "Local milliseconds": local wall-clock time counted like Unix time.
pub fn local_now_ms() -> i64 {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (days_from_civil(t.wYear as i64, t.wMonth as i64, t.wDay as i64) * DAY_MS)
        + (t.wHour as i64 * 3600 + t.wMinute as i64 * 60 + t.wSecond as i64) * 1000
        + t.wMilliseconds as i64
}

/// "2026-11-11 09:00" for local milliseconds (pure).
pub fn show_time(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(DAY_MS));
    let mins = ms.rem_euclid(DAY_MS) / 60_000;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", mins / 60, mins % 60)
}

// ---------------------------------------------------------------------------------------------
// Schedule (pure, unit-tested)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    Every(u32),
    Daily(u32, u32),
}

impl Schedule {
    pub fn parse(a: &Value) -> Result<Schedule, ActError> {
        let every = a.get("every_minutes").filter(|v| !v.is_null());
        let daily = a.get("daily_at").filter(|v| !v.is_null());
        match (every, daily) {
            (Some(e), None) => {
                let n = e.as_u64().ok_or_else(|| bad("'every_minutes' must be a number"))?;
                if n < MIN_EVERY as u64 || n > 7 * 24 * 60 {
                    return Err(bad(format!("'every_minutes' must be {MIN_EVERY}..10080")));
                }
                Ok(Schedule::Every(n as u32))
            }
            (None, Some(d)) => {
                let s = d.as_str().unwrap_or("");
                let (h, m) = s.split_once(':').ok_or_else(|| bad("'daily_at' must be \"HH:MM\""))?;
                match (h.trim().parse::<u32>(), m.trim().parse::<u32>()) {
                    (Ok(h), Ok(m)) if h < 24 && m < 60 => Ok(Schedule::Daily(h, m)),
                    _ => Err(bad("'daily_at' must be \"HH:MM\" (00:00..23:59)")),
                }
            }
            _ => Err(bad("give exactly one of 'every_minutes' or 'daily_at'")),
        }
    }

    pub fn describe(self) -> String {
        match self {
            Schedule::Every(n) if n % 60 == 0 => format!("every {} hour(s)", n / 60),
            Schedule::Every(n) => format!("every {n} minutes"),
            Schedule::Daily(h, m) => format!("every day at {h:02}:{m:02}"),
        }
    }

    fn to_json(self) -> Value {
        match self {
            Schedule::Every(n) => json!({"every_minutes": n}),
            Schedule::Daily(h, m) => json!({"daily_at": format!("{h:02}:{m:02}")}),
        }
    }

    /// Whether a run is due at `now` (local ms). A missed slot runs once, never more.
    pub fn due(self, created: i64, last: Option<i64>, now: i64) -> bool {
        let since = last.unwrap_or(created).max(created);
        match self {
            Schedule::Every(n) => now - since >= n as i64 * 60_000,
            Schedule::Daily(h, m) => {
                let today = now.div_euclid(DAY_MS) * DAY_MS + (h as i64 * 60 + m as i64) * 60_000;
                let slot = if now >= today { today } else { today - DAY_MS };
                since < slot && now >= slot
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Routines file

#[derive(Clone, Debug, PartialEq)]
pub struct Routine {
    pub name: String,
    pub skill: String,
    pub args: Value,
    pub schedule: Schedule,
    pub created: i64,
    pub last: Option<i64>,
    pub added_by: String,
}

impl Routine {
    fn to_json(&self) -> Value {
        let mut v = json!({
            "name": self.name, "skill": self.skill, "args": self.args,
            "created": self.created, "last_run": self.last, "added_by": self.added_by,
        });
        if let (Some(o), Some(s)) = (v.as_object_mut(), self.schedule.to_json().as_object()) {
            o.extend(s.clone());
        }
        v
    }

    fn from_json(v: &Value) -> Option<Routine> {
        Some(Routine {
            name: v["name"].as_str()?.to_string(),
            skill: v["skill"].as_str()?.to_string(),
            args: v.get("args").cloned().unwrap_or(json!({})),
            schedule: Schedule::parse(v).ok()?,
            created: v["created"].as_i64()?,
            last: v["last_run"].as_i64(),
            added_by: v["added_by"].as_str().unwrap_or("").to_string(),
        })
    }
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 40 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A routine from `routine.add` arguments, checked (pure apart from the skill plan check).
pub fn from_args(a: &Value, created: i64, added_by: &str) -> Result<Routine, ActError> {
    let name = a.get("name").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    if !valid_name(&name) {
        return Err(bad("'name' must be 1..40 characters: letters, digits, - _ ."));
    }
    let skill = a.get("skill").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if skill.is_empty() {
        return Err(bad("'skill' is required (a name from list_skills)"));
    }
    if skill.starts_with("routine.") {
        return Err(bad("a routine cannot schedule routine.* skills"));
    }
    let args = a.get("args").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
    if !args.is_object() {
        return Err(bad("'args' must be an object"));
    }
    if args.to_string().len() > 16_384 {
        return Err(bad("'args' is limited to 16 KB"));
    }
    if let Some(k) = crate::tasks::secret_key(&args) {
        return Err(crate::safety::blocked(format!(
            "the routine's arguments contain '{k}', which looks like a credential; routines never store secrets"
        )));
    }
    Ok(Routine { name, skill, args, schedule: Schedule::parse(a)?, created, last: None, added_by: added_by.into() })
}

struct Store {
    dir: PathBuf,
    list: Vec<Routine>,
    /// False when routines.json was changed outside XtraOS and the user has not approved it.
    trusted: bool,
}

impl Store {
    fn file(&self) -> PathBuf {
        self.dir.join("routines.json")
    }

    fn load(dir: &Path) -> Store {
        let text = std::fs::read_to_string(dir.join("routines.json")).unwrap_or_default();
        let list: Vec<Routine> = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(Routine::from_json)
            .collect();
        let ok = std::fs::read_to_string(dir.join("routines.ok")).unwrap_or_default();
        let trusted = text.is_empty() || ok.trim() == crate::actionlog::sha256_hex(text.as_bytes());
        Store { dir: dir.to_path_buf(), list, trusted }
    }

    fn save(&mut self) -> Result<(), ActError> {
        let text = serde_json::to_string_pretty(&Value::Array(self.list.iter().map(Routine::to_json).collect()))
            .unwrap_or_else(|_| "[]".into());
        let io = |e: std::io::Error| ActError { code: "INTERNAL", message: format!("routines: {e}"), hint: "" };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        std::fs::write(self.file(), &text).map_err(io)?;
        std::fs::write(self.dir.join("routines.ok"), crate::actionlog::sha256_hex(text.as_bytes())).map_err(io)?;
        self.trusted = true;
        Ok(())
    }
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = crate::pairing::data_dir();
    if g.as_ref().is_none_or(|s| s.dir != dir) {
        *g = Some(Store::load(&dir));
    }
    f(g.as_mut().expect("loaded"))
}

// ---------------------------------------------------------------------------------------------
// Run history

fn runs_file() -> PathBuf {
    crate::pairing::data_dir().join("routines-runs.jsonl")
}

fn record_run(name: &str, at: i64, ok: bool, code: Option<&str>, text: &str) {
    let line = json!({"name": name, "at": at, "ok": ok, "code": code, "text": crate::perceive::clip(text, 200)}).to_string();
    let path = runs_file();
    let mut lines: Vec<String> = std::fs::read_to_string(&path).unwrap_or_default().lines().map(String::from).collect();
    lines.push(line);
    let keep = trim_runs(lines);
    let _ = std::fs::write(&path, keep.join("\n") + "\n");
}

/// The newest MAX_RUN_LINES lines (pure).
pub fn trim_runs(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() > MAX_RUN_LINES {
        lines.drain(..lines.len() - MAX_RUN_LINES);
    }
    lines
}

/// Summary text of runs since `since` (pure).
pub fn summarize(runs: &[Value], since: i64, names: &[String]) -> (String, Value) {
    let recent: Vec<&Value> = runs.iter().filter(|r| r["at"].as_i64().unwrap_or(0) >= since).collect();
    let mut all: Vec<String> = names.to_vec();
    for r in &recent {
        if let Some(n) = r["name"].as_str() {
            if !all.iter().any(|x| x == n) {
                all.push(n.to_string());
            }
        }
    }
    let mut out = Vec::new();
    let mut text = String::new();
    for n in &all {
        let mine: Vec<&&Value> = recent.iter().filter(|r| r["name"] == n.as_str()).collect();
        let fails = mine.iter().filter(|r| r["ok"] != true).count();
        let last = mine.last();
        text.push_str(&format!(
            "{n}: {} run(s), {fails} failed{}\n",
            mine.len(),
            last.map(|l| format!("; last {} {}: {}", show_time(l["at"].as_i64().unwrap_or(0)), if l["ok"] == true { "ok" } else { l["code"].as_str().unwrap_or("failed") }, l["text"].as_str().unwrap_or(""))).unwrap_or_default()
        ));
        out.push(json!({"name": n, "runs": mine.len(), "failed": fails, "last": last.map(|l| (**l).clone())}));
    }
    if all.is_empty() {
        text.push_str("no routines\n");
    }
    (text.trim_end().to_string(), Value::Array(out))
}

// ---------------------------------------------------------------------------------------------
// Running

static LOG: Mutex<Option<Arc<ActionLog>>> = Mutex::new(None);

/// One run of a routine as its own agent (client `routine:<name>`), through authorize.
pub fn execute(r: &Routine) -> Result<Value, ActError> {
    let session = format!("routine-{}", r.name);
    let client = format!("routine:{}", r.name);
    crate::agents::connected(&session, &client);
    let caller = crate::locks::replace_current(Some((session.clone(), client.clone())));
    let p = json!({"name": r.skill, "args": r.args});
    let t0 = std::time::Instant::now();
    let res = crate::safety::authorize("run_skill", &p).and_then(|_| crate::skills::run(&p, &mut |_| {}));
    crate::locks::replace_current(caller); // every later call is authorised again for its caller
    crate::safety::activity_end();
    crate::agents::acted(&session, "run_skill", res.as_ref().err().map(|e| e.code));
    if let Some(log) = LOG.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let method = format!("run_skill:{}", r.skill);
        log.record(&Entry {
            ts_ms: crate::protocol::now_ms(),
            session: Some(&session),
            client: Some(&client),
            method: &method,
            ok: res.is_ok(),
            code: res.as_ref().err().map(|e| e.code),
            ms: t0.elapsed().as_secs_f64() * 1000.0,
            policy: None,
        });
    }
    let now = local_now_ms();
    let text = match &res {
        Ok(v) => v["text"].as_str().map(String::from).unwrap_or_else(|| v.to_string()),
        Err(e) => e.message.clone(),
    };
    record_run(&r.name, now, res.is_ok(), res.as_ref().err().map(|e| e.code), &text);
    with_store(|s| {
        if let Some(x) = s.list.iter_mut().find(|x| x.name == r.name) {
            x.last = Some(now);
            let _ = s.save();
        }
    });
    res
}

/// One scheduler tick: run every due routine (in order). Returns the names run.
pub fn run_due(now: i64) -> Vec<String> {
    if crate::safety::stopped() {
        return Vec::new();
    }
    if !with_store(|s| s.trusted) && !approve_changed_file() {
        return Vec::new();
    }
    let due: Vec<Routine> = with_store(|s| s.list.iter().filter(|r| r.schedule.due(r.created, r.last, now)).cloned().collect());
    for r in &due {
        let _ = execute(r);
    }
    due.into_iter().map(|r| r.name).collect()
}

/// routines.json changed outside XtraOS: run nothing until the user approves what is in it.
fn approve_changed_file() -> bool {
    static ASKED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let mut g = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some_and(|t| t.elapsed() < Duration::from_secs(3600)) {
            return false; // asked within the hour: do not nag
        }
        *g = Some(std::time::Instant::now());
    }
    let list = with_store(|s| s.list.clone());
    let shown: Vec<String> = list.iter().map(|r| format!("• {} — {} {}", r.name, r.skill, r.schedule.describe())).collect();
    let answer = crate::safety::prompt(
        "XtraOS Agent — routines were changed",
        &format!(
            "The routines file was changed outside XtraOS (by you or by a program). These routines would run on their own:\n\n{}\n\nKeep running them?",
            if shown.is_empty() { "(none)".to_string() } else { shown.join("\n") }
        ),
        vec!["Keep running".into(), "Pause them".into()],
        1,
        Duration::from_secs(120),
    );
    if matches!(answer, Ok(Some(0))) {
        with_store(|s| s.save().is_ok())
    } else {
        false
    }
}

/// Starts the scheduler thread (core start). `log`: the core's action log.
pub fn start(log: Arc<ActionLog>) {
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(log);
    let _ = std::thread::Builder::new().name("xtraos-routines".into()).spawn(|| loop {
        std::thread::sleep(TICK);
        let _ = std::panic::catch_unwind(|| run_due(local_now_ms()));
    });
}

// ---------------------------------------------------------------------------------------------
// Skills

/// Stores a routine the user approved (called by `routine.add` after the dialog, and by tests).
pub fn add_approved(r: Routine) -> Result<(), ActError> {
    with_store(|s| {
        if s.list.iter().any(|x| x.name == r.name) {
            return Err(bad(format!("a routine named '{}' exists; remove it first", r.name)));
        }
        if s.list.len() >= MAX_ROUTINES {
            return Err(bad(format!("at most {MAX_ROUTINES} routines")));
        }
        s.list.push(r);
        s.save()
    })
}

/// Re-reads routines.json (as at core start): a file changed outside XtraOS becomes untrusted.
pub fn reload() {
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn find(name: &str) -> Result<Routine, ActError> {
    let n = name.trim().to_lowercase();
    with_store(|s| s.list.iter().find(|r| r.name == n).cloned())
        .ok_or_else(|| bad(format!("no routine named '{n}' (see routine.list)")))
}

pub fn run(op: &str, a: &Value) -> Result<Value, ActError> {
    let name_arg = || a.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    match op {
        "routine.add" => {
            let client = crate::locks::current_client().unwrap_or_else(|| "unknown".into());
            let r = from_args(a, local_now_ms(), &client)?;
            if with_store(|s| s.list.iter().any(|x| x.name == r.name)) {
                return Err(bad(format!("a routine named '{}' exists; remove it first", r.name)));
            }
            let what = crate::skills::plan(&json!({"name": r.skill, "args": r.args}))?.what;
            crate::safety::activity("waiting for you: new routine");
            let answer = crate::safety::prompt(
                "XtraOS Agent — add a routine?",
                &format!(
                    "The AI agent \"{}\" wants to add a routine that runs on its own while XtraOS is running:\n\nName: {}\nDoes: {}\nWhen: {}\n\nEach run still follows your rules (and asks you where they say so). You can pause it from the tray icon → Agents, or ask an agent to remove it.",
                    crate::perceive::clip(&client, 40),
                    r.name,
                    crate::perceive::clip(&what, 300),
                    r.schedule.describe()
                ),
                vec!["Add routine".into(), "Cancel".into()],
                1,
                Duration::from_secs(120),
            )?;
            if answer != Some(0) {
                return Err(crate::safety::blocked("the user did not add the routine"));
            }
            let (name, when) = (r.name.clone(), r.schedule.describe());
            add_approved(r)?;
            Ok(json!({"text": format!("ok routine '{name}' added: {when}")}))
        }
        "routine.remove" => {
            let r = find(&name_arg())?;
            with_store(|s| {
                s.list.retain(|x| x.name != r.name);
                s.save()
            })?;
            Ok(json!({"text": format!("ok routine '{}' removed", r.name)}))
        }
        "routine.list" => {
            let (list, trusted) = with_store(|s| (s.list.clone(), s.trusted));
            let mut text = String::new();
            if !trusted {
                text.push_str("PAUSED: the routines file was changed outside XtraOS; the user is asked before they run\n");
            }
            for r in &list {
                text.push_str(&format!(
                    "{}: {} {} — last run {}\n",
                    r.name,
                    r.skill,
                    r.schedule.describe(),
                    r.last.map(show_time).unwrap_or_else(|| "never".into())
                ));
            }
            if list.is_empty() {
                text.push_str("no routines\n");
            }
            Ok(json!({"text": text.trim_end(), "routines": list.iter().map(Routine::to_json).collect::<Vec<_>>(), "trusted": trusted}))
        }
        "routine.run" => {
            let r = find(&name_arg())?;
            let v = execute(&r)?;
            Ok(json!({"text": format!("ok routine '{}' ran: {}", r.name, v["text"].as_str().unwrap_or("done")), "result": v}))
        }
        "routine.summary" => {
            let hours = a.get("hours").and_then(Value::as_u64).unwrap_or(24).clamp(1, 24 * 30) as i64;
            let runs: Vec<Value> = std::fs::read_to_string(runs_file())
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            let names: Vec<String> = with_store(|s| s.list.iter().map(|r| r.name.clone()).collect());
            let (text, per) = summarize(&runs, local_now_ms() - hours * 3_600_000, &names);
            Ok(json!({"text": format!("routines, last {hours} h:\n{text}"), "routines": per}))
        }
        o => Err(bad(format!("unknown routine op '{o}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn civil_round_trip_and_show() {
        let d = days_from_civil(2026, 11, 11);
        assert_eq!(civil_from_days(d), (2026, 11, 11));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(show_time(d * DAY_MS + 9 * 60 * MIN + 5 * MIN), "2026-11-11 09:05");
    }

    #[test]
    fn schedules_parse() {
        assert_eq!(Schedule::parse(&json!({"every_minutes": 30})).unwrap(), Schedule::Every(30));
        assert_eq!(Schedule::parse(&json!({"daily_at": "9:05"})).unwrap(), Schedule::Daily(9, 5));
        assert!(Schedule::parse(&json!({"every_minutes": 2})).is_err());
        assert!(Schedule::parse(&json!({"daily_at": "24:00"})).is_err());
        assert!(Schedule::parse(&json!({"every_minutes": 30, "daily_at": "09:00"})).is_err());
        assert!(Schedule::parse(&json!({})).is_err());
        assert_eq!(Schedule::Every(120).describe(), "every 2 hour(s)");
        assert_eq!(Schedule::Daily(7, 0).describe(), "every day at 07:00");
    }

    #[test]
    fn every_is_due_after_its_interval() {
        let s = Schedule::Every(30);
        assert!(!s.due(0, None, 29 * MIN));
        assert!(s.due(0, None, 30 * MIN));
        assert!(!s.due(0, Some(30 * MIN), 59 * MIN));
        assert!(s.due(0, Some(30 * MIN), 60 * MIN));
    }

    #[test]
    fn daily_runs_once_per_slot_and_catches_up_once() {
        let day = days_from_civil(2026, 11, 11) * DAY_MS;
        let s = Schedule::Daily(9, 0);
        let created = day + 8 * 60 * MIN; // 08:00
        assert!(!s.due(created, None, day + 8 * 60 * MIN + 59 * MIN));
        assert!(s.due(created, None, day + 9 * 60 * MIN));
        assert!(!s.due(created, Some(day + 9 * 60 * MIN), day + 15 * 60 * MIN), "once a day");
        // the core was off at 09:00: runs once at 11:00, then not again that day
        assert!(s.due(created, None, day + 11 * 60 * MIN));
        assert!(!s.due(created, Some(day + 11 * 60 * MIN), day + 12 * 60 * MIN));
        // off for three days: one catch-up run, not three
        assert!(s.due(created, Some(day + 9 * 60 * MIN), day + 3 * DAY_MS + 10 * 60 * MIN));
        assert!(!s.due(created, Some(day + 3 * DAY_MS + 10 * 60 * MIN), day + 3 * DAY_MS + 11 * 60 * MIN));
        // created after today's slot: first run tomorrow
        let late = day + 10 * 60 * MIN;
        assert!(!s.due(late, None, day + 23 * 60 * MIN));
        assert!(s.due(late, None, day + DAY_MS + 9 * 60 * MIN));
    }

    #[test]
    fn add_arguments_are_checked() {
        let ok = from_args(&json!({"name": "Disk-Report", "skill": "files.largest", "args": {"path": "C:/x"}, "daily_at": "08:30"}), 0, "c").unwrap();
        assert_eq!(ok.name, "disk-report");
        assert!(from_args(&json!({"name": "a b", "skill": "x", "every_minutes": 30}), 0, "c").is_err());
        assert!(from_args(&json!({"name": "loop", "skill": "routine.run", "every_minutes": 30}), 0, "c").is_err());
        assert!(from_args(&json!({"name": "x", "skill": "files.list", "args": [1], "every_minutes": 30}), 0, "c").is_err());
        let e = from_args(&json!({"name": "x", "skill": "whatsapp.send", "args": {"password": "p"}, "every_minutes": 30}), 0, "c").unwrap_err();
        assert_eq!(e.code, "BLOCKED_BY_POLICY");
        // round trip through the file format
        let back = Routine::from_json(&ok.to_json()).unwrap();
        assert_eq!(back, ok);
    }

    #[test]
    fn runs_are_trimmed_and_summarized() {
        let lines: Vec<String> = (0..2_005).map(|i| i.to_string()).collect();
        let t = trim_runs(lines);
        assert_eq!((t.len(), t[0].as_str()), (MAX_RUN_LINES, "5"));
        let runs = vec![
            json!({"name": "a", "at": 10, "ok": true, "text": "old"}),
            json!({"name": "a", "at": 100, "ok": true, "text": "fine"}),
            json!({"name": "a", "at": 200, "ok": false, "code": "STOPPED", "text": "x"}),
            json!({"name": "gone", "at": 150, "ok": true, "text": "y"}),
        ];
        let (text, per) = summarize(&runs, 50, &["a".into(), "idle".into()]);
        assert!(text.contains("a: 2 run(s), 1 failed; last"), "{text}");
        assert!(text.contains("STOPPED"), "{text}");
        assert!(text.contains("idle: 0 run(s), 0 failed"), "{text}");
        assert!(text.contains("gone: 1 run(s)"), "removed routines still show their runs: {text}");
        assert_eq!(per.as_array().unwrap().len(), 3);
    }
}
