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
