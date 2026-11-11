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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schedule {
    Every(u32),
    Daily(u32, u32),
    /// ADR 0064: a new file in this folder (not subfolders) matching the pattern.
    NewFile { folder: String, pattern: String },
    /// ADR 0066: once at this local time (ms), then the routine is removed.
    Once(i64),
}

/// "HH:MM" (today, or tomorrow when passed) or "YYYY-MM-DD HH:MM" → local ms (pure).
pub fn parse_local(s: &str, now: i64) -> Option<i64> {
    let s = s.trim();
    let hm = |t: &str| -> Option<i64> {
        let (h, m) = t.trim().split_once(':')?;
        let (h, m) = (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?);
        (h < 24 && m < 60 && h >= 0 && m >= 0).then_some((h * 60 + m) * 60_000)
    };
    match s.split_once(' ') {
        Some((date, time)) => {
            let mut p = date.split('-');
            let (y, mo, d) = (p.next()?.parse::<i64>().ok()?, p.next()?.parse::<i64>().ok()?, p.next()?.parse::<i64>().ok()?);
            if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || p.next().is_some() {
                return None;
            }
            let day = days_from_civil(y, mo, d);
            if civil_from_days(day) != (y, mo, d) {
                return None; // e.g. 2026-02-30
            }
            Some(day * DAY_MS + hm(time)?)
        }
        None => {
            let t = now.div_euclid(DAY_MS) * DAY_MS + hm(s)?;
            Some(if t <= now { t + DAY_MS } else { t })
        }
    }
}

impl Schedule {
    pub fn parse(a: &Value) -> Result<Schedule, ActError> {
        Self::parse_at(a, local_now_ms(), true)
    }

    /// `strict`: a new routine (a past or far `once_at` is refused); false when reading the
    /// stored file (a reminder missed while XtraOS was off still runs once).
    pub fn parse_at(a: &Value, now: i64, strict: bool) -> Result<Schedule, ActError> {
        let every = a.get("every_minutes").filter(|v| !v.is_null());
        let daily = a.get("daily_at").filter(|v| !v.is_null());
        let newf = a.get("new_file_in").filter(|v| !v.is_null());
        let once = a.get("once_at").filter(|v| !v.is_null());
        let kinds = [every.is_some(), daily.is_some(), newf.is_some(), once.is_some()].iter().filter(|k| **k).count();
        if kinds > 1 {
            return Err(bad("give exactly one of 'every_minutes', 'daily_at', 'new_file_in' or 'once_at'"));
        }
        if let Some(o) = once {
            let t = o
                .as_str()
                .and_then(|s| parse_local(s, now))
                .ok_or_else(|| bad("'once_at' must be \"HH:MM\" or \"YYYY-MM-DD HH:MM\""))?;
            if strict && t <= now {
                return Err(bad("'once_at' is in the past"));
            }
            if strict && t - now > 366 * DAY_MS {
                return Err(bad("'once_at' must be within a year"));
            }
            return Ok(Schedule::Once(t));
        }
        if let Some(f) = newf {
            let folder = f.as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("'new_file_in' must be a folder path"))?;
            let pattern = a.get("pattern").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).unwrap_or("*");
            if pattern.len() > 100 || pattern.contains(['/', '\\']) {
                return Err(bad("'pattern' is a file name pattern like *.pdf"));
            }
            return Ok(Schedule::NewFile { folder: folder.trim().to_string(), pattern: pattern.trim().to_string() });
        }
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
            _ => Err(bad("give exactly one of 'every_minutes', 'daily_at', 'new_file_in' or 'once_at'")),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Schedule::Every(n) if n % 60 == 0 => format!("every {} hour(s)", n / 60),
            Schedule::Every(n) => format!("every {n} minutes"),
            Schedule::Daily(h, m) => format!("every day at {h:02}:{m:02}"),
            Schedule::NewFile { folder, pattern } => format!("for each new {pattern} file in {folder}"),
            Schedule::Once(t) => format!("once at {}", show_time(*t)),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Schedule::Every(n) => json!({"every_minutes": n}),
            Schedule::Daily(h, m) => json!({"daily_at": format!("{h:02}:{m:02}")}),
            Schedule::NewFile { folder, pattern } => json!({"new_file_in": folder, "pattern": pattern}),
            Schedule::Once(t) => json!({"once_at": show_time(*t)}),
        }
    }

    /// Whether a timed run is due at `now` (local ms). A missed slot runs once, never more.
    /// A new-file routine is never "due": the tick looks for files instead.
    pub fn due(&self, created: i64, last: Option<i64>, now: i64) -> bool {
        let since = last.unwrap_or(created).max(created);
        match *self {
            Schedule::Every(n) => now - since >= n as i64 * 60_000,
            Schedule::Daily(h, m) => {
                let today = now.div_euclid(DAY_MS) * DAY_MS + (h as i64 * 60 + m as i64) * 60_000;
                let slot = if now >= today { today } else { today - DAY_MS };
                since < slot && now >= slot
            }
            Schedule::NewFile { .. } => false,
            // Late (the core was off) still runs, once.
            Schedule::Once(t) => last.is_none() && now >= t,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// New-file trigger and notifications (ADR 0064; pure parts unit-tested)

const SETTLE_MS: i64 = 10_000;
const MAX_FILES_PER_TICK: usize = 10;

/// UTC milliseconds now (file times are UTC).
pub fn utc_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Files to process (pure): newer than `cursor`, settled for 10 s, finished, matching; oldest
/// first, at most 10. `entries`: (path, file name, time in UTC ms).
pub fn pick_new(entries: &[(String, String, i64)], cursor: i64, now: i64, pattern: &str) -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = entries
        .iter()
        .filter(|(_, n, t)| {
            *t > cursor && now - *t >= SETTLE_MS && !crate::skills::unfinished(n) && crate::skills::wildcard(pattern, n)
        })
        .map(|(p, _, t)| (p.clone(), *t))
        .collect();
    v.sort_by_key(|(_, t)| *t);
    v.truncate(MAX_FILES_PER_TICK);
    v
}

/// The folder's files with their time: the later of created and changed (a copied file keeps
/// its old "changed" time but gets a new "created" time).
fn folder_files(folder: &Path) -> Vec<(String, String, i64)> {
    let ms = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    };
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            if !m.is_file() {
                return None;
            }
            let t = ms(m.created()).max(ms(m.modified()));
            Some((e.path().display().to_string(), e.file_name().to_string_lossy().into_owned(), t))
        })
        .collect()
}

