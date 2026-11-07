//! `audit_export` (ADR 0042): the action log for a date range as CSV or JSON, with the chain
//! check and the head hash. Keeping the head hash off this PC makes a later rewrite of the
//! whole (hash-chained) log detectable (ADR 0038). The log holds no request parameters, so
//! the export holds no secrets.
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "audit_export {from?: \"YYYY-MM-DD\", to?: \"YYYY-MM-DD\", format?: csv|json, out?: path}",
    }
}

/// Days since 1970-01-01 → (year, month, day) (proleptic Gregorian).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Unix ms → "2026-11-07T16:51:00.123Z".
pub fn utc_iso(ms: i64) -> String {
    let (secs, milli) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, mo, d) = civil(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Local date "YYYY-MM-DD" of a unix-ms timestamp (Windows time zone rules).
fn local_date(ms: i64) -> String {
    #[repr(C)]
    #[derive(Default)]
    struct St {
        y: u16,
        mo: u16,
        dow: u16,
        d: u16,
        h: u16,
        mi: u16,
        s: u16,
        ms: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn FileTimeToLocalFileTime(ft: *const u64, local: *mut u64) -> i32;
        fn FileTimeToSystemTime(ft: *const u64, st: *mut St) -> i32;
    }
    let ft = (ms.max(0) as u64 + 11_644_473_600_000) * 10_000;
    let (mut local, mut st) = (0u64, St::default());
    unsafe {
        if FileTimeToLocalFileTime(&ft, &mut local) == 0
            || FileTimeToSystemTime(&local, &mut st) == 0
        {
            return utc_iso(ms)[..10].to_string();
        }
    }
    format!("{:04}-{:02}-{:02}", st.y, st.mo, st.d)
}

fn date_arg(p: &Value, k: &str) -> Result<Option<String>, ActError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let s = v.as_str().unwrap_or("");
            let ok = s.len() == 10
                && s.chars().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                });
            if !ok {
                return Err(bad(format!("'{k}' must look like 2026-11-07")));
            }
            Ok(Some(s.to_string()))
        }
    }
}

/// One CSV field.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

const COLUMNS: [&str; 11] = [
    "seq", "time_utc", "session", "client", "method", "ok", "code", "ms", "policy", "hash", "prev",
];

fn row(e: &Value) -> Vec<String> {
    let s = |k: &str| match &e[k] {
        Value::Null => String::new(),
        Value::String(x) => x.clone(),
        v => v.to_string(),
    };
    vec![
        s("seq"),
        utc_iso(e["ts_ms"].as_i64().unwrap_or(0)),
        s("session"),
        s("client"),
        s("method"),
        s("ok"),
        s("code"),
        s("ms"),
        s("policy"),
        s("hash"),
        s("prev"),
    ]
}

fn documents() -> PathBuf {
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) {
            let s = p.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
            if !s.is_empty() {
                return PathBuf::from(s);
            }
        }
    }
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into())).join("Documents")
}

/// The `audit_export` method.
pub fn run(p: &Value) -> Result<Value, ActError> {
    let from = date_arg(p, "from")?;
    let to = date_arg(p, "to")?;
    if let (Some(f), Some(t)) = (&from, &to) {
        if f > t {
            return Err(bad("'from' is after 'to'"));
        }
    }
    let format = p.get("format").and_then(Value::as_str).unwrap_or("csv");
    if !matches!(format, "csv" | "json") {
        return Err(bad("'format' is csv or json"));
    }
    let dir = crate::pairing::data_dir().join("logs");
    let mut all = Vec::new();
    for f in crate::actionlog::log_files(&dir) {
        if let Ok(t) = std::fs::read_to_string(&f) {
            all.extend(
                t.lines()
                    .filter_map(|l| serde_json::from_str::<Value>(l).ok()),
            );
        }
    }
    let head = all
        .iter()
        .rev()
        .find_map(|e| e["hash"].as_str().map(String::from));
    let picked: Vec<&Value> = all
        .iter()
        .filter(|e| {
            let d = local_date(e["ts_ms"].as_i64().unwrap_or(0));
            from.as_ref().is_none_or(|f| d >= *f) && to.as_ref().is_none_or(|t| d <= *t)
        })
        .collect();
    let chain = crate::actionlog::verify_dir(&dir);
    let out = match p.get("out").and_then(Value::as_str) {
        Some(o) => crate::skills::checked(o)?,
        None => documents().join(format!(
            "xtraos-audit-{}_{}.{format}",
            from.as_deref().unwrap_or("start"),
            to.as_deref().unwrap_or("now")
        )),
    };
    if out.exists() {
        return Err(bad(format!(
            "{} already exists: choose another 'out' (never overwritten)",
            out.display()
        )));
    }
    let body = if format == "csv" {
        let mut s = COLUMNS.join(",");
        s.push_str("\r\n");
        for e in &picked {
            s.push_str(
                &row(e)
                    .iter()
                    .map(|f| csv_field(f))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            s.push_str("\r\n");
        }
        s
    } else {
        serde_json::to_string_pretty(&json!({
            "exported_at": utc_iso(crate::protocol::now_ms() as i64),
            "from": from, "to": to,
            "chain": {"ok": chain.ok, "message": chain.message},
            "head_hash": head,
            "columns": COLUMNS,
            "entries": picked.iter().map(|e| row(e)).collect::<Vec<_>>(),
        }))
        .unwrap_or_default()
    };
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&out, body)
        .map_err(|e| bad(format!("could not write {}: {e}", out.display())))?;
    Ok(json!({
        "text": format!(
            "exported {} entries to {}; log chain {}; head hash {} (keep it somewhere else to detect a rewrite later)",
            picked.len(), out.display(),
            if chain.ok { "OK".to_string() } else { format!("BROKEN: {}", chain.message) },
            head.as_deref().unwrap_or("-")
        ),
        "path": out.display().to_string(), "entries": picked.len(),
        "chain_ok": chain.ok, "head_hash": head,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates_and_csv() {
        assert_eq!(utc_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc_iso(1_794_063_058_485), "2026-11-07T14:50:58.485Z");
        assert_eq!(
            utc_iso(951_782_400_000),
            "2000-02-29T00:00:00.000Z",
            "leap day"
        );
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert!(date_arg(&json!({"from": "2026-11-07"}), "from")
            .unwrap()
            .is_some());
        assert!(date_arg(&json!({"from": "07-11-2026"}), "from").is_err());
        assert!(run(&json!({"from": "2026-12-01", "to": "2026-11-01"})).is_err());
        assert!(run(&json!({"format": "xml"})).is_err());
    }
}
