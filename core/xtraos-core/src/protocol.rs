//! JSON-RPC 2.0 request handling for the core pipe (pure logic, no I/O → unit-testable).
//!
//! Wire format: one JSON object per line (UTF-8, '\n' terminated).
//! Errors use the shared string codes from docs/04-mcp-tool-contract.md:
//!   {"jsonrpc":"2.0","id":<id>,"error":{"code":"NOT_PAIRED","message":"...","hint":"..."}}
//!
//! Rules (docs/08-test-plan.md P1-IPC-*):
//! - The first request on a connection must be `pair` with the correct token. Anything else,
//!   or a wrong/missing token → NOT_PAIRED and the connection is closed.
//! - Malformed JSON / bad shape / unknown method → INVALID_ARGS (connection stays open if paired).
//! - Methods planned in the contract but not built yet → NOT_IMPLEMENTED (never fake success).

use serde_json::{json, Value};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Methods from docs/04-mcp-tool-contract.md that exist on paper but not yet in the core.
const PLANNED_METHODS: &[&str] = &["agent_workspace"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotPaired,
    InvalidArgs,
    NotImplemented,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NotPaired => "NOT_PAIRED",
            ErrorCode::InvalidArgs => "INVALID_ARGS",
            ErrorCode::NotImplemented => "NOT_IMPLEMENTED",
            ErrorCode::Internal => "INTERNAL",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub client: String,
    pub paired_at: Instant,
}

/// Per-connection state.
#[derive(Debug, Default)]
pub struct Conn {
    pub session: Option<Session>,
}

/// Read-only server facts the handler needs.
pub struct Ctx<'a> {
    pub token: &'a str,
    pub pipe: &'a str,
    pub started: Instant,
    pub next_session_id: &'a dyn Fn() -> u64,
}

/// What the I/O layer should do with the reply.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Reply(Value),
    ReplyAndClose(Value),
    /// An OS action: the I/O layer runs it (blocking thread) and builds the reply.
    Execute {
        id: Value,
        method: String,
        params: Value,
    },
}

/// Summary for the action log (never contains params or the token).
#[derive(Debug, Clone, PartialEq)]
pub struct LogInfo {
    pub method: String,
    pub ok: bool,
    pub code: Option<&'static str>,
}

pub fn error(id: &Value, code: ErrorCode, message: &str, hint: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code.as_str(),"message":message,"hint":hint}})
}

fn result(id: &Value, value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":value})
}

/// Constant-time comparison so token checks do not leak timing.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn sanitize_client(name: Option<&Value>) -> String {
    let raw = name.and_then(Value::as_str).unwrap_or("unknown");
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).take(64).collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

