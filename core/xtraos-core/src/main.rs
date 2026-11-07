//! `xtraos-core` binary: starts the pipe server and runs until Ctrl+C (or the session ends).
//! Environment overrides (tests): XTRAOS_PIPE, XTRAOS_TOKEN, XTRAOS_HOME.
//! Release builds are windowless (no console window at login or when MCP starts the core,
//! ADR 0019); output still reaches redirected stdout/stderr (tests read the READY line).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::Arc;

use xtraos_core::{actionlog::ActionLog, ipc, pairing, protocol};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("[xtraos-core] fatal: {e}");
        std::process::exit(1);
    }
}

async fn run() -> std::io::Result<()> {
    let dir = pairing::data_dir();
    let pipe = pairing::pipe_name()?;
    let token = pairing::load_or_create_token(&dir)?;
    let log = Arc::new(ActionLog::create(&dir.join("logs"))?);

    let server = ipc::start(ipc::ServerConfig {
        pipe_name: pipe.clone(),
        token,
        log: log.clone(),
    })
    .map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("could not create pipe {pipe} (already running, or name taken?): {e}"),
        )
    })?;
    pairing::write_connection_info(&dir, &pipe)?;

    eprintln!("[xtraos-core] {} listening on {pipe}", protocol::VERSION);
    eprintln!("[xtraos-core] action log: {}", log.path().display());
    println!("READY {pipe}"); // machine-readable readiness line for tests/launchers

    tokio::signal::ctrl_c().await?;
    eprintln!("[xtraos-core] stopping");
    server.shutdown().await
}
