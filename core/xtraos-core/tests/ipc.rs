//! Integration tests over a REAL named pipe (docs/08-test-plan.md, Phase 1 M1).
//! Each test uses its own random pipe name + token + temp log dir, so tests never touch the
//! user's real core instance.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, ServerOptions};
use xtraos_core::{actionlog::ActionLog, ipc, winsec};

const ERROR_PIPE_BUSY: i32 = 231;

fn rand_hex(n: usize) -> String {
    winsec::random_bytes(n)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Fixture {
    pipe: String,
    token: String,
    log: Arc<ActionLog>,
    dir: PathBuf,
    server: Option<ipc::ServerHandle>,
}

impl Fixture {
    fn start() -> Self {
        let pipe = format!(r"\\.\pipe\xtraos-test-{}", rand_hex(6));
        let token = rand_hex(32);
        let dir = std::env::temp_dir().join(format!("xtraos-it-{}", rand_hex(4)));
        let log = Arc::new(ActionLog::create(&dir).unwrap());
        let server = ipc::start(ipc::ServerConfig {
            pipe_name: pipe.clone(),
            token: token.clone(),
            log: log.clone(),
        })
        .unwrap();
        Fixture {
            pipe,
            token,
            log,
            dir,
            server: Some(server),
        }
    }

    async fn connect(&self) -> Client {
        for _ in 0..50 {
            match ClientOptions::new().open(&self.pipe) {
                Ok(c) => return Client::new(c),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => panic!("open failed: {e}"),
            }
        }
        panic!("pipe stayed busy");
    }

    async fn paired(&self) -> Client {
        let mut c = self.connect().await;
        let r = c.call(json!({"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":self.token,"client":"it"}})).await.unwrap();
        assert!(r.get("result").is_some(), "pair failed: {r}");
        c
    }

    async fn stop(mut self) {
        self.server.take().unwrap().shutdown().await.unwrap();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

struct Client {
    rd: BufReader<tokio::io::ReadHalf<NamedPipeClient>>,
    wr: tokio::io::WriteHalf<NamedPipeClient>,
}

impl Client {
    fn new(p: NamedPipeClient) -> Self {
        let (rd, wr) = tokio::io::split(p);
        Client {
            rd: BufReader::new(rd),
            wr,
        }
    }

    async fn send_raw(&mut self, line: &[u8]) {
        self.wr.write_all(line).await.unwrap();
        self.wr.write_all(b"\n").await.unwrap();
    }

    /// Next reply, or None if the server closed the connection.
    async fn recv(&mut self) -> Option<Value> {
        let mut s = String::new();
        let n = tokio::time::timeout(Duration::from_secs(5), self.rd.read_line(&mut s))
            .await
            .expect("reply timeout")
            .ok()?;
        if n == 0 {
            None
        } else {
            Some(serde_json::from_str(&s).unwrap())
        }
    }

    async fn call(&mut self, v: Value) -> Option<Value> {
        self.send_raw(&serde_json::to_vec(&v).unwrap()).await;
        self.recv().await
    }

    async fn is_closed(&mut self) -> bool {
        let mut s = String::new();
        matches!(
            tokio::time::timeout(Duration::from_secs(2), self.rd.read_line(&mut s)).await,
            Ok(Ok(0)) | Ok(Err(_))
        )
    }
}

fn code(v: &Value) -> &str {
    v.pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
}

// P1-IPC-01
#[tokio::test(flavor = "multi_thread")]
async fn ipc01_pair_then_status_and_ping() {
    let f = Fixture::start();
    let mut c = f.paired().await;
    let st = c
        .call(json!({"jsonrpc":"2.0","id":2,"method":"status"}))
        .await
        .unwrap();
    assert_eq!(st.pointer("/result/state").unwrap(), "running");
    assert_eq!(st.pointer("/result/client").unwrap(), "it");
    assert_eq!(st.pointer("/result/pipe").unwrap(), &json!(f.pipe));
    let p = c
        .call(json!({"jsonrpc":"2.0","id":3,"method":"ping"}))
        .await
        .unwrap();
    assert_eq!(p["result"], "pong");
    assert_eq!(p["id"], 3);
    f.stop().await;
}

// P1-IPC-02
#[tokio::test(flavor = "multi_thread")]
async fn ipc02_unpaired_wrong_or_missing_token_is_refused_and_closed() {
    let f = Fixture::start();
    let cases = [
        json!({"jsonrpc":"2.0","id":1,"method":"status"}),
        json!({"jsonrpc":"2.0","id":1,"method":"pair","params":{"token":"0".repeat(64)}}),
        json!({"jsonrpc":"2.0","id":1,"method":"pair","params":{}}),
    ];
    for req in cases {
        let mut c = f.connect().await;
        let r = c.call(req.clone()).await.unwrap();
        assert_eq!(code(&r), "NOT_PAIRED", "{req}");
        assert!(
            c.is_closed().await,
            "connection should be closed after {req}"
        );
    }
    f.stop().await;
}

// P1-IPC-03
#[tokio::test(flavor = "multi_thread")]
async fn ipc03_pipe_dacl_is_current_user_only_and_denies_network() {
    let f = Fixture::start();
    let sid = winsec::current_user_sid().unwrap();
    let sddl = winsec::dacl_sddl_of(&f.pipe).unwrap();
    assert!(sddl.starts_with("D:P"), "DACL must be protected: {sddl}");
    assert_eq!(
        sddl.matches("(A;").count(),
        1,
        "exactly one allow ACE: {sddl}"
    );
    assert!(
        sddl.contains(&format!(";;;{sid})")),
        "allow ACE must be the current user: {sddl}"
    );
    assert!(
        sddl.contains("(D;") && sddl.contains(";;;NU)"),
        "NETWORK must be denied: {sddl}"
    );
    f.stop().await;
}

// P1-IPC-03 (squatting): a second "first instance" on the same name must fail.
#[tokio::test(flavor = "multi_thread")]
async fn ipc03b_pipe_name_cannot_be_squatted() {
    let f = Fixture::start();
    let r = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&f.pipe);
    assert!(
        r.is_err(),
        "creating a first instance on an existing name must fail"
    );
    f.stop().await;
}

// P1-IPC-04
#[tokio::test(flavor = "multi_thread")]
async fn ipc04_malformed_and_unknown_requests_keep_core_running() {
    let f = Fixture::start();
    let mut c = f.paired().await;
    c.send_raw(b"this is not json").await;
    assert_eq!(code(&c.recv().await.unwrap()), "INVALID_ARGS");
    let r = c
        .call(json!({"jsonrpc":"2.0","id":5,"method":"definitely_not_a_method"}))
        .await
        .unwrap();
    assert_eq!(code(&r), "INVALID_ARGS");
    let r = c
        .call(json!({"jsonrpc":"2.0","id":6,"method":"agent_workspace","params":{}}))
        .await
        .unwrap();
    assert_eq!(
        code(&r),
        "NOT_IMPLEMENTED",
        "planned tools must never fake success"
    );
    // same connection still works
    let r = c
        .call(json!({"jsonrpc":"2.0","id":7,"method":"ping"}))
        .await
        .unwrap();
    assert_eq!(r["result"], "pong");
    // and new connections still work
    let mut c2 = f.paired().await;
    assert_eq!(
        c2.call(json!({"jsonrpc":"2.0","id":8,"method":"ping"}))
            .await
            .unwrap()["result"],
        "pong"
    );
    f.stop().await;
}

// P1-IPC-04 (oversized line): rejected + closed, core keeps serving.
#[tokio::test(flavor = "multi_thread")]
async fn ipc04b_oversized_request_is_rejected() {
    let f = Fixture::start();
    let mut c = f.paired().await;
    let big = vec![b'a'; ipc::MAX_LINE + 10];
    c.send_raw(&big).await;
    let r = c.recv().await.unwrap();
    assert_eq!(code(&r), "INVALID_ARGS");
    assert!(c.is_closed().await);
    let mut c2 = f.paired().await;
    assert_eq!(
        c2.call(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .await
            .unwrap()["result"],
        "pong"
    );
    f.stop().await;
}

// Concurrency: separate sessions per connection.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_clients_get_distinct_sessions() {
    let f = Fixture::start();
    let mut a = f.paired().await;
    let mut b = f.paired().await;
    let sa = a
        .call(json!({"jsonrpc":"2.0","id":1,"method":"status"}))
        .await
        .unwrap();
    let sb = b
        .call(json!({"jsonrpc":"2.0","id":1,"method":"status"}))
        .await
        .unwrap();
    assert_ne!(sa.pointer("/result/session"), sb.pointer("/result/session"));
    f.stop().await;
}

// P1-LOG-01: one entry per request, schema valid, no token / params leaked.
#[tokio::test(flavor = "multi_thread")]
async fn log01_every_request_is_logged_without_secrets() {
    let f = Fixture::start();
    let mut c = f.paired().await;
    c.call(json!({"jsonrpc":"2.0","id":2,"method":"status"}))
        .await
        .unwrap();
    c.call(json!({"jsonrpc":"2.0","id":3,"method":"agent_workspace","params":{"secret_param":"do-not-log"}}))
        .await
        .unwrap();
    let mut bad = f.connect().await;
    bad.call(json!({"jsonrpc":"2.0","id":1,"method":"status"}))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let text = std::fs::read_to_string(f.log.path()).unwrap();
    assert!(!text.contains(&f.token), "token must never be logged");
    assert!(!text.contains("do-not-log"), "params must never be logged");
    let entries: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        entries.len(),
        4,
        "pair, status, click, refused status: {text}"
    );
    for e in &entries {
        for k in ["ts_ms", "method", "ok", "ms"] {
            assert!(e.get(k).is_some(), "missing {k} in {e}");
        }
    }
    let methods: Vec<&str> = entries
        .iter()
        .map(|e| e["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, ["pair", "status", "agent_workspace", "status"]);
    assert_eq!(entries[2]["code"], "NOT_IMPLEMENTED");
    assert_eq!(entries[3]["code"], "NOT_PAIRED");
    f.stop().await;
}

// Shutdown releases the pipe name so a new core can start (restart path, P1-IPC-05).
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_releases_the_pipe_for_a_restart() {
    let f = Fixture::start();
    let pipe = f.pipe.clone();
    let token = f.token.clone();
    let log = f.log.clone();
    f.stop().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let again = ipc::start(ipc::ServerConfig {
        pipe_name: pipe,
        token,
        log,
    })
    .expect("restart on same name");
    again.shutdown().await.unwrap();
}