pub fn handle_line(conn: &mut Conn, line: &str, ctx: &Ctx) -> (Outcome, LogInfo) {
    let paired = conn.session.is_some();
    let invalid = |id: &Value, msg: &str, method: &str| {
        let reply = error(
            id,
            ErrorCode::InvalidArgs,
            msg,
            "Send one JSON-RPC 2.0 object per line.",
        );
        let info = LogInfo {
            method: method.into(),
            ok: false,
            code: Some("INVALID_ARGS"),
        };
        // An unpaired client that cannot even speak the protocol is disconnected.
        (
            if paired {
                Outcome::Reply(reply)
            } else {
                Outcome::ReplyAndClose(reply)
            },
            info,
        )
    };

    let req: Value = match serde_json::from_str(line.trim_end_matches(['\r', '\n'])) {
        Ok(v) => v,
        Err(_) => return invalid(&Value::Null, "malformed JSON", "<malformed>"),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    if !req.is_object() || req.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid(&id, "not a JSON-RPC 2.0 request", "<invalid>");
    }
    let method = match req.get("method").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return invalid(&id, "missing method", "<invalid>"),
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // ---- Not paired yet: only a correct `pair` is accepted. ----
    if !paired {
        let token_ok = method == "pair"
            && params
                .get("token")
                .and_then(Value::as_str)
                .is_some_and(|t| constant_time_eq(t, ctx.token));
        if !token_ok {
            let reply = error(
                &id,
                ErrorCode::NotPaired,
                "pair with the XtraOS pairing token first",
                "Start the XtraOS core and make sure the client reads the current pairing token.",
            );
            return (
                Outcome::ReplyAndClose(reply),
                LogInfo {
                    method,
                    ok: false,
                    code: Some("NOT_PAIRED"),
                },
            );
        }
        let session = Session {
            id: format!("s{}", (ctx.next_session_id)()),
            client: sanitize_client(params.get("client")),
            paired_at: Instant::now(),
        };
        let reply = result(
            &id,
            json!({"session": session.id, "client": session.client, "version": VERSION}),
        );
        conn.session = Some(session);
        return (
            Outcome::Reply(reply),
            LogInfo {
                method,
                ok: true,
                code: None,
            },
        );
    }

    // ---- Paired ----
    let session = conn.session.as_ref().expect("paired");
    let ok = |v: Value, method: String| {
        (
            Outcome::Reply(result(&id, v)),
            LogInfo {
                method,
                ok: true,
                code: None,
            },
        )
    };
    match method.as_str() {
        "ping" => ok(json!("pong"), method),
        "status" => ok(
            json!({
                "version": VERSION,
                "session": session.id,
                "client": session.client,
                "state": if crate::safety::stopped() { "stopped" } else { "running" },
                "uptime_ms": ctx.started.elapsed().as_millis() as u64,
                "pipe": ctx.pipe,
                "stopped": crate::safety::stopped(),
                "kill_hotkey": crate::safety::hotkey_name(),
                "admin_helper": crate::helper::status_json(),
                // P6-M4 (ADR 0047): an owned dialog of the foreground window, or null.
                "dialog": crate::winmgr::foreground_dialog(),
                // ADR 0050: CPU/RAM headline and active warnings.
                "health": crate::health::status_json(),
            }),
            method,
        ),
        "pair" => invalid(&id, "already paired", &method),
        m if crate::input::METHODS.contains(&m)
            || crate::winmgr::METHODS.contains(&m)
            || crate::perceive::METHODS.contains(&m)
            || crate::safety::METHODS.contains(&m)
            || matches!(
                m,
                "batch"
                    | "app_map"
                    | "list_skills"
                    | "run_skill"
                    | "elevated_helper"
                    | "browser"
                    | "record_session"
                    | "touch"
                    | "pen"
                    | "volume"
                    | "system"
                    | "sandbox"
                    | "audit_export"
            ) =>
        {
            let params = if params.is_null() { json!({}) } else { params };
            if !params.is_object() {
                return invalid(&id, "params must be an object", &method);
            }
            let info = LogInfo {
                method: method.clone(),
                ok: false,
                code: None,
            };
            (Outcome::Execute { id, method, params }, info)
        }
        m if PLANNED_METHODS.contains(&m) => {
            let reply = error(
                &id,
                ErrorCode::NotImplemented,
                &format!("'{m}' is planned but not built yet"),
                "See docs/07-development-plan.md for the milestone that adds it.",
            );
            (
                Outcome::Reply(reply),
                LogInfo {
                    method,
                    ok: false,
                    code: Some("NOT_IMPLEMENTED"),
                },
            )
        }
        _ => invalid(&id, "unknown method", &method),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(f: &'a dyn Fn() -> u64) -> Ctx<'a> {
        Ctx {
            token: "secret-token",
            pipe: r"\\.\pipe\test",
            started: Instant::now(),
            next_session_id: f,
        }
    }

    fn code(o: &Outcome) -> Option<String> {
        let v = match o {
            Outcome::Reply(v) | Outcome::ReplyAndClose(v) => v,
            Outcome::Execute { .. } => return None,
        };
        v.pointer("/error/code")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    #[test]
    fn pair_then_status() {
        let next = || 7;
        let c = ctx(&next);
        let mut conn = Conn::default();
        let (o, info) = handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":"secret-token","client":"test"}}"#,
            &c,
        );
        assert!(matches!(o, Outcome::Reply(_)) && info.ok);
        let (o, _) = handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":2,"method":"status"}"#,
            &c,
        );
        let Outcome::Reply(v) = o else { panic!() };
        assert_eq!(v.pointer("/result/session").unwrap(), "s7");
        assert_eq!(v.pointer("/result/client").unwrap(), "test");
        assert_eq!(v.pointer("/result/state").unwrap(), "running");
    }

    #[test]
    fn anything_before_pair_is_refused_and_closed() {
        let next = || 1;
        let c = ctx(&next);
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"status"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":"wrong"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair"}"#,
        ] {
            let mut conn = Conn::default();
            let (o, info) = handle_line(&mut conn, line, &c);
            assert!(matches!(o, Outcome::ReplyAndClose(_)), "{line}");
            assert_eq!(code(&o).as_deref(), Some("NOT_PAIRED"), "{line}");
            assert!(!info.ok && conn.session.is_none());
        }
    }

    #[test]
    fn malformed_before_pair_closes_after_pair_stays_open() {
        let next = || 1;
        let c = ctx(&next);
        let mut conn = Conn::default();
        let (o, _) = handle_line(&mut conn, "not json", &c);
        assert!(matches!(o, Outcome::ReplyAndClose(_)));
        assert_eq!(code(&o).as_deref(), Some("INVALID_ARGS"));

        let mut conn = Conn::default();
        handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":"secret-token"}}"#,
            &c,
        );
        for line in [
            "not json",
            r#"[1,2]"#,
            r#"{"jsonrpc":"1.0","id":3,"method":"status"}"#,
            r#"{"jsonrpc":"2.0","id":4}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"pair","params":{"token":"secret-token"}}"#,
        ] {
            let (o, _) = handle_line(&mut conn, line, &c);
            assert!(matches!(o, Outcome::Reply(_)), "{line}");
            assert_eq!(code(&o).as_deref(), Some("INVALID_ARGS"), "{line}");
        }
    }

    #[test]
    fn planned_methods_are_not_implemented_never_fake_success() {
        let next = || 1;
        let c = ctx(&next);
        let mut conn = Conn::default();
        handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":"secret-token"}}"#,
            &c,
        );
        let (o, info) = handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":2,"method":"agent_workspace","params":{}}"#,
            &c,
        );
        assert_eq!(code(&o).as_deref(), Some("NOT_IMPLEMENTED"));
        assert_eq!(info.code, Some("NOT_IMPLEMENTED"));
    }

    #[test]
    fn client_name_is_sanitised() {
        assert_eq!(sanitize_client(Some(&json!("a\u{0007}b"))), "ab");
        assert_eq!(sanitize_client(Some(&json!("x".repeat(200)))).len(), 64);
        assert_eq!(sanitize_client(None), "unknown");
        assert_eq!(sanitize_client(Some(&json!(""))), "unknown");
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }
}

