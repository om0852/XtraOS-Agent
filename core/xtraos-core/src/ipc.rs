//! Named-pipe server (core ↔ MCP). Security (ADR 0004, docs/05-security-model.md):
//! - DACL: protected, NETWORK denied, only the current user allowed (winsec::pipe_sddl);
//! - PIPE_REJECT_REMOTE_CLIENTS (tokio `reject_remote_clients`);
//! - FILE_FLAG_FIRST_PIPE_INSTANCE (tokio `first_pipe_instance`) so another process that
//!   pre-created the name (pipe squatting) makes the core refuse to start;
//! - every connection must `pair` with the token first (protocol.rs).
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::actionlog::{ActionLog, Entry};
use crate::protocol::{self, Conn, Ctx, ErrorCode, Outcome};
use crate::winsec::{self, OwnedSecurityAttributes};

/// Longest accepted request line (bytes). Larger → INVALID_ARGS and disconnect.
pub const MAX_LINE: usize = 1024 * 1024;

pub struct ServerConfig {
    pub pipe_name: String,
    pub token: String,
    pub log: Arc<ActionLog>,
}

struct Shared {
    pipe_name: String,
    token: String,
    log: Arc<ActionLog>,
    started: Instant,
    sessions: AtomicU64,
}

pub struct ServerHandle {
    shutdown: Arc<Notify>,
    task: JoinHandle<io::Result<()>>,
}

impl ServerHandle {
    pub async fn shutdown(self) -> io::Result<()> {
        self.shutdown.notify_one();
        self.task.await.map_err(io::Error::other)?
    }
}

fn create_instance(
    name: &str,
    sa: &mut OwnedSecurityAttributes,
    first: bool,
) -> io::Result<NamedPipeServer> {
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first)
        .reject_remote_clients(true)
        .pipe_mode(PipeMode::Byte);
    // SAFETY: `sa` owns a valid SECURITY_ATTRIBUTES whose descriptor outlives this call.
    unsafe { opts.create_with_security_attributes_raw(name, sa.as_mut_ptr()) }
}

/// Creates the first pipe instance immediately (so squatting/ACL errors surface here) and
/// starts the accept loop.
pub fn start(cfg: ServerConfig) -> io::Result<ServerHandle> {
    let sid = winsec::current_user_sid()?;
    let mut sa = OwnedSecurityAttributes::from_sddl(&winsec::pipe_sddl(&sid))?;
    let first = create_instance(&cfg.pipe_name, &mut sa, true)?;

    let shared = Arc::new(Shared {
        pipe_name: cfg.pipe_name,
        token: cfg.token,
        log: cfg.log,
        started: Instant::now(),
        sessions: AtomicU64::new(1),
    });
    let shutdown = Arc::new(Notify::new());
    let stop = shutdown.clone();

    let task = tokio::spawn(async move {
        let mut server = first;
        loop {
            tokio::select! {
                r = server.connect() => r?,
                _ = stop.notified() => return Ok(()),
            }
            let client = server;
            server = create_instance(&shared.pipe_name, &mut sa, false)?;
            let s = shared.clone();
            tokio::spawn(async move {
                if let Err(e) = serve(client, s).await {
                    eprintln!("[xtraos-core] connection ended with error: {e}");
                }
            });
        }
    });
    Ok(ServerHandle { shutdown, task })
}

async fn serve(pipe: NamedPipeServer, shared: Arc<Shared>) -> io::Result<()> {
    let (rd, mut wr) = tokio::io::split(pipe);
    let mut reader = BufReader::new(rd);
    let mut conn = Conn::default();
    let next = || shared.sessions.fetch_add(1, Ordering::Relaxed);

    loop {
        let mut line = String::new();
        let read = (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_line(&mut line)
            .await;
        let t0 = Instant::now();
        let (outcome, info) = match read {
            Ok(0) => return Ok(()), // client closed
            Ok(_) if line.len() > MAX_LINE => too_big(),
            Ok(_) => {
                let ctx = Ctx {
                    token: &shared.token,
                    pipe: &shared.pipe_name,
                    started: shared.started,
                    next_session_id: &next,
                };
                protocol::handle_line(&mut conn, &line, &ctx)
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let reply = protocol::error(
                    &serde_json::Value::Null,
                    ErrorCode::InvalidArgs,
                    "request is not valid UTF-8",
                    "Send UTF-8 JSON.",
                );
                (
                    Outcome::ReplyAndClose(reply),
                    protocol::LogInfo {
                        method: "<invalid-utf8>".into(),
                        ok: false,
                        code: Some("INVALID_ARGS"),
                    },
                )
            }
            Err(e) => return Err(e),
        };

        shared.log.record(&Entry {
            ts_ms: protocol::now_ms(),
            session: conn.session.as_ref().map(|s| s.id.as_str()),
            client: conn.session.as_ref().map(|s| s.client.as_str()),
            method: &info.method,
            ok: info.ok,
            code: info.code,
            ms: t0.elapsed().as_secs_f64() * 1000.0,
        });

        let (reply, close) = match outcome {
            Outcome::Reply(v) => (v, false),
            Outcome::ReplyAndClose(v) => (v, true),
        };
        let mut bytes = serde_json::to_vec(&reply).map_err(io::Error::other)?;
        bytes.push(b'\n');
        wr.write_all(&bytes).await?;
        wr.flush().await?;
        if close {
            return Ok(());
        }
    }
}

fn too_big() -> (Outcome, protocol::LogInfo) {
    let reply = protocol::error(
        &serde_json::Value::Null,
        ErrorCode::InvalidArgs,
        &format!("request line exceeds {MAX_LINE} bytes"),
        "Split large payloads.",
    );
    (
        Outcome::ReplyAndClose(reply),
        protocol::LogInfo {
            method: "<too-large>".into(),
            ok: false,
            code: Some("INVALID_ARGS"),
        },
    )
}