/// `{file}` → the path, `{name}` → the file name, in every string of the arguments (pure).
pub fn with_file(args: &Value, path: &str) -> Value {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match args {
        Value::String(s) => Value::String(s.replace("{file}", path).replace("{name}", &name)),
        Value::Array(a) => Value::Array(a.iter().map(|x| with_file(x, path)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), with_file(x, path))).collect()),
        other => other.clone(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    Never,
    Failure,
    Always,
}

impl Notify {
    pub fn parse(a: &Value) -> Result<Notify, ActError> {
        match a.get("notify").and_then(Value::as_str).unwrap_or("failure") {
            "never" => Ok(Notify::Never),
            "failure" => Ok(Notify::Failure),
            "always" => Ok(Notify::Always),
            o => Err(bad(format!("'notify' must be never, failure or always, not '{o}'"))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Notify::Never => "never",
            Notify::Failure => "failure",
            Notify::Always => "always",
        }
    }

    /// Whether to show a notification after a run (pure). `failure`: only when the routine
    /// starts failing or works again.
    pub fn should(self, prev_ok: Option<bool>, ok: bool) -> bool {
        match self {
            Notify::Never => false,
            Notify::Always => true,
            Notify::Failure => (!ok && prev_ok != Some(false)) || (ok && prev_ok == Some(false)),
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
    /// New-file trigger: files up to this time (UTC ms) are done.
    pub cursor: Option<i64>,
    pub notify: Notify,
    pub last_ok: Option<bool>,
    /// ADR 0068: kept but not run until resumed.
    pub paused: bool,
}

impl Routine {
    fn to_json(&self) -> Value {
        let mut v = json!({
            "name": self.name, "skill": self.skill, "args": self.args,
            "created": self.created, "last_run": self.last, "added_by": self.added_by,
            "cursor": self.cursor, "notify": self.notify.name(), "last_ok": self.last_ok,
            "paused": self.paused,
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
            schedule: Schedule::parse_at(v, local_now_ms(), false).ok()?,
            created: v["created"].as_i64()?,
            last: v["last_run"].as_i64(),
            added_by: v["added_by"].as_str().unwrap_or("").to_string(),
            cursor: v["cursor"].as_i64(),
            notify: Notify::parse(v).unwrap_or(Notify::Failure),
            last_ok: v["last_ok"].as_bool(),
            paused: v["paused"].as_bool().unwrap_or(false),
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
    let schedule = Schedule::parse(a)?;
    let cursor = match &schedule {
        Schedule::NewFile { folder, .. } => {
            // Protected places are refused; files already there are never processed.
            let f = crate::skills::checked(folder)?;
            if !f.is_dir() {
                return Err(bad(format!("{} is not a folder", f.display())));
            }
            Some(utc_now_ms())
        }
        _ => None,
    };
    Ok(Routine {
        name,
        skill,
        args,
        schedule,
        created,
        last: None,
        added_by: added_by.into(),
        cursor,
        notify: Notify::parse(a)?,
        last_ok: None,
        paused: false,
    })
}

/// The arguments a check (or the dialog) sees: for a new-file routine, `{file}` as an example
/// file in its folder (pure).
pub fn sample_args(r: &Routine) -> Value {
    match &r.schedule {
        Schedule::NewFile { folder, .. } => with_file(&r.args, &Path::new(folder).join("example-new-file").display().to_string()),
        _ => r.args.clone(),
    }
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

/// The ROUTINES section of the viewer window (ADR 0064): read straight from the files, so the
/// viewer (its own process) needs no core.
pub fn overview(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("routines.json")).unwrap_or_default();
    let list: Vec<Routine> = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Routine::from_json)
        .collect();
    let mut s = String::from("ROUTINES (run on their own, by your rules)\r\n");
    if list.is_empty() {
        s.push_str("  (none)\r\n");
    }
    let ok = std::fs::read_to_string(dir.join("routines.ok")).unwrap_or_default();
    if !text.is_empty() && ok.trim() != crate::actionlog::sha256_hex(text.as_bytes()) {
        s.push_str("  PAUSED: the routines file was changed outside XtraOS; you will be asked\r\n");
    }
    for r in &list {
        let last = match (r.last, r.last_ok) {
            (Some(t), Some(true)) => format!("last {} ok", show_time(t)),
            (Some(t), Some(false)) => format!("last {} FAILED", show_time(t)),
            (Some(t), None) => format!("last {}", show_time(t)),
            _ => "not run yet".into(),
        };
        s.push_str(&format!(
            "  {:<20} {} — {}; {last}{}\r\n",
            r.name,
            r.skill,
            r.schedule.describe(),
            if r.paused { " (PAUSED)" } else { "" }
        ));
    }
    s.push_str("\r\n");
    s
}

// ---------------------------------------------------------------------------------------------
// Running

static LOG: Mutex<Option<Arc<ActionLog>>> = Mutex::new(None);

/// One run of a routine as its own agent (client `routine:<name>`), through authorize.
pub fn execute(r: &Routine) -> Result<Value, ActError> {
    execute_with(r, &r.args)
}

/// A run with these arguments (a new-file run: `{file}` already substituted).
fn execute_with(r: &Routine, args: &Value) -> Result<Value, ActError> {
    let session = format!("routine-{}", r.name);
    let client = format!("routine:{}", r.name);
    crate::agents::connected(&session, &client);
    let caller = crate::locks::replace_current(Some((session.clone(), client.clone())));
    let p = json!({"name": r.skill, "args": args});
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
    let prev_ok = with_store(|s| {
        let x = s.list.iter_mut().find(|x| x.name == r.name)?;
        let prev = x.last_ok;
        x.last = Some(now);
        x.last_ok = Some(res.is_ok());
        let _ = s.save();
        Some(prev)
    })
    .flatten();
    // ADR 0064: a Windows notification from the tray icon.
    if r.notify.should(prev_ok, res.is_ok()) {
        let title = format!("XtraOS routine '{}' {}", r.name, if res.is_ok() { "ran" } else { "failed" });
        crate::safety::notify(&title, &crate::perceive::clip(&text, 200), res.is_err());
    }
    res
}

/// One scheduler tick: run every due routine (in order), and each new-file routine for the new
/// files in its folder. Returns the names run (one entry per run).
pub fn run_due(now: i64) -> Vec<String> {
    if crate::safety::stopped() {
        return Vec::new();
    }
    if !with_store(|s| s.trusted) && !approve_changed_file() {
        return Vec::new();
    }
    let list: Vec<Routine> = with_store(|s| s.list.clone());
    let mut ran = Vec::new();
    for r in &list {
        if r.paused {
            continue; // ADR 0068
        }
        if let Schedule::NewFile { folder, pattern } = &r.schedule {
            let files = pick_new(&folder_files(Path::new(folder)), r.cursor.unwrap_or(i64::MAX), utc_now_ms(), pattern);
            for (path, t) in files {
                if crate::safety::stopped() {
                    break;
                }
                // The cursor moves first: a file that fails is not retried every tick.
                with_store(|s| {
                    if let Some(x) = s.list.iter_mut().find(|x| x.name == r.name) {
                        x.cursor = Some(x.cursor.unwrap_or(0).max(t));
                        let _ = s.save();
                    }
                });
                let _ = execute_with(r, &with_file(&r.args, &path));
                ran.push(r.name.clone());
            }
        } else if r.schedule.due(r.created, r.last, now) {
            let _ = execute(r);
            ran.push(r.name.clone());
            // ADR 0066: a one-time routine is done (its run stays in the summary).
            if matches!(r.schedule, Schedule::Once(_)) {
                with_store(|s| {
                    s.list.retain(|x| x.name != r.name);
                    let _ = s.save();
                });
            }
        }
    }
    ran
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
            let what = crate::skills::plan(&json!({"name": r.skill, "args": sample_args(&r)}))?.what;
            crate::safety::activity("waiting for you: new routine");
            let answer = crate::safety::prompt(
                "XtraOS Agent — add a routine?",
                &format!(
                    "The AI agent \"{}\" wants to add a routine that runs on its own while XtraOS is running:\n\nName: {}\nDoes: {}\nWhen: {}\nNotifies you: {}\n\nEach run still follows your rules (and asks you where they say so). You can pause it from the tray icon → Agents, or ask an agent to remove it.",
                    crate::perceive::clip(&client, 40),
                    r.name,
                    crate::perceive::clip(&what, 300),
                    r.schedule.describe(),
                    match r.notify {
                        Notify::Never => "never",
                        Notify::Failure => "when it starts failing or works again",
                        Notify::Always => "after every run",
                    }
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
        op @ ("routine.pause" | "routine.resume") => {
            let r = find(&name_arg())?;
            let pause = op == "routine.pause";
            with_store(|s| {
                if let Some(x) = s.list.iter_mut().find(|x| x.name == r.name) {
                    x.paused = pause;
                    // Resumed: files that arrived while paused are skipped, a timed routine
                    // does not catch up all the runs it missed.
                    if !pause {
                        if matches!(x.schedule, Schedule::NewFile { .. }) {
                            x.cursor = Some(utc_now_ms());
                        } else if !matches!(x.schedule, Schedule::Once(_)) {
                            x.last = Some(local_now_ms());
                        }
                    }
                }
                s.save()
            })?;
            Ok(json!({"text": format!("ok routine '{}' {}", r.name, if pause { "paused (routine.resume to run it again)" } else { "resumed" })}))
        }
        "routine.list" => {
            let (list, trusted) = with_store(|s| (s.list.clone(), s.trusted));
            let mut text = String::new();
            if !trusted {
                text.push_str("PAUSED: the routines file was changed outside XtraOS; the user is asked before they run\n");
            }
            for r in &list {
                text.push_str(&format!(
                    "{}: {} {} — last run {}{}\n",
                    r.name,
                    r.skill,
                    r.schedule.describe(),
                    r.last.map(show_time).unwrap_or_else(|| "never".into()),
                    if r.paused { " (PAUSED)" } else { "" }
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
    fn new_file_trigger_parses_and_validates() {
        let s = Schedule::parse(&json!({"new_file_in": "C:/In", "pattern": "*.pdf"})).unwrap();
        assert_eq!(s, Schedule::NewFile { folder: "C:/In".into(), pattern: "*.pdf".into() });
        assert_eq!(s.describe(), "for each new *.pdf file in C:/In");
        assert_eq!(Schedule::parse(&json!({"new_file_in": "C:/In"})).unwrap(), Schedule::NewFile { folder: "C:/In".into(), pattern: "*".into() });
        assert!(Schedule::parse(&json!({"new_file_in": "C:/In", "every_minutes": 10})).is_err());
        assert!(Schedule::parse(&json!({"new_file_in": "C:/In", "pattern": "a/b"})).is_err());
        assert!(!s.due(0, None, i64::MAX / 2), "never due by time");
        let tmp = std::env::temp_dir();
        let r = from_args(&json!({"name": "nf", "skill": "office.export_pdf", "args": {"path": "{file}"}, "new_file_in": tmp.display().to_string(), "pattern": "*.docx"}), 0, "c").unwrap();
        assert!(r.cursor.is_some(), "starts now: files already there are skipped");
        assert_eq!(sample_args(&r)["path"], tmp.join("example-new-file").display().to_string());
        assert!(from_args(&json!({"name": "nf", "skill": "files.list", "new_file_in": tmp.join("no-such-dir-xyz").display().to_string()}), 0, "c").is_err());
        assert!(from_args(&json!({"name": "nf", "skill": "files.list", "new_file_in": "C:/Windows/System32"}), 0, "c").is_err(), "protected");
        let back = Routine::from_json(&r.to_json()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn new_files_are_picked_once_settled_and_in_order() {
        let e = vec![
            ("C:/in/old.pdf".to_string(), "old.pdf".to_string(), 1_000),
            ("C:/in/b.pdf".into(), "b.pdf".into(), 30_000),
            ("C:/in/a.pdf".into(), "a.pdf".into(), 20_000),
            ("C:/in/fresh.pdf".into(), "fresh.pdf".into(), 55_000),
            ("C:/in/dl.pdf.crdownload".into(), "dl.pdf.crdownload".into(), 21_000),
            ("C:/in/note.txt".into(), "note.txt".into(), 22_000),
        ];
        let got = pick_new(&e, 5_000, 60_000, "*.pdf");
        assert_eq!(got, vec![("C:/in/a.pdf".to_string(), 20_000), ("C:/in/b.pdf".to_string(), 30_000)]);
        let many: Vec<(String, String, i64)> = (0..15).map(|i| (format!("f{i}"), format!("f{i}"), 100 + i)).collect();
        assert_eq!(pick_new(&many, 0, 100_000, "*").len(), MAX_FILES_PER_TICK);
    }

    #[test]
    fn file_placeholders_and_notifications() {
        let a = json!({"path": "{file}", "out": "C:/out/{name}.pdf", "n": 3, "list": ["{name}"]});
        let v = with_file(&a, "C:/in/report.docx");
        assert_eq!(v, json!({"path": "C:/in/report.docx", "out": "C:/out/report.docx.pdf", "n": 3, "list": ["report.docx"]}));
        assert!(Notify::parse(&json!({"notify": "sometimes"})).is_err());
        assert_eq!(Notify::parse(&json!({})).unwrap(), Notify::Failure);
        let f = Notify::Failure;
        assert!(f.should(None, false), "first failure");
        assert!(!f.should(Some(false), false), "no repeat while failing");
        assert!(f.should(Some(false), true), "works again");
        assert!(!f.should(Some(true), true));
        assert!(Notify::Always.should(Some(true), true));
        assert!(!Notify::Never.should(None, false));
    }

    #[test]
    fn viewer_overview_lists_routines() {
        let d = std::env::temp_dir().join(format!("xtraos-rt-ov-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut r = from_args(&json!({"name": "disk", "skill": "files.largest", "args": {"path": "C:/x"}, "daily_at": "08:30"}), 0, "c").unwrap();
        r.last = Some(days_from_civil(2026, 11, 11) * DAY_MS + 8 * 3_600_000 + 30 * 60_000);
        r.last_ok = Some(false);
        let text = serde_json::to_string(&json!([r.to_json()])).unwrap();
        std::fs::write(d.join("routines.json"), &text).unwrap();
        std::fs::write(d.join("routines.ok"), crate::actionlog::sha256_hex(text.as_bytes())).unwrap();
        let s = overview(&d);
        assert!(s.contains("disk") && s.contains("every day at 08:30") && s.contains("last 2026-11-11 08:30 FAILED"), "{s}");
        assert!(!s.contains("PAUSED"), "{s}");
        std::fs::write(d.join("routines.ok"), "x").unwrap();
        assert!(overview(&d).contains("PAUSED"));
        assert!(overview(&d.join("none")).contains("(none)"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn once_at_parses_and_is_due_once() {
        let day = days_from_civil(2026, 11, 11) * DAY_MS;
        let now = day + 15 * 60 * MIN; // 15:00
        assert_eq!(parse_local("17:00", now), Some(day + 17 * 60 * MIN), "later today");
        assert_eq!(parse_local("09:30", now), Some(day + DAY_MS + 9 * 60 * MIN + 30 * MIN), "passed: tomorrow");
        assert_eq!(parse_local("2026-11-12 08:05", now), Some(day + DAY_MS + 8 * 60 * MIN + 5 * MIN));
        assert_eq!(parse_local("2026-02-30 08:00", now), None);
        assert_eq!(parse_local("25:00", now), None);
        assert_eq!(parse_local("tomorrow", now), None);
        let s = Schedule::parse_at(&json!({"once_at": "17:00"}), now, true).unwrap();
        assert_eq!(s, Schedule::Once(day + 17 * 60 * MIN));
        assert_eq!(s.describe(), "once at 2026-11-11 17:00");
        assert!(Schedule::parse_at(&json!({"once_at": "2026-11-10 09:00"}), now, true).is_err(), "past");
        assert!(Schedule::parse_at(&json!({"once_at": "2028-01-01 09:00"}), now, true).is_err(), "> 1 year");
        assert!(Schedule::parse_at(&json!({"once_at": "2026-11-10 09:00"}), now, false).is_ok(), "stored: kept");
        assert!(Schedule::parse_at(&json!({"once_at": "17:00", "daily_at": "09:00"}), now, true).is_err());
        assert!(!s.due(now, None, day + 16 * 60 * MIN));
        assert!(s.due(now, None, day + 17 * 60 * MIN));
        assert!(s.due(now, None, day + 2 * DAY_MS), "late still runs");
        assert!(!s.due(now, Some(day + 17 * 60 * MIN), day + 18 * 60 * MIN), "once");
        // stored as an absolute time, read back the same
        let back = Schedule::parse_at(&s.to_json(), now + 3 * DAY_MS, false).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn paused_state_round_trips() {
        let mut r = from_args(&json!({"name": "p", "skill": "files.list", "args": {"path": "C:/x"}, "every_minutes": 30}), 0, "c").unwrap();
        assert!(!r.paused);
        r.paused = true;
        let back = Routine::from_json(&r.to_json()).unwrap();
        assert!(back.paused);
        // older files without the field: not paused
        let mut v = r.to_json();
        v.as_object_mut().unwrap().remove("paused");
        assert!(!Routine::from_json(&v).unwrap().paused);
    }

    #[test]
    fn notify_arguments() {
        let (t, x) = crate::skills::notify_args(&json!({"text": " Call the bank "})).unwrap();
        assert_eq!((t.as_str(), x.as_str()), ("XtraOS reminder", "Call the bank"));
        assert!(crate::skills::notify_args(&json!({})).is_err());
        assert!(crate::skills::notify_args(&json!({"text": "x".repeat(251)})).is_err());
        assert!(crate::skills::notify_args(&json!({"text": "x", "title": "t".repeat(61)})).is_err());
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