#[cfg(test)]
mod execute_tests {
    use super::*;

    #[test]
    fn input_methods_are_dispatched_for_execution_only_when_paired() {
        let next = || 1;
        let c = Ctx {
            token: "t",
            pipe: "p",
            started: Instant::now(),
            next_session_id: &next,
        };
        let mut conn = Conn::default();
        let (o, _) = handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"click","params":{"x":1,"y":2}}"#,
            &c,
        );
        assert!(
            matches!(o, Outcome::ReplyAndClose(_)),
            "unpaired click must be refused"
        );

        handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":"t"}}"#,
            &c,
        );
        let (o, info) = handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":9,"method":"click","params":{"x":1,"y":2}}"#,
            &c,
        );
        match o {
            Outcome::Execute { id, method, params } => {
                assert_eq!(id, json!(9));
                assert_eq!(method, "click");
                assert_eq!(params, json!({"x":1,"y":2}));
            }
            other => panic!("expected Execute, got {other:?}"),
        }
        assert_eq!(info.method, "click");

        let (o, _) = handle_line(
            &mut conn,
            r#"{"jsonrpc":"2.0","id":10,"method":"type","params":[1]}"#,
            &c,
        );
        assert!(
            matches!(o, Outcome::Reply(_)),
            "non-object params are INVALID_ARGS"
        );
    }
}
