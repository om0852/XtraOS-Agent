//! Shared test helpers: an in-process core on a random pipe + a paired JSON-RPC client.
#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use xtraos_core::{actionlog::ActionLog, input, ipc, winsec};

pub fn rand_hex(n: usize) -> String {
    winsec::random_bytes(n)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn code(v: &Value) -> &str {
    v.pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub struct Core {
    server: Option<ipc::ServerHandle>,
    pub log: Arc<ActionLog>,
    rd: BufReader<tokio::io::ReadHalf<NamedPipeClient>>,
    wr: tokio::io::WriteHalf<NamedPipeClient>,
    id: u64,
    dir: PathBuf,
}

impl Core {
    pub async fn start() -> Self {
        input::init_dpi();
        let pipe = format!(r"\\.\pipe\xtraos-t-{}", rand_hex(6));
        let token = rand_hex(32);
        let dir = std::env::temp_dir().join(format!("xtraos-t-{}", rand_hex(4)));
        let log = Arc::new(ActionLog::create(&dir).unwrap());
        let server = ipc::start(ipc::ServerConfig {
            pipe_name: pipe.clone(),
            token: token.clone(),
            log: log.clone(),
        })
        .unwrap();
        let p = ClientOptions::new().open(&pipe).unwrap();
        let (rd, wr) = tokio::io::split(p);
        let mut core = Core {
            server: Some(server),
            log,
            rd: BufReader::new(rd),
            wr,
            id: 0,
            dir,
        };
        let r = core
            .call("pair", json!({"token": token, "client": "test"}))
            .await;
        assert!(r.get("result").is_some(), "{r}");
        core
    }

    pub async fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let msg = json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params});
        self.wr
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .unwrap();
        let mut s = String::new();
        tokio::time::timeout(Duration::from_secs(90), self.rd.read_line(&mut s))
            .await
            .expect("core reply timeout")
            .unwrap();
        serde_json::from_str(&s).unwrap()
    }

    /// Call that must succeed; returns `result`.
    pub async fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params.clone()).await;
        assert!(r.get("result").is_some(), "{method} {params} failed: {r}");
        r["result"].clone()
    }

    pub async fn stop(mut self) {
        self.server.take().unwrap().shutdown().await.unwrap();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

// ------------------------------------------------------------------------------- testbed

/// The testbed window with its stdin/stdout control channel (`--control`).
pub struct Testbed {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    out: std::io::BufReader<std::process::ChildStdout>,
    pub hwnd: u64,
    pub rects: std::collections::HashMap<String, [i32; 4]>,
}

pub fn testbed_exe() -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testbed/bin/testbed.exe");
    assert!(exe.exists(), "build the testbed first: testbed/build.ps1");
    exe
}

impl Testbed {
    pub fn start(x: i32, y: i32) -> Self {
        Self::start_with(x, y, &[])
    }

    /// Like `start`, with extra arguments (e.g. `--opaque`).
    pub fn start_with(x: i32, y: i32, extra: &[&str]) -> Self {
        use std::io::BufRead;
        let mut args = vec![x.to_string(), y.to_string(), "--control".to_string()];
        args.extend(extra.iter().map(|a| a.to_string()));
        let mut child = std::process::Command::new(testbed_exe())
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut tb = Testbed {
            child,
            stdin,
            out,
            hwnd: 0,
            rects: Default::default(),
        };
        let mut s = String::new();
        tb.out.read_line(&mut s).unwrap();
        assert!(s.contains("ready"), "testbed did not start: {s}");
        std::thread::sleep(Duration::from_millis(400));
        let r = tb.cmd("rects");
        tb.hwnd = r["hwnd"].as_u64().unwrap();
        for (k, v) in r["controls"].as_object().unwrap() {
            let a: Vec<i32> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_i64().unwrap() as i32)
                .collect();
            tb.rects.insert(k.clone(), [a[0], a[1], a[2], a[3]]);
        }
        tb
    }

    pub fn window(&self) -> String {
        format!("w{}", self.hwnd)
    }

    pub fn cmd(&mut self, c: &str) -> Value {
        use std::io::{BufRead, Write};
        writeln!(self.stdin, "{c}").unwrap();
        self.stdin.flush().unwrap();
        let mut s = String::new();
        self.out.read_line(&mut s).unwrap();
        serde_json::from_str(&s).unwrap_or_else(|e| panic!("bad testbed line {s:?}: {e}"))
    }

    pub fn state(&mut self) -> Value {
        std::thread::sleep(Duration::from_millis(150));
        self.cmd("state")
    }

    pub fn center(&self, name: &str) -> (i32, i32) {
        let [x, y, w, h] = self.rects[name];
        (x + w / 2, y + h / 2)
    }
}

impl Drop for Testbed {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        for _ in 0..30 {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
    }
}

/// `e12 Button "btnA"` lines of an observe/find text → name → ref (first wins).
pub fn refs_by_name(text: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    for l in text.lines() {
        let l = l.trim_start_matches(['+', '~', ' ']);
        let mut it = l.splitn(3, ' ');
        let (Some(r), Some(_role), Some(rest)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if !r.starts_with('e') {
            continue;
        }
        if let Some(name) = rest.strip_prefix('"').and_then(|s| s.split('"').next()) {
            m.entry(name.to_string()).or_insert_with(|| r.to_string());
        }
    }
    m
}

/// Rough token estimate used by the budgets (docs/06: chars / 4).
pub fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}
